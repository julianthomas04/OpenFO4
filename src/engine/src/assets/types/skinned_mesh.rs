use glam::{Mat3, Mat4, Vec3};

use crate::anim::gltf_import::read_inverse_binds;
use crate::anim::{AnimationAsset, ImportError, JointIndex, Skeleton, SkinData};
use crate::assets::types::mesh::{CpuMesh, cofactor_normal_matrix, handedness};
use crate::assets::types::model_extras::hidden_nodes;
use crate::rendering::frustum::Aabb;
use crate::rendering::skinning::skin_vertex::SkinVertex;
use crate::rendering::vertex::Vertex;

#[derive(Debug, thiserror::Error)]
pub enum SkinnedMeshError {
    #[error(transparent)]
    Gltf(#[from] gltf::Error),
    #[error("skin data: {0}")]
    SkinData(#[from] ImportError),
    #[error("file has no skinned primitives")]
    NoSkinnedPrimitives,
    #[error(
        "mesh node {0:?} has primitives but no skin, and no skeleton joint of that name to carry it"
    )]
    NodeWithoutSkin(String),
    #[error("skin {skin} joint {joint:?} is not a bone of the skeleton")]
    UnknownJoint { skin: usize, joint: String },
    #[error("primitive {primitive} of mesh node {node:?} lacks JOINTS_0 or WEIGHTS_0")]
    MissingSkinAttributes { node: String, primitive: usize },
    #[error(
        "vertex {vertex} of mesh node {node:?} uses joint slot {slot} but its skin has {joints} joints"
    )]
    JointOutOfRange {
        node: String,
        vertex: usize,
        slot: u16,
        joints: usize,
    },
    #[error(
        "vertex {vertex} of mesh node {node:?} has weights that do not sum to a positive number"
    )]
    UnweightedVertex { node: String, vertex: usize },
    #[error(
        "primitive {primitive} of mesh node {node:?} has {vertices} vertices but {skin_vertices} skin vertices"
    )]
    SkinVertexCountMismatch {
        node: String,
        primitive: usize,
        vertices: usize,
        skin_vertices: usize,
    },
    #[error("skinned mesh has no vertices, so it has no bounds")]
    NoVertices,
}

pub struct SkinnedPart {
    pub vertices: Vec<Vertex>,
    pub skin_vertices: Vec<SkinVertex>,
    pub indices: Vec<u32>,
    /// Encoded material name, read like the static meshes' `CpuMesh::texture_path`.
    pub material_name: Option<String>,
    /// Index into `SkinnedMesh::inverse_binds`.
    pub skin: usize,
}

pub struct SkinnedMesh {
    pub parts: Vec<SkinnedPart>,
    /// Per skin, the inverse bind matrix of every skeleton joint (identity for
    /// joints the skin does not use), so a palette is `model[j] * inverse_bind[j]`.
    pub inverse_binds: Vec<Vec<Mat4>>,
}

impl SkinnedMesh {
    /// Vertex joint indices are remapped from skin-local slots to skeleton joint
    /// indices by bone name. Mesh node transforms are ignored for skinned nodes,
    /// as glTF specifies: vertices are already in bind space. A mesh node
    /// without a skin is a rigid part riding the skeleton joint of its own name.
    pub fn load(path: &str, skeleton: &Skeleton) -> Result<Self, SkinnedMeshError> {
        let (document, buffers) = open_glb(path)?;
        let skins = AnimationAsset::from_document(&document, &buffers)?.skins;
        Self::build(&document, &buffers, &skins, skeleton)
    }

    /// A model whose skeleton is its own node hierarchy; the file is read once.
    pub fn load_with_skeleton(path: &str) -> Result<(Skeleton, Self), SkinnedMeshError> {
        let (document, buffers) = open_glb(path)?;
        let asset = AnimationAsset::from_document(&document, &buffers)?;
        let mesh = Self::build(&document, &buffers, &asset.skins, &asset.skeleton)?;
        Ok((asset.skeleton, mesh))
    }

    fn build(
        document: &gltf::Document,
        buffers: &[gltf::buffer::Data],
        skin_data: &[SkinData],
        skeleton: &Skeleton,
    ) -> Result<Self, SkinnedMeshError> {
        let joint_maps = skin_data
            .iter()
            .enumerate()
            .map(|(skin, data)| joint_map(skin, &data.joint_names, skeleton))
            .collect::<Result<Vec<_>, _>>()?;
        let mut inverse_binds: Vec<Vec<Mat4>> = skin_data
            .iter()
            .zip(&joint_maps)
            .map(|(data, map)| inverse_bind_by_joint(&data.inverse_bind, map, skeleton.len()))
            .collect();

        let mut parts = Vec::new();
        let mut rigid_skin = None;
        let hidden = hidden_nodes(document);
        for node in document.nodes().filter(|node| !hidden[node.index()]) {
            let Some(mesh) = node.mesh() else { continue };
            let node_name = node.name().unwrap_or("").to_owned();
            if let Some(skin) = node.skin() {
                let skin = skin.index();
                collect_node_parts(
                    &mesh,
                    &node_name,
                    skin,
                    &joint_maps[skin],
                    buffers,
                    &mut parts,
                )?;
            } else {
                let joint = Some(node_name.as_str())
                    .filter(|name| !name.is_empty())
                    .and_then(|name| skeleton.index_of(name))
                    .ok_or_else(|| SkinnedMeshError::NodeWithoutSkin(node_name.clone()))?;
                let skin = *rigid_skin.get_or_insert_with(|| {
                    inverse_binds.push(vec![Mat4::IDENTITY; skeleton.len()]);
                    inverse_binds.len() - 1
                });
                collect_rigid_parts(&mesh, joint, skin, buffers, &mut parts);
            }
        }
        if parts.is_empty() {
            return Err(SkinnedMeshError::NoSkinnedPrimitives);
        }
        Ok(Self {
            parts,
            inverse_binds,
        })
    }

