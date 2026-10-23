use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use glam::{Mat3, Mat4, Vec3};
use gltf::mesh::util::ReadIndices;

use crate::assets::types::effect_material::{EffectMaterial, RefractionMaterial};
use crate::assets::types::model_extras::{
    ExtrasError, ModelExtras, effect_material, has_hidden_mesh, hidden_nodes, refraction_material,
};
use crate::assets::types::skinned_mesh::RestPose;
use crate::rendering::vertex::Vertex;

/// w = 0 tells the fragment shader to derive the tangent frame from screen-space
/// derivatives instead.
const MISSING_TANGENT: [f32; 4] = [0.0, 0.0, 0.0, 0.0];

/// Transpose of the inverse scaled by |det|: transforms normals correctly under
/// non-uniform scale and stays defined for singular matrices.
pub(super) fn cofactor_normal_matrix(m: Mat3) -> Mat3 {
    Mat3::from_cols(
        m.y_axis.cross(m.z_axis),
        m.z_axis.cross(m.x_axis),
        m.x_axis.cross(m.y_axis),
    ) * handedness(m)
}

/// -1 for mirroring transforms, whose bitangent sign must flip.
pub(super) fn handedness(m: Mat3) -> f32 {
    if m.determinant() < 0.0 { -1.0 } else { 1.0 }
}

/// World matrix of every node by index, from the node hierarchy alone.
pub(crate) fn node_world_matrices(document: &gltf::Document) -> Vec<Mat4> {
    fn fill(node: &gltf::Node, parent: Mat4, worlds: &mut [Mat4]) {
        let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        worlds[node.index()] = world;
        for child in node.children() {
            fill(&child, world, worlds);
        }
    }

    let mut worlds = vec![Mat4::IDENTITY; document.nodes().len()];
    let children: std::collections::HashSet<usize> = document
        .nodes()
        .flat_map(|node| node.children().map(|child| child.index()))
        .collect();
    for root in document
        .nodes()
        .filter(|node| !children.contains(&node.index()))
    {
        fill(&root, Mat4::IDENTITY, &mut worlds);
    }
    worlds
}

/// Collects a document's meshes; node world matrices are only computed once a
/// skinned node needs them.
struct Collector<'a> {
    document: &'a gltf::Document,
    buffers: &'a [gltf::buffer::Data],
    source: &'a str,
    node_worlds: OnceCell<Vec<Mat4>>,
    meshes: Vec<CpuMesh>,
    /// Nodes holding mesh-emitter source geometry: collected by name instead
    /// of into `meshes`, since they are never drawn.
    emitter_nodes: &'a HashSet<String>,
    emitter_meshes: HashMap<String, Vec<CpuMesh>>,
    /// Culled nodes (see `model_extras::hidden_nodes`): their meshes are not
    /// drawable, but emitter source geometry is still collected.
    hidden_nodes: &'a [bool],
}

impl Collector<'_> {
    fn collect_from_node(&mut self, node: &gltf::Node, parent_transform: Mat4) {
        let world = parent_transform * Mat4::from_cols_array_2d(&node.transform().matrix());
        self.collect_node_primitives(node, world);
        for child in node.children() {
            self.collect_from_node(&child, world);
        }
    }

    fn collect_node_primitives(&mut self, node: &gltf::Node, transform: Mat4) {
        let Some(mesh) = node.mesh() else { return };
        if let Some(name) = node
            .name()
            .filter(|name| self.emitter_nodes.contains(*name))
        {
            let entry = self.emitter_meshes.entry(name.to_owned()).or_default();
            entry.extend(mesh.primitives().filter_map(|primitive| {
                CpuMesh::from_primitive(&primitive, self.buffers, transform)
            }));
            return;
        }
        if self.hidden_nodes[node.index()] {
            return;
        }
        match node.skin() {
            Some(skin) => self.collect_skinned_primitives(node, &mesh, &skin, transform),
            None => CpuMesh::collect_primitives(&mesh, self.buffers, transform, &mut self.meshes),
        }
    }

    /// Skins each primitive once with the file's rest node pose. A skin or
    /// primitive whose data is unusable is reported and loaded unskinned
    /// instead, so one bad primitive never drops the rest of the file.
    fn collect_skinned_primitives(
        &mut self,
        node: &gltf::Node,
        mesh: &gltf::Mesh,
        skin: &gltf::Skin,
        transform: Mat4,
    ) {
        let node_name = node.name().unwrap_or("");
        let node_worlds = self
            .node_worlds
            .get_or_init(|| node_world_matrices(self.document));
        let rest_pose = match RestPose::new(skin, self.buffers, node_worlds) {
            Ok(rest_pose) => Some(rest_pose),
            Err(error) => {
                self.warn_unskinned(node_name, None, &error);
                None
            }
        };
        for primitive in mesh.primitives() {
            let skinned = rest_pose
                .as_ref()
                .map(|pose| pose.skin_primitive(&primitive, node_name, self.buffers));
            match skinned {
                Some(Ok(Some(cpu))) => self.meshes.push(cpu),
                Some(Ok(None)) => {}
                Some(Err(error)) => {
                    self.warn_unskinned(node_name, Some(primitive.index()), &error);
                    self.meshes.extend(CpuMesh::from_primitive(
                        &primitive,
                        self.buffers,
                        transform,
                    ));
                }
                None => {
                    self.meshes.extend(CpuMesh::from_primitive(
                        &primitive,
                        self.buffers,
                        transform,
                    ));
                }
            }
        }
    }

    fn warn_unskinned(
        &self,
        node_name: &str,
        primitive: Option<usize>,
        error: &dyn std::fmt::Display,
    ) {
        let source = self.source;
        let scope =
            primitive.map_or_else(|| "skin".to_owned(), |index| format!("primitive {index}"));
        crate::utils::warn_once(
            &format!("skin-fallback:{source}:{node_name}:{scope}"),
            || format!("{source}: mesh node {node_name:?} {scope}: {error}; loading it unskinned"),
        );
    }
}

