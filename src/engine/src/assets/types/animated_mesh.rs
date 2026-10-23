use std::collections::HashSet;
use std::sync::Arc;

use glam::{Mat4, Vec3};

use super::mesh::CpuMesh;
use super::model_extras::{ModelExtras, has_hidden_mesh};
use crate::anim::{AnimationAsset, AnimationRig, ClipRoles, ImportError, JointIndex, RigError};
use crate::rendering::frustum::Aabb;

#[derive(Debug, thiserror::Error)]
pub enum ModelLoadError {
    #[error(transparent)]
    Gltf(#[from] gltf::Error),
    #[error("animation import failed: {0}")]
    Import(#[from] ImportError),
    #[error("animation rig failed: {0}")]
    Rig(#[from] RigError),
    #[error("mesh node {0:?} is missing from the skeleton")]
    PartWithoutJoint(String),
    #[error("file has no meshes")]
    NoMeshes,
    #[error(transparent)]
    Mesh(#[from] anyhow::Error),
}

/// One primitive whose vertices stay in its owning node's space; the node's
/// animated model-space transform places it.
pub struct AnimatedPart {
    pub joint: JointIndex,
    pub mesh: CpuMesh,
}

pub struct AnimatedModel {
    pub rig: Arc<AnimationRig>,
    pub parts: Vec<AnimatedPart>,
}

pub enum LoadedModel {
    /// Node transforms baked into the vertices.
    Static(Vec<CpuMesh>),
    Animated(AnimatedModel),
}

/// A model file as the streamer consumes it: the drawable model plus the node-level extras.
pub struct ModelAsset {
    pub model: LoadedModel,
    pub extras: ModelExtras,
}

fn has_animation_data(document: &gltf::Document) -> bool {
    document.animations().next().is_some()
        || document.nodes().any(|node| {
            node.extras()
                .as_ref()
                .is_some_and(|raw| raw.get().contains("\"anim\""))
        })
}

impl LoadedModel {
    pub fn load(path: &str) -> Result<ModelAsset, ModelLoadError> {
        let gltf = gltf::Gltf::open(path)?;
        let base = std::path::Path::new(path).parent();
        let buffers = gltf::import_buffers(&gltf.document, base, gltf.blob)?;
        let document = &gltf.document;

        if !has_animation_data(document) {
            let static_model = CpuMesh::model_from_document(document, &buffers, path)?;
            return Ok(ModelAsset {
                model: Self::Static(static_model.meshes),
                extras: static_model.extras,
            });
        }
        let asset = AnimationAsset::from_document(document, &buffers)?;
        report_import_warnings(path, &asset);
        let mut rig = AnimationRig::new(asset)?;
        let extras = ModelExtras::from_document(document);
        let parts = collect_parts(document, &buffers, &rig, &extras.hidden_nodes)?;
        if parts.is_empty() && !has_hidden_mesh(document, &extras.hidden_nodes) {
            return Err(ModelLoadError::NoMeshes);
        }
        rig.set_joint_reach(joint_reaches(&parts, rig.asset.skeleton.len()));
        if let Some(open) = rig.roles().open.filter(|_| rig.roles().open_is_fallback) {
            crate::utils::info_once(&format!("anim-fallback:{path}"), || {
                ClipRoles::missing_open_motion_message(path, &rig.asset.clips, open)
            });
        }
        let rig = Arc::new(rig);
        Ok(ModelAsset {
            model: Self::Animated(AnimatedModel { rig, parts }),
            extras,
        })
    }
}

/// Per joint, the largest distance from its origin to a corner of an attached
/// part's bounds (parts are in their joint's space); 0 for joints without parts.
fn joint_reaches(parts: &[AnimatedPart], joint_count: usize) -> Vec<f32> {
    let mut reaches = vec![0.0_f32; joint_count];
    for part in parts {
        let Some(bounds) = Aabb::from_points(
            part.mesh
                .vertices
                .iter()
                .map(|vertex| Vec3::from(vertex.position)),
        ) else {
            continue;
        };
        let reach = bounds
            .corners()
            .iter()
            .map(|corner| corner.length())
            .fold(0.0, f32::max);
        let slot = &mut reaches[part.joint as usize];
        *slot = slot.max(reach);
    }
    reaches
}

fn report_import_warnings(path: &str, asset: &AnimationAsset) {
    for warning in &asset.warnings {
        crate::utils::warn_once(&format!("anim-import:{path}:{warning}"), || {
            format!("anim: {path}: {warning}")
        });
    }
}

/// Mesh nodes reachable from a scene, or every mesh node when no scene lists any.
fn mesh_nodes(document: &gltf::Document) -> Vec<gltf::Node<'_>> {
    fn visit<'a>(node: &gltf::Node<'a>, seen: &mut HashSet<usize>, out: &mut Vec<gltf::Node<'a>>) {
        if !seen.insert(node.index()) {
            return;
        }
        if node.mesh().is_some() {
            out.push(node.clone());
        }
        for child in node.children() {
            visit(&child, seen, out);
        }
    }

    let mut seen = HashSet::new();
    let mut nodes = Vec::new();
    for scene in document.scenes() {
        for root in scene.nodes() {
            visit(&root, &mut seen, &mut nodes);
        }
    }
    if nodes.is_empty() {
        nodes.extend(document.nodes().filter(|node| node.mesh().is_some()));
    }
    nodes
}

fn collect_parts(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    rig: &AnimationRig,
    hidden_nodes: &[bool],
) -> Result<Vec<AnimatedPart>, ModelLoadError> {
    let mut parts = Vec::new();
    for node in mesh_nodes(document)
        .into_iter()
        .filter(|node| !hidden_nodes[node.index()])
    {
        let name = node.name().unwrap_or_default();
        let joint = rig
            .joint_by_name(name)
            .ok_or_else(|| ModelLoadError::PartWithoutJoint(name.to_owned()))?;
        let Some(mesh) = node.mesh() else { continue };
        let mut meshes = Vec::new();
        CpuMesh::collect_primitives(&mesh, buffers, Mat4::IDENTITY, &mut meshes);
        parts.extend(meshes.into_iter().map(|mesh| AnimatedPart { joint, mesh }));
    }
    Ok(parts)
}
