//! Grass models: GLBs whose vertices carry a colour (`COLOR_0`), which vanilla
//! uses as the blade tint (rgb) and the sway weight (alpha). The shared
//! `Vertex` has no colour, so grass has its own vertex layout. Parsing happens
//! on the streaming workers (`GrassModelCache`); the render thread only uploads.

use std::collections::HashMap;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

use crate::assets::layout::MESHES_DIR;
use crate::assets::types::animated_mesh::ModelLoadError;
use crate::assets::types::grass::CellGrass;
use crate::assets::types::mesh::CpuMesh;
use crate::shared_cache::SharedCache;
use crate::utils::warn_once;

/// Vertex colour of a model without one: untinted, and a zero sway weight so it stands still.
const NO_COLOR: [u8; 4] = [255, 255, 255, 0];

/// Grass types in the catalog number in the dozens; the cap only bounds a
/// pathological mod load order.
const MODEL_CACHE_CAP: u64 = 512;

/// Byte size of one `GrassBlade` instance; the layout below is pinned to it.
pub const BLADE_STRIDE: u64 = 48;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct GrassVertex {
    pub position: [f32; 3],
    pub tex_coords: [f32; 2],
    pub normal: [f32; 3],
    pub tangent: [f32; 4],
    pub color: [u8; 4],
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x3,
    1 => Float32x2,
    2 => Float32x3,
    3 => Float32x4,
    4 => Unorm8x4,
];

/// `GrassBlade`: position @0, height factor @12, right axis @16, tint @28, up axis @32.
const BLADE_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    5 => Float32x3,
    6 => Float32,
    7 => Float32x3,
    8 => Float32,
    9 => Float32x3,
];

pub fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<GrassVertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &VERTEX_ATTRIBUTES,
    }
}

/// `GrassBlade` read as one instance per blade.
pub fn blade_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: BLADE_STRIDE,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &BLADE_ATTRIBUTES,
    }
}

/// One primitive of a grass model.
pub struct GrassMeshCpu {
    pub vertices: Vec<GrassVertex>,
    /// False for shapes the exporter filled with the `NO_COLOR` placeholder: vanilla
    /// renders those with the instancing permutations that have no colour input.
    pub has_color_stream: bool,
    pub indices: Vec<u32>,
    pub material_name: Option<String>,
}

impl GrassMeshCpu {
    /// Farthest vertex from the model origin.
    pub fn radius(&self) -> f32 {
        self.vertices
            .iter()
            .map(|vertex| Vec3::from(vertex.position).length())
            .fold(0.0, f32::max)
    }
}

/// A parsed grass model: what the render thread uploads.
pub struct GrassModelCpu {
    pub meshes: Vec<GrassMeshCpu>,
    pub radius: f32,
}

impl GrassModelCpu {
    pub fn from_meshes(meshes: Vec<CpuMesh>, source: &str) -> Self {
        let meshes: Vec<GrassMeshCpu> = meshes
            .into_iter()
            .map(|mesh| grass_mesh(mesh, source))
            .collect();
        let radius = meshes.iter().map(GrassMeshCpu::radius).fold(0.0, f32::max);
        Self { meshes, radius }
    }

    pub fn load(file: &str) -> Result<Self, ModelLoadError> {
        let meshes = CpuMesh::load(file).map_err(ModelLoadError::Mesh)?;
        Ok(Self::from_meshes(meshes, file))
    }
}

/// The installer exports a shape without a colour stream as one constant
/// `NO_COLOR` for every vertex.
fn has_color_stream(colors: Option<&[[u8; 4]]>) -> bool {
    colors.is_some_and(|colors| colors.iter().any(|&color| color != NO_COLOR))
}

fn grass_mesh(cpu: CpuMesh, source: &str) -> GrassMeshCpu {
    if cpu.vertex_colors.is_none() {
        warn_once(&format!("grass-no-color:{source}"), || {
            format!(
                "grass model {source} has no usable COLOR_0; it renders untinted and without sway"
            )
        });
    }
    let colors = cpu.vertex_colors.as_deref();
    let vertices = cpu
        .vertices
        .iter()
        .enumerate()
        .map(|(index, vertex)| GrassVertex {
            position: vertex.position,
            tex_coords: vertex.tex_coords,
            normal: vertex.normal,
            tangent: vertex.tangent,
            color: colors.map_or(NO_COLOR, |colors| colors[index]),
        })
        .collect();
    GrassMeshCpu {
        vertices,
        has_color_stream: has_color_stream(colors),
        indices: cpu.indices,
        material_name: cpu.texture_path,
    }
}

/// The parsed models a cell's grass uses, by the `model_path` of its `CellGrass`.
pub type GrassModels = HashMap<String, Arc<GrassModelCpu>>;

/// Worker-side model cache shared by the streaming pool: each model is parsed
/// once however many cells place it, and a model that fails is reported once
/// and remembered as missing.
pub struct GrassModelCache {
    asset_root: String,
    cache: SharedCache<String, Option<Arc<GrassModelCpu>>>,
}