/// How a primitive's indices read: triangle lists are drawable, point sets only feed
/// mesh-emitter vertex sampling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Topology {
    #[default]
    Triangles,
    Points,
}

impl Topology {
    /// `None` for modes the engine has no use for (lines, strips and fans).
    fn from_mode(mode: gltf::mesh::Mode) -> Option<Self> {
        match mode {
            gltf::mesh::Mode::Triangles => Some(Self::Triangles),
            gltf::mesh::Mode::Points => Some(Self::Points),
            _ => None,
        }
    }
}

pub struct CpuMesh {
    pub vertices: Vec<Vertex>,
    pub topology: Topology,
    pub indices: Vec<u32>,
    pub texture_path: Option<String>,
    /// `COLOR_0` as RGBA8, one per vertex; `None` when the primitive has no colour
    /// attribute or its count does not match the vertices.
    pub vertex_colors: Option<Vec<[u8; 4]>>,
    /// `material.extras.effect` of the primitive's material: the shape is drawn by the effect stage.
    pub effect: Option<Arc<EffectMaterial>>,
    /// `material.extras.refraction`: the shape is drawn by the refraction stage.
    pub refraction: Option<Arc<RefractionMaterial>>,
}

/// An unusable extras object is reported once per material; the shape then has no extras.
fn usable_extras<T>(
    parsed: Result<Option<T>, ExtrasError>,
    kind: &str,
    material_name: &str,
) -> Option<Arc<T>> {
    match parsed {
        Ok(extras) => extras.map(Arc::new),
        Err(error) => {
            crate::utils::warn_once(&format!("material-extras:{kind}:{material_name}"), || {
                format!(
                    "material {material_name:?}: {kind} extras unusable ({error}); ignoring them"
                )
            });
            None
        }
    }
}

/// A model file: drawable meshes, mesh-emitter source geometry (by glTF node
/// name, node transforms baked) and the node-level extras.
pub struct LoadedModel {
    pub meshes: Vec<CpuMesh>,
    pub emitter_meshes: HashMap<String, Vec<CpuMesh>>,
    pub extras: ModelExtras,
}

impl CpuMesh {
    pub fn load(path: &str) -> anyhow::Result<Vec<Self>> {
        // Gltf::open + import_buffers instead of gltf::import: import() eagerly
        // decodes every embedded image, which we never use here.
        let gltf = gltf::Gltf::open(path)?;
        let base = std::path::Path::new(path).parent();
        let buffers = gltf::import_buffers(&gltf.document, base, gltf.blob)?;
        Self::from_document(&gltf.document, &buffers, path)
            .map_err(|error| anyhow::anyhow!("{path}: {error}"))
    }