    /// Per skeleton joint, the largest bind-space distance to a vertex it
    /// influences; 0 for a joint no vertex uses.
    pub fn joint_reaches(&self) -> Vec<f32> {
        let joint_positions: Vec<Vec<Vec3>> = self
            .inverse_binds
            .iter()
            .map(|skin| {
                skin.iter()
                    .map(|inverse_bind| inverse_bind.inverse().w_axis.truncate())
                    .collect()
            })
            .collect();
        let mut reaches = vec![0.0_f32; self.inverse_binds.first().map_or(0, Vec::len)];
        for part in &self.parts {
            let positions = &joint_positions[part.skin];
            for (vertex, skin) in part.vertices.iter().zip(&part.skin_vertices) {
                let position = Vec3::from(vertex.position);
                for (&joint, weight) in skin.joints.iter().zip(skin.weights) {
                    if weight > 0.0 {
                        let reach = &mut reaches[joint as usize];
                        *reach = reach.max(position.distance(positions[joint as usize]));
                    }
                }
            }
        }
        reaches
    }

    pub fn bind_bounds(&self) -> Option<Aabb> {
        Aabb::from_points(
            self.parts
                .iter()
                .flat_map(|part| part.vertices.iter().map(|v| Vec3::from(v.position))),
        )
    }
}

fn open_glb(path: &str) -> Result<(gltf::Document, Vec<gltf::buffer::Data>), SkinnedMeshError> {
    let gltf = gltf::Gltf::open(path)?;
    let base = std::path::Path::new(path).parent();
    let buffers = gltf::import_buffers(&gltf.document, base, gltf.blob)?;
    Ok((gltf.document, buffers))
}

fn joint_map(
    skin: usize,
    joint_names: &[String],
    skeleton: &Skeleton,
) -> Result<Vec<JointIndex>, SkinnedMeshError> {
    joint_names
        .iter()
        .map(|name| {
            skeleton
                .index_of(name)
                .ok_or_else(|| SkinnedMeshError::UnknownJoint {
                    skin,
                    joint: name.clone(),
                })
        })
        .collect()
}

fn inverse_bind_by_joint(
    inverse_bind: &[Mat4],
    joint_map: &[JointIndex],
    joint_count: usize,
) -> Vec<Mat4> {
    let mut by_joint = vec![Mat4::IDENTITY; joint_count];
    for (&joint, &matrix) in joint_map.iter().zip(inverse_bind) {
        by_joint[joint as usize] = matrix;
    }
    by_joint
}

fn collect_node_parts(
    mesh: &gltf::Mesh,
    node_name: &str,
    skin: usize,
    joint_map: &[JointIndex],
    buffers: &[gltf::buffer::Data],
    out: &mut Vec<SkinnedPart>,
) -> Result<(), SkinnedMeshError> {
    for primitive in mesh.primitives() {
        let Some((cpu, skin_vertices)) =
            read_skinned_primitive(&primitive, node_name, joint_map, buffers)?
        else {
            continue;
        };
        out.push(SkinnedPart {
            vertices: cpu.vertices,
            skin_vertices,
            indices: cpu.indices,
            material_name: cpu.texture_path,
            skin,
        });
    }
    Ok(())
}

/// Primitives of an unskinned node, in node space and fully weighted to `joint`.
fn collect_rigid_parts(
    mesh: &gltf::Mesh,
    joint: JointIndex,
    skin: usize,
    buffers: &[gltf::buffer::Data],
    out: &mut Vec<SkinnedPart>,
) {
    for primitive in mesh.primitives() {
        let Some(cpu) = CpuMesh::from_primitive(&primitive, buffers, Mat4::IDENTITY) else {
            continue;
        };
        let skin_vertices = vec![
            SkinVertex {
                joints: [joint, 0, 0, 0],
                weights: [1.0, 0.0, 0.0, 0.0]
            };
            cpu.vertices.len()
        ];
        out.push(SkinnedPart {
            vertices: cpu.vertices,
            skin_vertices,
            indices: cpu.indices,
            material_name: cpu.texture_path,
            skin,
        });
    }
}