impl GrassModelCache {
    pub fn new(asset_root: &str) -> Self {
        Self {
            asset_root: asset_root.to_owned(),
            cache: SharedCache::new(MODEL_CACHE_CAP),
        }
    }

    /// Parses every model `grass` places; models that cannot load are absent
    /// from the result and their grass is skipped at integration.
    pub fn prepare(&self, grass: &[CellGrass]) -> GrassModels {
        let mut paths: Vec<String> = grass.iter().map(|g| g.model_path.clone()).collect();
        paths.sort_unstable();
        paths.dedup();
        self.cache
            .resolve(
                &paths,
                |path| self.load(path),
                |model| u64::from(model.is_some()),
            )
            .into_iter()
            .filter_map(|(path, model)| Some((path, model?)))
            .collect()
    }

    fn load(&self, model_path: &str) -> Option<Arc<GrassModelCpu>> {
        let file = model_file_path(&self.asset_root, model_path);
        match GrassModelCpu::load(&file) {
            Ok(model) => Some(Arc::new(model)),
            Err(error) => {
                warn_once(&format!("grass-model:{model_path}"), || {
                    format!(
                        "grass model '{model_path}' failed to load: {error}; its grass is not drawn"
                    )
                });
                None
            }
        }
    }
}

/// The GLB file of a grass model path: with or without the meshes root (in any
/// case), any separator. The root of the result is always the layout's folder name.
fn model_file_path(asset_root: &str, model_path: &str) -> String {
    let normalized = model_path.replace('\\', "/");
    let relative = normalized.trim_start_matches('/');
    let below_root = relative
        .as_bytes()
        .get(..=MESHES_DIR.len())
        .filter(|head| {
            head[..MESHES_DIR.len()].eq_ignore_ascii_case(MESHES_DIR.as_bytes())
                && head[MESHES_DIR.len()] == b'/'
        })
        .map_or(relative, |head| &relative[head.len()..]);
    format!("{asset_root}/{MESHES_DIR}/{below_root}")
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals parsed or copied, not computed"
)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    /// Binary glTF 2.0 container: header, a JSON chunk and a BIN chunk, each padded to 4 bytes.
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
        body.extend(chunk(*b"BIN\0", bin, 0));
        let mut out = b"glTF".to_vec();
        out.extend(2u32.to_le_bytes());
        out.extend(((12 + body.len()) as u32).to_le_bytes());
        out.extend(body);
        out
    }

    /// One triangle at (0,0,0), (3,0,0), (0,4,0): 36 position bytes, then 12 colour bytes.
    fn triangle(with_color: bool, node: &Value) -> GrassModelCpu {
        let mut bin: Vec<u8> = [[0.0f32, 0.0, 0.0], [3.0, 0.0, 0.0], [0.0, 4.0, 0.0]]
            .iter()
            .flatten()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        bin.extend([10u8, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120]);
        let mut attributes = json!({"POSITION": 0});
        let mut accessors = json!([{
            "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
            "min": [0, 0, 0], "max": [3, 4, 0]
        }]);
        if with_color {
            attributes["COLOR_0"] = json!(1);
            accessors.as_array_mut().unwrap().push(json!({
                "bufferView": 1, "componentType": 5121, "normalized": true, "count": 3, "type": "VEC4"
            }));
        }
        let doc = json!({
            "asset": {"version": "2.0"},
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": [node],
            "meshes": [{"primitives": [{"attributes": attributes, "material": 0}]}],
            "materials": [{"name": "g_d.dds|g_n.dds|test:102|ts"}],
            "buffers": [{"byteLength": bin.len()}],
            "bufferViews": [
                {"buffer": 0, "byteOffset": 0, "byteLength": 36},
                {"buffer": 0, "byteOffset": 36, "byteLength": 12}
            ],
            "accessors": accessors
        });
        let gltf = gltf::Gltf::from_slice(&glb(&doc, &bin)).unwrap();
        let buffers = gltf::import_buffers(&gltf.document, None, gltf.blob).unwrap();
        let meshes = CpuMesh::from_document(&gltf.document, &buffers, "test.glb").unwrap();
        GrassModelCpu::from_meshes(meshes, "test.glb")
    }

    #[test]
    fn vertex_colours_are_read_per_vertex_with_alpha() {
        let model = triangle(true, &json!({"mesh": 0}));
        let colors: Vec<[u8; 4]> = model.meshes[0].vertices.iter().map(|v| v.color).collect();
        assert_eq!(
            colors,
            vec![[10, 20, 30, 40], [50, 60, 70, 80], [90, 100, 110, 120]]
        );
    }

    #[test]
    fn a_model_without_colours_is_untinted_and_does_not_sway() {
        let model = triangle(false, &json!({"mesh": 0}));
        assert!(model.meshes[0].vertices.iter().all(|v| v.color == NO_COLOR));
    }

    #[test]
    fn only_a_model_with_a_real_colour_stream_counts_as_coloured() {
        assert!(triangle(true, &json!({"mesh": 0})).meshes[0].has_color_stream);
        assert!(!triangle(false, &json!({"mesh": 0})).meshes[0].has_color_stream);
    }

    #[test]
    fn the_exporters_constant_placeholder_colour_is_not_a_colour_stream() {
        assert!(!has_color_stream(Some(&[NO_COLOR, NO_COLOR])));
        assert!(!has_color_stream(None));
        assert!(has_color_stream(Some(&[NO_COLOR, [255, 255, 255, 40]])));
    }

    #[test]
    fn the_material_name_and_node_transform_are_kept() {
        let model = triangle(true, &json!({"mesh": 0, "translation": [1, 0, 0]}));
        let mesh = &model.meshes[0];
        assert_eq!(
            mesh.material_name.as_deref(),
            Some("g_d.dds|g_n.dds|test:102|ts")
        );
        assert_eq!(mesh.vertices[1].position, [4.0, 0.0, 0.0]);
        assert_eq!(mesh.indices, vec![0, 1, 2]);
    }

    #[test]
    fn the_model_radius_is_the_farthest_vertex_from_the_origin() {
        assert_eq!(triangle(true, &json!({"mesh": 0})).radius, 4.0);
    }

    #[test]
    fn the_radius_of_a_small_model_is_not_rounded_up() {
        let model = triangle(true, &json!({"mesh": 0, "scale": [0.125, 0.125, 0.125]}));
        assert_eq!(model.radius, 0.5);
    }

    #[test]
    fn a_missing_model_file_is_a_typed_load_error() {
        let error = GrassModelCpu::load("no/such/model.glb").err().unwrap();
        assert!(matches!(error, ModelLoadError::Mesh(_)), "{error}");
    }

    #[test]
    fn the_cache_leaves_models_that_fail_to_load_out_of_the_result() {
        let cache = GrassModelCache::new("no/such/root");
        let grass = [CellGrass {
            model_path: "landscape/grass/missing.glb".to_owned(),
            flags: crate::assets::types::grass::GrassFlags(0),
            wave_period: 1.0,
            blades: Vec::new(),
        }];
        assert!(cache.prepare(&grass).is_empty());
    }

    #[test]
    fn vertex_layout_offsets_match_the_vertex_struct() {
        let layout = vertex_layout();
        assert_eq!(layout.array_stride, 52);
        let offsets: Vec<u64> = layout.attributes.iter().map(|a| a.offset).collect();
        assert_eq!(
            offsets,
            vec![
                std::mem::offset_of!(GrassVertex, position) as u64,
                std::mem::offset_of!(GrassVertex, tex_coords) as u64,
                std::mem::offset_of!(GrassVertex, normal) as u64,
                std::mem::offset_of!(GrassVertex, tangent) as u64,
                std::mem::offset_of!(GrassVertex, color) as u64,
            ]
        );
    }

    /// Byte offsets of `GrassBlade`: position, `height_k`, right, tint, up.
    const BLADE_OFFSETS: [u64; 5] = [0, 12, 16, 28, 32];

    #[test]
    fn blade_layout_reads_the_contract_offsets_per_instance() {
        let layout = blade_layout();
        assert_eq!(layout.array_stride, BLADE_STRIDE);
        assert_eq!(layout.step_mode, wgpu::VertexStepMode::Instance);
        let offsets: Vec<u64> = layout.attributes.iter().map(|a| a.offset).collect();
        assert_eq!(offsets, BLADE_OFFSETS);
    }

    #[test]
    fn blade_stride_is_the_blade_struct_size() {
        assert_eq!(
            std::mem::size_of::<crate::assets::types::grass::GrassBlade>() as u64,
            BLADE_STRIDE
        );
    }

    /// The shader builds the blade frame from the instance's `right` and `up`
    /// axes (columns 0 and 1) and takes column 2 as `cross(right, up)`. That
    /// holds for any right-handed rotation.
    #[test]
    fn the_third_frame_column_is_right_cross_up_for_a_known_rotation() {
        let rotation = glam::Mat3::from_quat(
            glam::Quat::from_rotation_y(0.7) * glam::Quat::from_rotation_x(-0.4),
        );
        let third = rotation.x_axis.cross(rotation.y_axis);
        assert!((third - rotation.z_axis).length() < 1e-6);
    }

    #[test]
    fn a_meshes_rooted_backslash_path_keeps_its_root() {
        assert_eq!(
            model_file_path("assets", "Meshes\\Landscape\\Grass\\Bent.glb"),
            "assets/meshes/Landscape/Grass/Bent.glb"
        );
    }

    #[test]
    fn the_root_is_matched_without_regard_to_case_and_written_in_the_layout_case() {
        assert_eq!(
            model_file_path("assets", "MESHES/landscape/grass/bent.glb"),
            "assets/meshes/landscape/grass/bent.glb"
        );
    }

    #[test]
    fn a_path_relative_to_the_meshes_root_gains_it() {
        assert_eq!(
            model_file_path("assets", "landscape/grass/bent.glb"),
            "assets/meshes/landscape/grass/bent.glb"
        );
    }

    #[test]
    fn a_folder_that_merely_starts_with_meshes_is_not_the_root() {
        assert_eq!(
            model_file_path("assets", "meshesextra/a.glb"),
            "assets/meshes/meshesextra/a.glb"
        );
    }
}