    /// Every primitive with its node transform baked into the vertices. Skinned
    /// nodes are instead skinned once with the file's rest node pose. `source`
    /// names the file in skin fallback warnings.
    pub fn from_document(
        document: &gltf::Document,
        buffers: &[gltf::buffer::Data],
        source: &str,
    ) -> anyhow::Result<Vec<Self>> {
        let hidden = hidden_nodes(document);
        let (meshes, _) = Self::collect_all(document, buffers, source, &HashSet::new(), &hidden);
        if meshes.is_empty() && !has_hidden_mesh(document, &hidden) {
            anyhow::bail!("file has no meshes");
        }
        Ok(meshes)
    }

    /// Meshes plus the node-level extras. A file with particle systems or other
    /// extras but no geometry is valid.
    pub fn load_model(path: &str) -> anyhow::Result<LoadedModel> {
        let gltf = gltf::Gltf::open(path)?;
        let base = std::path::Path::new(path).parent();
        let buffers = gltf::import_buffers(&gltf.document, base, gltf.blob)?;
        Self::model_from_document(&gltf.document, &buffers, path)
            .map_err(|error| anyhow::anyhow!("{path}: {error}"))
    }

    pub fn model_from_document(
        document: &gltf::Document,
        buffers: &[gltf::buffer::Data],
        source: &str,
    ) -> anyhow::Result<LoadedModel> {
        let extras = ModelExtras::from_document(document);
        for rejected in &extras.rejected {
            crate::utils::warn_once(
                &format!("model-extras:{source}:{}", rejected.node_name),
                || {
                    format!(
                        "{source}: node {:?}: {}; ignoring it",
                        rejected.node_name, rejected.error
                    )
                },
            );
        }
        let (meshes, emitter_meshes) = Self::collect_all(
            document,
            buffers,
            source,
            &extras.emitter_mesh_nodes(),
            &extras.hidden_nodes,
        );
        if meshes.is_empty()
            && emitter_meshes.is_empty()
            && extras.is_empty()
            && !has_hidden_mesh(document, &extras.hidden_nodes)
        {
            anyhow::bail!("file has no meshes");
        }
        Ok(LoadedModel {
            meshes,
            emitter_meshes,
            extras,
        })
    }

    fn collect_all(
        document: &gltf::Document,
        buffers: &[gltf::buffer::Data],
        source: &str,
        emitter_nodes: &HashSet<String>,
        hidden_nodes: &[bool],
    ) -> (Vec<Self>, HashMap<String, Vec<Self>>) {
        let mut collector = Collector {
            document,
            buffers,
            source,
            node_worlds: OnceCell::new(),
            meshes: Vec::new(),
            emitter_nodes,
            emitter_meshes: HashMap::new(),
            hidden_nodes,
        };

        for scene in document.scenes() {
            for node in scene.nodes() {
                collector.collect_from_node(&node, Mat4::IDENTITY);
            }
        }

        if collector.meshes.is_empty() && collector.emitter_meshes.is_empty() {
            for node in document.nodes() {
                let local = Mat4::from_cols_array_2d(&node.transform().matrix());
                collector.collect_node_primitives(&node, local);
            }
        }

        (collector.meshes, collector.emitter_meshes)
    }

    /// Appends the mesh's drawable primitives with `transform` applied to their vertices.
    /// Point sets are not drawable: they are skipped with a warning.
    pub(crate) fn collect_primitives(
        mesh: &gltf::Mesh,
        buffers: &[gltf::buffer::Data],
        transform: Mat4,
        out: &mut Vec<Self>,
    ) {
        out.extend(
            mesh.primitives()
                .filter_map(|primitive| Self::from_primitive(&primitive, buffers, transform))
                .filter(|cpu| {
                    let drawable = cpu.topology == Topology::Triangles;
                    if !drawable {
                        crate::utils::warn_once(
                            &format!("points-primitive:{}", cpu.texture_path.as_deref().unwrap_or("")),
                            || {
                                format!(
                                    "a point-set primitive (material {:?}) outside a mesh emitter cannot be drawn; skipping it",
                                    cpu.texture_path.as_deref().unwrap_or("")
                                )
                            },
                        );
                    }
                    drawable
                }),
        );
    }