/// `None` for a primitive without positions.
fn read_skinned_primitive(
    primitive: &gltf::Primitive,
    node_name: &str,
    joint_map: &[JointIndex],
    buffers: &[gltf::buffer::Data],
) -> Result<Option<(CpuMesh, Vec<SkinVertex>)>, SkinnedMeshError> {
    let Some(cpu) = CpuMesh::from_primitive(primitive, buffers, Mat4::IDENTITY) else {
        return Ok(None);
    };
    let skin_vertices = read_skin_vertices(primitive, node_name, joint_map, buffers)?;
    if skin_vertices.len() != cpu.vertices.len() {
        return Err(SkinnedMeshError::SkinVertexCountMismatch {
            node: node_name.to_owned(),
            primitive: primitive.index(),
            vertices: cpu.vertices.len(),
            skin_vertices: skin_vertices.len(),
        });
    }
    Ok(Some((cpu, skin_vertices)))
}

/// A skin's `node_world * inverse_bind` palette at the file's rest node pose,
/// so a prop authored posed (node pose != bind pose) draws in its node pose.
/// The skinned node's own transform is ignored, as glTF specifies.
pub(crate) struct RestPose {
    palette: Vec<Mat4>,
    slots: Vec<JointIndex>,
}

impl RestPose {
    pub(crate) fn new(
        skin: &gltf::Skin,
        buffers: &[gltf::buffer::Data],
        node_worlds: &[Mat4],
    ) -> Result<Self, SkinnedMeshError> {
        let inverse_binds = read_inverse_binds(skin, buffers)?;
        let palette: Vec<Mat4> = skin
            .joints()
            .zip(&inverse_binds)
            .map(|(joint, &inverse_bind)| node_worlds[joint.index()] * inverse_bind)
            .collect();
        let slots = (0..palette.len() as JointIndex).collect();
        Ok(Self { palette, slots })
    }

    /// `None` for a primitive without positions.
    pub(crate) fn skin_primitive(
        &self,
        primitive: &gltf::Primitive,
        node_name: &str,
        buffers: &[gltf::buffer::Data],
    ) -> Result<Option<CpuMesh>, SkinnedMeshError> {
        let Some((mut cpu, skin_vertices)) =
            read_skinned_primitive(primitive, node_name, &self.slots, buffers)?
        else {
            return Ok(None);
        };
        for (vertex, skin_vertex) in cpu.vertices.iter_mut().zip(&skin_vertices) {
            skin_vertex_in_place(vertex, skin_vertex, &self.palette);
        }
        Ok(Some(cpu))
    }
}

/// Linear blend skinning of one vertex; weights are already normalized and
/// zero-weight lanes are skipped.
fn skin_vertex_in_place(vertex: &mut Vertex, skin: &SkinVertex, palette: &[Mat4]) {
    let blended = skin
        .joints
        .iter()
        .zip(skin.weights)
        .filter(|&(_, weight)| weight > 0.0)
        .fold(Mat4::ZERO, |sum, (&joint, weight)| {
            sum + palette[joint as usize] * weight
        });
    let linear = Mat3::from_mat4(blended);
    let position = blended.transform_point3(Vec3::from(vertex.position));
    let normal = (cofactor_normal_matrix(linear) * Vec3::from(vertex.normal)).normalize_or_zero();
    let tangent = (linear * Vec3::new(vertex.tangent[0], vertex.tangent[1], vertex.tangent[2]))
        .normalize_or_zero();
    vertex.position = position.to_array();
    vertex.normal = normal.to_array();
    vertex.tangent = [
        tangent.x,
        tangent.y,
        tangent.z,
        vertex.tangent[3] * handedness(linear),
    ];
}

fn read_skin_vertices(
    primitive: &gltf::Primitive,
    node_name: &str,
    joint_map: &[JointIndex],
    buffers: &[gltf::buffer::Data],
) -> Result<Vec<SkinVertex>, SkinnedMeshError> {
    let reader = primitive.reader(|b| Some(&buffers[b.index()]));
    let (Some(joints), Some(weights)) = (reader.read_joints(0), reader.read_weights(0)) else {
        return Err(SkinnedMeshError::MissingSkinAttributes {
            node: node_name.to_owned(),
            primitive: primitive.index(),
        });
    };

    joints
        .into_u16()
        .zip(weights.into_f32())
        .enumerate()
        .map(|(vertex, (slots, weights))| {
            let sum: f32 = weights.iter().sum();
            if !(sum.is_finite() && sum > 0.0) {
                return Err(SkinnedMeshError::UnweightedVertex {
                    node: node_name.to_owned(),
                    vertex,
                });
            }
            let mut joints = [0; 4];
            let mut normalized = [0.0; 4];
            for (lane, (&slot, &weight)) in slots.iter().zip(&weights).enumerate() {
                if weight <= 0.0 {
                    continue;
                }
                joints[lane] = *joint_map.get(slot as usize).ok_or_else(|| {
                    SkinnedMeshError::JointOutOfRange {
                        node: node_name.to_owned(),
                        vertex,
                        slot,
                        joints: joint_map.len(),
                    }
                })?;
                normalized[lane] = weight / sum;
            }
            Ok(SkinVertex {
                joints,
                weights: normalized,
            })
        })
        .collect()
}