    /// `None` for a primitive without positions or with a topology the engine does not use.
    /// A point set without normals gets zero normals, which means "no direction".
    pub(crate) fn from_primitive(
        primitive: &gltf::Primitive,
        buffers: &[gltf::buffer::Data],
        transform: Mat4,
    ) -> Option<Self> {
        let Some(topology) = Topology::from_mode(primitive.mode()) else {
            crate::utils::warn_once(&format!("primitive-mode:{:?}", primitive.mode()), || {
                format!(
                    "primitive mode {:?} is not supported; skipping the primitive",
                    primitive.mode()
                )
            });
            return None;
        };
        let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

        let positions: Vec<[f32; 3]> = reader.read_positions()?.collect();

        let uvs: Vec<[f32; 2]> = reader.read_tex_coords(0).map_or_else(
            || vec![[0.0, 0.0]; positions.len()],
            |tc| tc.into_f32().collect(),
        );

        let missing_normal = match topology {
            Topology::Triangles => [0.0, 1.0, 0.0],
            Topology::Points => [0.0; 3],
        };
        let normals: Vec<[f32; 3]> = reader.read_normals().map_or_else(
            || vec![missing_normal; positions.len()],
            std::iter::Iterator::collect,
        );

        let tangents: Vec<[f32; 4]> = reader.read_tangents().map_or_else(
            || vec![MISSING_TANGENT; positions.len()],
            std::iter::Iterator::collect,
        );

        let mat3 = Mat3::from_mat4(transform);
        let normal_matrix = cofactor_normal_matrix(mat3);
        let handedness = handedness(mat3);

        let vertices: Vec<Vertex> = positions
            .iter()
            .zip(uvs.iter())
            .zip(normals.iter())
            .zip(tangents.iter())
            .map(|(((&pos, &uv), &nrm), &tan)| {
                let p = transform.transform_point3(Vec3::from(pos));
                let n = (normal_matrix * Vec3::from(nrm)).normalize_or_zero();
                let t = (mat3 * Vec3::from([tan[0], tan[1], tan[2]])).normalize_or_zero();
                Vertex {
                    position: p.to_array(),
                    tex_coords: uv,
                    normal: n.to_array(),
                    tangent: [t.x, t.y, t.z, tan[3] * handedness],
                }
            })
            .collect();

        let indices: Vec<u32> = match reader.read_indices() {
            Some(ReadIndices::U8(it)) => it.map(u32::from).collect(),
            Some(ReadIndices::U16(it)) => it.map(u32::from).collect(),
            Some(ReadIndices::U32(it)) => it.collect(),
            None => (0..positions.len() as u32).collect(),
        };

        let material = primitive.material();
        let texture_path = material.name().unwrap_or("").to_string();
        let effect = usable_extras(effect_material(&material), "effect", &texture_path);
        let refraction = usable_extras(refraction_material(&material), "refraction", &texture_path);
        let vertex_colors = reader
            .read_colors(0)
            .map(|colors| colors.into_rgba_u8().collect::<Vec<_>>())
            .filter(|colors| colors.len() == vertices.len());

        Some(Self {
            vertices,
            topology,
            indices,
            texture_path: Some(texture_path),
            vertex_colors,
            effect,
            refraction,
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals parsed or copied, not computed"
)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    /// Binary glTF 2.0 container: 12-byte header, a JSON chunk, a BIN chunk,
    /// each padded to 4 bytes (glTF 2.0 spec, section "Binary glTF Layout").
    fn glb(json: &Value, bin: &[u8]) -> Vec<u8> {
        fn chunk(kind: [u8; 4], data: &[u8], pad: u8) -> Vec<u8> {
            let mut data = data.to_vec();
            while !data.len().is_multiple_of(4) {
                data.push(pad);
            }
            let mut out = (data.len() as u32).to_le_bytes().to_vec();
            out.extend(kind);
            out.extend(data);
            out
        }
        let mut body = chunk(*b"JSON", json.to_string().as_bytes(), b' ');
        if !bin.is_empty() {
            body.extend(chunk(*b"BIN\0", bin, 0));
        }
        let mut out = b"glTF".to_vec();
        out.extend(2u32.to_le_bytes());
        out.extend(((12 + body.len()) as u32).to_le_bytes());
        out.extend(body);
        out
    }

    fn load(json: &Value, bin: &[u8]) -> anyhow::Result<Vec<CpuMesh>> {
        let gltf = gltf::Gltf::from_slice(&glb(json, bin)).unwrap();
        let buffers = gltf::import_buffers(&gltf.document, None, gltf.blob).unwrap();
        CpuMesh::from_document(&gltf.document, &buffers, "test.glb")
    }

    fn positions_bin() -> Vec<u8> {
        [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
            .iter()
            .flatten()
            .flat_map(|f| f.to_le_bytes())
            .collect()
    }

    /// One triangle (positions only) on `nodes`; node 0 is the scene root unless
    /// `scene_nodes` says otherwise.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "call sites pass json! literals"
    )]
    fn triangle_doc(nodes: Value, scene_nodes: Value, primitive_extra: &Value) -> Value {
        let mut primitive = json!({"attributes": {"POSITION": 0}});
        primitive
            .as_object_mut()
            .unwrap()
            .extend(primitive_extra.as_object().unwrap().clone());
        json!({
            "asset": {"version": "2.0"},
            "scene": 0,
            "scenes": [{"nodes": scene_nodes}],
            "nodes": nodes,
            "meshes": [{"primitives": [primitive]}],
            "materials": [{"name": "a_d.dds|a_n.dds"}],
            "buffers": [{"byteLength": 36}],
            "bufferViews": [{"buffer": 0, "byteLength": 36}],
            "accessors": [{
                "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                "min": [0, 0, 0], "max": [1, 1, 0]
            }]
        })
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn node_translation_is_baked_into_vertex_positions() {
        let doc = triangle_doc(
            json!([{"mesh": 0, "translation": [1, 2, 3]}]),
            json!([0]),
            &json!({}),
        );
        let meshes = load(&doc, &positions_bin()).unwrap();
        assert_eq!(meshes.len(), 1);
        assert!(close(meshes[0].vertices[0].position, [1.0, 2.0, 3.0]));
        assert!(close(meshes[0].vertices[1].position, [2.0, 2.0, 3.0]));
    }

    #[test]
    fn child_node_transform_composes_with_its_parent() {
        let nodes = json!([
            {"children": [1], "translation": [10, 0, 0]},
            {"mesh": 0, "translation": [0, 5, 0]}
        ]);
        let meshes = load(
            &triangle_doc(nodes, json!([0]), &json!({})),
            &positions_bin(),
        )
        .unwrap();
        assert!(close(meshes[0].vertices[0].position, [10.0, 5.0, 0.0]));
    }

    #[test]
    fn missing_normals_default_to_up_and_missing_tangents_to_screen_space_marker() {
        let doc = triangle_doc(json!([{"mesh": 0}]), json!([0]), &json!({}));
        let mesh = &load(&doc, &positions_bin()).unwrap()[0];
        assert_eq!(mesh.vertices[0].normal, [0.0, 1.0, 0.0]);
        assert_eq!(mesh.vertices[0].tangent[3], 0.0);
        assert_eq!(mesh.vertices[0].tex_coords, [0.0, 0.0]);
    }

    #[test]
    fn authored_normals_uvs_and_indices_are_read_and_the_node_rotation_turns_the_normals() {
        let floats =
            |values: &[f32]| -> Vec<u8> { values.iter().flat_map(|f| f.to_le_bytes()).collect() };
        let mut bin = positions_bin();
        bin.extend(floats(&[1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0])); // normals, 36..72
        bin.extend(floats(&[0.0, 0.0, 1.0, 0.0, 0.5, 1.0])); // uvs, 72..96
        bin.extend([2u8, 0, 0, 0, 1, 0, 0, 0]); // u16 indices 2, 0, 1 (+ padding), 96..104
        let mut doc = triangle_doc(
            // 90 degrees about +Z: (1, 0, 0) -> (0, 1, 0).
            json!([{"mesh": 0, "rotation": [0, 0, 0.707_106_77, 0.707_106_77]}]),
            json!([0]),
            &json!({"attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}, "indices": 3}),
        );
        doc["buffers"][0]["byteLength"] = json!(bin.len());
        doc["bufferViews"] = json!([
            {"buffer": 0, "byteOffset": 0, "byteLength": 36},
            {"buffer": 0, "byteOffset": 36, "byteLength": 36},
            {"buffer": 0, "byteOffset": 72, "byteLength": 24},
            {"buffer": 0, "byteOffset": 96, "byteLength": 6}
        ]);
        doc["accessors"] = json!([
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0]},
            {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"},
            {"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2"},
            {"bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR"}
        ]);
        let mesh = &load(&doc, &bin).unwrap()[0];
        assert_eq!(mesh.indices, vec![2, 0, 1]);
        assert_eq!(mesh.vertices[1].tex_coords, [1.0, 0.0]);
        assert_eq!(mesh.vertices[2].tex_coords, [0.5, 1.0]);
        assert!(close(mesh.vertices[0].normal, [0.0, 1.0, 0.0]));
        assert!(close(mesh.vertices[1].position, [0.0, 1.0, 0.0]));
    }

    #[test]
    fn missing_indices_default_to_sequential() {
        let doc = triangle_doc(json!([{"mesh": 0}]), json!([0]), &json!({}));
        assert_eq!(
            load(&doc, &positions_bin()).unwrap()[0].indices,
            vec![0, 1, 2]
        );
    }

    #[test]
    fn material_name_is_carried_as_texture_path() {
        let doc = triangle_doc(json!([{"mesh": 0}]), json!([0]), &json!({"material": 0}));
        let mesh = &load(&doc, &positions_bin()).unwrap()[0];
        assert_eq!(mesh.texture_path.as_deref(), Some("a_d.dds|a_n.dds"));
    }

    #[test]
    fn primitive_without_material_has_empty_texture_path() {
        let doc = triangle_doc(json!([{"mesh": 0}]), json!([0]), &json!({}));
        let mesh = &load(&doc, &positions_bin()).unwrap()[0];
        assert_eq!(mesh.texture_path.as_deref(), Some(""));
    }

    #[test]
    fn node_outside_every_scene_is_still_loaded_with_its_local_transform() {
        let doc = triangle_doc(
            json!([{"mesh": 0, "translation": [4, 0, 0]}]),
            json!([]),
            &json!({}),
        );
        let meshes = load(&doc, &positions_bin()).unwrap();
        assert!(close(meshes[0].vertices[0].position, [4.0, 0.0, 0.0]));
    }

    fn load_model(json: &Value, bin: &[u8]) -> anyhow::Result<LoadedModel> {
        let gltf = gltf::Gltf::from_slice(&glb(json, bin)).unwrap();
        let buffers = gltf::import_buffers(&gltf.document, None, gltf.blob).unwrap();
        CpuMesh::model_from_document(&gltf.document, &buffers, "test.glb")
    }

    fn particle_extras() -> Value {
        json!({"master_particle_system":
            {"v": 1, "max_emitter_objects": 20, "systems": ["Sparks"]}})
    }

    #[test]
    fn particle_only_document_loads_with_its_extras_and_no_meshes() {
        let doc = json!({
            "asset": {"version": "2.0"},
            "scenes": [{"nodes": [0]}],
            "nodes": [{"name": "MPS", "extras": particle_extras()}]
        });
        let model = load_model(&doc, &[]).unwrap();
        assert!(model.meshes.is_empty());
        assert_eq!(model.extras.master_particle_systems.len(), 1);
        assert_eq!(model.extras.master_particle_systems[0].node_name, "MPS");
    }

    #[test]
    fn document_with_neither_meshes_nor_extras_is_still_an_error() {
        let doc = json!({
            "asset": {"version": "2.0"},
            "scenes": [{"nodes": [0]}],
            "nodes": [{}]
        });
        let Err(error) = load_model(&doc, &[]) else {
            panic!("a document without meshes or extras must not load");
        };
        assert_eq!(error.to_string(), "file has no meshes");
    }

    #[test]
    fn model_whose_meshes_are_all_hidden_loads_empty() {
        let doc = triangle_doc(
            json!([{"mesh": 0, "extras": {"hidden": true}}]),
            json!([0]),
            &json!({}),
        );
        assert!(
            load_model(&doc, &positions_bin())
                .unwrap()
                .meshes
                .is_empty()
        );
        assert!(load(&doc, &positions_bin()).unwrap().is_empty());
    }

    #[test]
    fn rejected_extras_do_not_make_an_empty_document_valid() {
        let doc = json!({
            "asset": {"version": "2.0"},
            "scenes": [{"nodes": [0]}],
            "nodes": [{"extras": {"particle_system": {"v": 99}}}]
        });
        assert!(load_model(&doc, &[]).is_err());
    }

    #[test]
    fn mesh_emitter_geometry_is_kept_apart_from_drawable_meshes() {
        let system = json!({
            "v": 1, "world_space": true, "max_particles": 10, "material": 0,
            "fade": {"near_begin": 0, "near_end": 0, "far_begin": 0, "far_end": 0},
            "data": {"aspect_ratio": 1, "aspect_flags": 0, "speed_to_aspect": [0, 0, 0]},
            "emitters": [{
                "type": "mesh", "name": "E", "node": "EmitterNode", "mesh_nodes": ["Source"],
                "emission_type": "face_surface",
                "speed": 1, "speed_variation": 0, "declination": 0, "declination_variation": 0,
                "planar_angle": 0, "planar_angle_variation": 0,
                "initial_radius": 1, "radius_variation": 0, "life_span": 1,
                "life_span_variation": 0,
                "emitter_ctlr": {"birth_rate": {"constant": 5}, "active": {"constant": true},
                                 "cycle": "loop", "start": 0, "stop": 1}
            }],
            "modifiers": [], "update": {"frequency": 1, "phase": 0}
        });
        let doc = triangle_doc(
            json!([
                {"name": "Sys", "extras": {"particle_system": system}},
                {"name": "Source", "mesh": 0, "translation": [0, 0, 1]},
                {"name": "Visible", "mesh": 0}
            ]),
            json!([0, 1, 2]),
            &json!({}),
        );
        let model = load_model(&doc, &positions_bin()).unwrap();
        assert_eq!(model.meshes.len(), 1);
        assert!(close(model.meshes[0].vertices[0].position, [0.0, 0.0, 0.0]));
        let source = &model.emitter_meshes["Source"];
        assert!(close(source[0].vertices[0].position, [0.0, 0.0, 1.0]));
        assert_eq!(model.extras.particle_systems.len(), 1);
    }

    #[test]
    fn point_primitives_feed_only_the_emitter_geometry_and_are_never_drawn() {
        let system = json!({
            "v": 1, "world_space": true, "max_particles": 10, "material": 0,
            "fade": {"near_begin": 0, "near_end": 0, "far_begin": 0, "far_end": 0},
            "data": {"aspect_ratio": 1, "aspect_flags": 0, "speed_to_aspect": [0, 0, 0]},
            "emitters": [{
                "type": "mesh", "name": "E", "node": "Sys", "mesh_nodes": ["Source"],
                "emission_type": "vertices",
                "speed": 1, "speed_variation": 0, "declination": 0, "declination_variation": 0,
                "planar_angle": 0, "planar_angle_variation": 0,
                "initial_radius": 1, "radius_variation": 0, "life_span": 1,
                "life_span_variation": 0,
                "emitter_ctlr": {"birth_rate": {"constant": 5}, "active": {"constant": true},
                                 "cycle": "loop", "start": 0, "stop": 1}
            }],
            "modifiers": [], "update": {"frequency": 1, "phase": 0}
        });
        let doc = triangle_doc(
            json!([
                {"name": "Sys", "extras": {"particle_system": system}},
                {"name": "Source", "mesh": 0},
                {"name": "Stray", "mesh": 0}
            ]),
            json!([0, 1, 2]),
            &json!({"mode": 0}),
        );

        let model = load_model(&doc, &positions_bin()).unwrap();

        assert!(model.meshes.is_empty(), "a point set is not drawable");
        let source = &model.emitter_meshes["Source"];
        assert_eq!(source[0].topology, Topology::Points);
        assert_eq!(source[0].vertices.len(), 3);
        assert_eq!(
            source[0].vertices[0].normal, [0.0; 3],
            "a point set without normals has no direction"
        );
    }

    #[test]
    fn triangle_primitives_are_triangles_and_unsupported_modes_are_skipped() {
        let triangles = triangle_doc(json!([{"mesh": 0}]), json!([0]), &json!({}));
        assert_eq!(
            load(&triangles, &positions_bin()).unwrap()[0].topology,
            Topology::Triangles
        );
        let strip = triangle_doc(json!([{"mesh": 0}]), json!([0]), &json!({"mode": 5}));
        assert!(load(&strip, &positions_bin()).is_err());
    }

    #[test]
    fn document_without_any_mesh_node_is_an_error() {
        let doc = json!({
            "asset": {"version": "2.0"},
            "scenes": [{"nodes": [0]}],
            "nodes": [{}]
        });
        let Err(error) = load(&doc, &[]) else {
            panic!("a document without meshes must not load");
        };
        assert_eq!(error.to_string(), "file has no meshes");
    }

    #[test]
    fn mirrored_node_flips_the_tangent_handedness_sign() {
        let mut bin = positions_bin();
        for tangent in [[1.0f32, 0.0, 0.0, 1.0]; 3] {
            bin.extend(tangent.iter().flat_map(|f| f.to_le_bytes()));
        }
        let mut doc = triangle_doc(
            json!([{"mesh": 0, "scale": [-1, 1, 1]}]),
            json!([0]),
            &json!({"attributes": {"POSITION": 0, "TANGENT": 1}}),
        );
        doc["buffers"][0]["byteLength"] = json!(bin.len());
        doc["bufferViews"] = json!([
            {"buffer": 0, "byteOffset": 0, "byteLength": 36},
            {"buffer": 0, "byteOffset": 36, "byteLength": 48}
        ]);
        doc["accessors"].as_array_mut().unwrap().push(json!({
            "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC4"
        }));
        let mesh = &load(&doc, &bin).unwrap()[0];
        assert_eq!(mesh.vertices[0].tangent[3], -1.0);
        assert!(close(
            [
                mesh.vertices[0].tangent[0],
                mesh.vertices[0].tangent[1],
                mesh.vertices[0].tangent[2]
            ],
            [-1.0, 0.0, 0.0]
        ));
    }

    #[test]
    fn handedness_is_negative_only_for_mirroring_matrices() {
        assert_eq!(handedness(Mat3::IDENTITY), 1.0);
        assert_eq!(
            handedness(Mat3::from_diagonal(Vec3::new(-1.0, 1.0, 1.0))),
            -1.0
        );
        assert_eq!(
            handedness(Mat3::from_diagonal(Vec3::new(-1.0, -1.0, 1.0))),
            1.0
        );
        assert_eq!(handedness(Mat3::ZERO), 1.0);
    }

    #[test]
    fn cofactor_matrix_keeps_normals_perpendicular_under_non_uniform_scale() {
        // Surface direction (1, -1, 0) is perpendicular to normal (1, 1, 0);
        // after scaling x by 2 the transformed normal must still be perpendicular
        // to the transformed surface direction.
        let m = Mat3::from_diagonal(Vec3::new(2.0, 1.0, 1.0));
        let surface = m * Vec3::new(1.0, -1.0, 0.0);
        let normal = cofactor_normal_matrix(m) * Vec3::new(1.0, 1.0, 0.0);
        assert!(surface.dot(normal).abs() < 1e-6);
    }

    #[test]
    fn cofactor_matrix_of_a_mirror_keeps_normals_pointing_outward() {
        let m = Mat3::from_diagonal(Vec3::new(-1.0, 1.0, 1.0));
        assert_eq!(cofactor_normal_matrix(m) * Vec3::X, Vec3::NEG_X);
    }

    #[test]
    fn cofactor_matrix_of_a_singular_matrix_is_finite() {
        let m = Mat3::from_diagonal(Vec3::new(0.0, 1.0, 1.0));
        assert!(
            cofactor_normal_matrix(m)
                .to_cols_array()
                .iter()
                .all(|v| v.is_finite())
        );
    }

    #[test]
    fn meshes_under_a_hidden_node_are_not_drawn() {
        let doc = triangle_doc(
            json!([
                {"name": "Root", "children": [1, 3]},
                {"name": "Stage", "children": [2], "extras": {"hidden": true}},
                {"name": "UnderStage", "mesh": 0},
                {"name": "Shown", "mesh": 0, "translation": [0, 0, 7]}
            ]),
            json!([0]),
            &json!({}),
        );
        let model = load_model(&doc, &positions_bin()).unwrap();
        assert_eq!(model.meshes.len(), 1);
        assert!(close(model.meshes[0].vertices[0].position, [0.0, 0.0, 7.0]));
    }

    #[test]
    fn hidden_emitter_source_stays_available_but_is_not_drawn() {
        let system = json!({
            "v": 1, "world_space": true, "max_particles": 10, "material": 0,
            "fade": {"near_begin": 0, "near_end": 0, "far_begin": 0, "far_end": 0},
            "data": {"aspect_ratio": 1, "aspect_flags": 0, "speed_to_aspect": [0, 0, 0]},
            "emitters": [{
                "type": "mesh", "name": "E", "node": "EmitterNode", "mesh_nodes": ["Source"],
                "emission_type": "face_surface",
                "speed": 1, "speed_variation": 0, "declination": 0, "declination_variation": 0,
                "planar_angle": 0, "planar_angle_variation": 0,
                "initial_radius": 1, "radius_variation": 0, "life_span": 1,
                "life_span_variation": 0,
                "emitter_ctlr": {"birth_rate": {"constant": 5}, "active": {"constant": true},
                                 "cycle": "loop", "start": 0, "stop": 1}
            }],
            "modifiers": [], "update": {"frequency": 1, "phase": 0}
        });
        let doc = triangle_doc(
            json!([
                {"name": "Sys", "extras": {"particle_system": system}},
                {"name": "Source", "mesh": 0, "extras": {"hidden": true}}
            ]),
            json!([0, 1]),
            &json!({}),
        );
        let model = load_model(&doc, &positions_bin()).unwrap();
        assert!(model.meshes.is_empty());
        assert_eq!(model.emitter_meshes["Source"].len(), 1);
        assert_eq!(model.extras.particle_systems.len(), 1);
    }
}
