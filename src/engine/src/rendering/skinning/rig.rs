use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::{Arc, Mutex};

use glam::{Mat4, Vec3};
use serde::Deserialize;

use super::gpu_mesh::GpuSkinnedMesh;
use super::materials::{CharacterGpuContext, MaterialOverride, PartPolicy};
use crate::anim::{AnimationAsset, Clip, ImportError, PoseError, Skeleton, local_to_model};
use crate::assets::layout::MESHES_DIR;
use crate::assets::types::skinned_mesh::{SkinnedMesh, SkinnedMeshError};
use crate::rendering::frustum::Aabb;

pub const CHARACTER_ASSETS_DIR: &str = "CharacterAssets";
pub const ANIM_INDEX_FILE: &str = "skeleton.anims.json";
/// Slack around the animated joints for what the offset bound does not cover (engine units).
const BOUNDS_PAD: f32 = 1.0;

/// Asset paths of one creature. `assets_dir` is the folder holding the unit's
/// animation index, relative to the asset root; bodies are file names inside it.
pub struct CreatureDef {
    pub name: String,
    pub assets_dir: String,
    pub bodies: Vec<String>,
}

/// One clip pack as listed by the converter's index: the clip names are known
/// without opening the (possibly huge) pack file.
#[derive(Debug, Deserialize)]
pub struct ClipPackEntry {
    pub file: String,
    pub clips: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AnimIndex {
    /// The converted NIF skeleton: the full render hierarchy the bodies are
    /// skinned to. The clips (Havok, a subset of its bones) bind to it by name.
    skeleton: String,
    packs: Vec<ClipPackEntry>,
}

#[derive(Debug, thiserror::Error)]
pub enum CreatureLoadError {
    #[error("animation index '{path}' does not exist; run the installer's animation conversion")]
    IndexMissing { path: String },
    #[error("animation index '{path}' cannot be read: {source}")]
    IndexUnreadable {
        path: String,
        source: std::io::Error,
    },
    #[error("animation index '{path}' is malformed: {source}")]
    IndexMalformed {
        path: String,
        source: serde_json::Error,
    },
    #[error("animation index '{path}' names skeleton '{name}', which is not a relative path")]
    SkeletonPathInvalid { path: String, name: String },
    #[error(
        "animation index '{path}' names skeleton '{name}', which leaves the '{MESHES_DIR}' folder"
    )]
    SkeletonEscapesMeshes { path: String, name: String },
    #[error("skeleton '{path}' does not exist")]
    SkeletonMissing { path: String },
    #[error("skeleton '{path}': {source}")]
    Skeleton { path: String, source: ImportError },
    #[error("skeleton '{0}' has no joints")]
    EmptySkeleton(String),
    #[error(transparent)]
    RestPose(#[from] PoseError),
    #[error("creature has no body meshes")]
    NoBodies,
    #[error("body '{path}' does not exist")]
    BodyMissing { path: String },
    #[error("body '{path}': {source}")]
    Body {
        path: String,
        source: SkinnedMeshError,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum PackLoadError {
    #[error("clip pack index {index} is out of range, the creature has {count} packs")]
    UnknownPack { index: usize, count: usize },
    #[error("clip pack '{path}' does not exist")]
    Missing { path: String },
    #[error("clip pack '{path}': {source}")]
    Import { path: String, source: ImportError },
}

/// The clips of one pack file, as imported.
pub struct LoadedPack {
    clips: Vec<Clip>,
    by_name: HashMap<String, usize>,
}

impl LoadedPack {
    fn new(clips: Vec<Clip>) -> Self {
        let by_name = clips
            .iter()
            .enumerate()
            .map(|(index, clip)| (clip.name().to_owned(), index))
            .collect();
        Self { clips, by_name }
    }

    pub fn clip_named(&self, name: &str) -> Option<&Clip> {
        self.by_name.get(name).map(|&index| &self.clips[index])
    }
}

/// Everything shared by all instances of one creature. Clip packs load on first use.
pub struct CreatureRig {
    pub name: String,
    pub skeleton: Skeleton,
    pub meshes: Vec<GpuSkinnedMesh>,
    rest_model_pose: Vec<Mat4>,
    pose_pad: f32,
    /// Per skeleton joint, the largest distance to a vertex it influences, over all bodies.
    joint_reach: Vec<f32>,
    pack_dir: String,
    packs: Vec<ClipPackEntry>,
    loaded_packs: Mutex<HashMap<usize, Arc<LoadedPack>>>,
    /// `[mesh][skin]` -> ordinal of that skin's palette slice within an instance.
    skin_slots: Vec<Vec<usize>>,
    skin_slot_count: usize,
}

impl CreatureRig {
    pub fn load(
        def: &CreatureDef,
        asset_root: &str,
        gpu: &mut CharacterGpuContext,
    ) -> Result<Self, CreatureLoadError> {
        let pack_dir = format!("{asset_root}/{}", def.assets_dir);
        let index_path = format!("{pack_dir}/{ANIM_INDEX_FILE}");
        let index = read_index(&index_path)?;
        let skeleton_path = resolve_skeleton_path(&def.assets_dir, &index.skeleton)
            .map_err(|fault| fault.into_error(&index_path, &index.skeleton))?;
        let skeleton = load_skeleton(&format!("{asset_root}/{skeleton_path}"))?;

        let rest_model_pose = rest_model_pose(&skeleton)?;
        let meshes = upload_bodies(def, &pack_dir, &skeleton, gpu)?;
        Ok(Self::assemble(
            def.name.clone(),
            skeleton,
            rest_model_pose,
            meshes,
            pack_dir,
            index.packs,
        ))
    }

    /// A placed prop's own skinned model as a rig, with the placement's
    /// material swaps applied. It has no clip packs; `name` is the model path.
    pub fn from_prop_model(
        name: &str,
        model: &PropModel,
        overrides: &[MaterialOverride],
        gpu: &mut CharacterGpuContext,
    ) -> Result<Self, CreatureLoadError> {
        let mesh = GpuSkinnedMesh::upload(&model.mesh, PartPolicy::Prop(overrides), gpu).map_err(
            |source| CreatureLoadError::Body {
                path: name.to_owned(),
                source,
            },
        )?;
        Ok(Self::assemble(
            name.to_owned(),
            model.skeleton.clone(),
            model.rest_model_pose.clone(),
            vec![mesh],
            String::new(),
            Vec::new(),
        ))
    }

    fn assemble(
        name: String,
        skeleton: Skeleton,
        rest_model_pose: Vec<Mat4>,
        meshes: Vec<GpuSkinnedMesh>,
        pack_dir: String,
        packs: Vec<ClipPackEntry>,
    ) -> Self {
        let mut joint_reach = vec![0.0_f32; skeleton.len()];
        for mesh in &meshes {
            for (total, &reach) in joint_reach.iter_mut().zip(&mesh.joint_reaches) {
                *total = total.max(reach);
            }
        }
        let pose_pad = joint_reach.iter().copied().fold(0.0, f32::max) + BOUNDS_PAD;

        let mut skin_slot_count = 0;
        let skin_slots = meshes
            .iter()
            .map(|mesh| {
                mesh.inverse_binds
                    .iter()
                    .map(|_| {
                        skin_slot_count += 1;
                        skin_slot_count - 1
                    })
                    .collect()
            })
            .collect();

        Self {
            name,
            skeleton,
            meshes,
            rest_model_pose,
            pose_pad,
            joint_reach,
            pack_dir,
            packs,
            loaded_packs: Mutex::new(HashMap::new()),
            skin_slots,
            skin_slot_count,
        }
    }

    pub fn packs(&self) -> &[ClipPackEntry] {
        &self.packs
    }

    pub fn rest_model_pose(&self) -> &[Mat4] {
        &self.rest_model_pose
    }

    /// Loads the pack on first request and shares it afterwards.
    pub fn load_pack(&self, pack: usize) -> Result<Arc<LoadedPack>, PackLoadError> {
        let entry = self.packs.get(pack).ok_or(PackLoadError::UnknownPack {
            index: pack,
            count: self.packs.len(),
        })?;
        let mut loaded = self
            .loaded_packs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(existing) = loaded.get(&pack) {
            return Ok(Arc::clone(existing));
        }
        let path = format!("{}/{}", self.pack_dir, entry.file);
        if !Path::new(&path).is_file() {
            return Err(PackLoadError::Missing { path });
        }
        let clips = AnimationAsset::load(&path)
            .map_err(|source| PackLoadError::Import { path, source })?
            .clips;
        let pack_clips = Arc::new(LoadedPack::new(clips));
        loaded.insert(pack, Arc::clone(&pack_clips));
        Ok(pack_clips)
    }

    /// Model-space bounds of a pose: the joint positions padded by the largest
    /// distance any vertex keeps from its joints.
    pub fn pose_bounds(&self, model_pose: &[Mat4]) -> Aabb {
        let mut bounds = Aabb {
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
        };
        for joint in model_pose {
            let position = joint.w_axis.truncate();
            bounds.min = bounds.min.min(position);
            bounds.max = bounds.max.max(position);
        }
        Aabb {
            min: bounds.min - Vec3::splat(self.pose_pad),
            max: bounds.max + Vec3::splat(self.pose_pad),
        }
    }

    /// Per joint, the largest distance to a vertex it influences; 0 when it influences none.
    pub fn joint_reach(&self) -> &[f32] {
        &self.joint_reach
    }

    /// Largest distance any vertex keeps from its joints, plus a safety margin.
    pub fn pose_pad(&self) -> f32 {
        self.pose_pad
    }

    pub fn palette_len(&self) -> usize {
        self.skin_slot_count * self.skeleton.len()
    }

    /// Palette index, relative to an instance's offset, of the matrices a part's skin uses.
    pub fn palette_start(&self, mesh: usize, skin: usize) -> usize {
        self.skin_slots[mesh][skin] * self.skeleton.len()
    }

    /// Appends `model[joint] * inverse_bind[joint]` for every skin, in `palette_start` order.
    pub fn write_palette(&self, model_pose: &[Mat4], out: &mut Vec<Mat4>) {
        for mesh in &self.meshes {
            for inverse_bind in &mesh.inverse_binds {
                out.extend(
                    model_pose
                        .iter()
                        .zip(inverse_bind)
                        .map(|(model, inverse)| *model * *inverse),
                );
            }
        }
    }

    pub fn release_textures(&self, gpu: &mut CharacterGpuContext) {
        release_mesh_textures(&self.meshes, gpu);
    }
}

fn release_mesh_textures(meshes: &[GpuSkinnedMesh], gpu: &mut CharacterGpuContext) {
    let references: Vec<_> = meshes
        .iter()
        .flat_map(GpuSkinnedMesh::texture_refs)
        .collect();
    gpu.release_textures(&references);
}

fn read_index(path: &str) -> Result<AnimIndex, CreatureLoadError> {
    let text = std::fs::read_to_string(path).map_err(|source| match source.kind() {
        ErrorKind::NotFound => CreatureLoadError::IndexMissing {
            path: path.to_owned(),
        },
        _ => CreatureLoadError::IndexUnreadable {
            path: path.to_owned(),
            source,
        },
    })?;
    serde_json::from_str(&text).map_err(|source| CreatureLoadError::IndexMalformed {
        path: path.to_owned(),
        source,
    })
}

enum SkeletonPathFault {
    NotRelative,
    EscapesMeshes,
}

impl SkeletonPathFault {
    fn into_error(self, index_path: &str, name: &str) -> CreatureLoadError {
        let (path, name) = (index_path.to_owned(), name.to_owned());
        match self {
            Self::NotRelative => CreatureLoadError::SkeletonPathInvalid { path, name },
            Self::EscapesMeshes => CreatureLoadError::SkeletonEscapesMeshes { path, name },
        }
    }
}

/// Resolves the index's `skeleton` against the index folder (both relative to
/// the asset root) into a normalized asset-root-relative path, lexically: `..`
/// may climb out of the unit but never out of the meshes folder.
fn resolve_skeleton_path(assets_dir: &str, skeleton: &str) -> Result<String, SkeletonPathFault> {
    let name = skeleton.replace('\\', "/");
    if name.is_empty() || name.starts_with('/') || name.contains(':') {
        return Err(SkeletonPathFault::NotRelative);
    }
    let mut resolved: Vec<&str> = assets_dir
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    for part in name.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                resolved.pop().ok_or(SkeletonPathFault::EscapesMeshes)?;
            }
            normal => resolved.push(normal),
        }
    }
    if resolved.len() < 2 || resolved[0] != MESHES_DIR {
        return Err(SkeletonPathFault::EscapesMeshes);
    }
    Ok(resolved.join("/"))
}

fn load_skeleton(path: &str) -> Result<Skeleton, CreatureLoadError> {
    if !Path::new(path).is_file() {
        return Err(CreatureLoadError::SkeletonMissing {
            path: path.to_owned(),
        });
    }
    let skeleton = AnimationAsset::load(path)
        .map_err(|source| CreatureLoadError::Skeleton {
            path: path.to_owned(),
            source,
        })?
        .skeleton;
    if skeleton.is_empty() {
        return Err(CreatureLoadError::EmptySkeleton(path.to_owned()));
    }
    Ok(skeleton)
}

/// A failing body releases the textures of the bodies uploaded before it.
fn upload_bodies(
    def: &CreatureDef,
    pack_dir: &str,
    skeleton: &Skeleton,
    gpu: &mut CharacterGpuContext,
) -> Result<Vec<GpuSkinnedMesh>, CreatureLoadError> {
    if def.bodies.is_empty() {
        return Err(CreatureLoadError::NoBodies);
    }
    let mut meshes = Vec::with_capacity(def.bodies.len());
    for body in &def.bodies {
        match upload_body(&format!("{pack_dir}/{body}"), skeleton, gpu) {
            Ok(mesh) => meshes.push(mesh),
            Err(error) => {
                release_mesh_textures(&meshes, gpu);
                return Err(error);
            }
        }
    }
    Ok(meshes)
}

fn upload_body(
    path: &str,
    skeleton: &Skeleton,
    gpu: &mut CharacterGpuContext,
) -> Result<GpuSkinnedMesh, CreatureLoadError> {
    if !Path::new(path).is_file() {
        return Err(CreatureLoadError::BodyMissing {
            path: path.to_owned(),
        });
    }
    let into_error = |source| CreatureLoadError::Body {
        path: path.to_owned(),
        source,
    };
    let skinned = SkinnedMesh::load(path, skeleton).map_err(into_error)?;
    GpuSkinnedMesh::upload(&skinned, PartPolicy::Creature, gpu).map_err(into_error)
}

fn rest_model_pose(skeleton: &Skeleton) -> Result<Vec<Mat4>, PoseError> {
    let mut pose = vec![Mat4::IDENTITY; skeleton.len()];
    local_to_model(skeleton, skeleton.rest_pose(), &mut pose)?;
    Ok(pose)
}

/// A placed prop's model as read from disk: pure CPU data, so it can be parsed
/// off the render thread and uploaded later by `CreatureRig::from_prop_model`.
/// Its skeleton is the model's own node hierarchy.
pub struct PropModel {
    skeleton: Skeleton,
    mesh: SkinnedMesh,
    rest_model_pose: Vec<Mat4>,
}

impl PropModel {
    /// `path` is the full file path.
    pub fn parse(path: &str) -> Result<Self, CreatureLoadError> {
        if !Path::new(path).is_file() {
            return Err(CreatureLoadError::BodyMissing {
                path: path.to_owned(),
            });
        }
        let (skeleton, mesh) =
            SkinnedMesh::load_with_skeleton(path).map_err(|source| CreatureLoadError::Body {
                path: path.to_owned(),
                source,
            })?;
        if skeleton.is_empty() {
            return Err(CreatureLoadError::EmptySkeleton(path.to_owned()));
        }
        let rest_model_pose = rest_model_pose(&skeleton)?;
        Ok(Self {
            skeleton,
            mesh,
            rest_model_pose,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(assets_dir: &str, skeleton: &str) -> Option<String> {
        resolve_skeleton_path(assets_dir, skeleton).ok()
    }

    #[test]
    fn a_skeleton_beside_the_index_resolves_below_the_meshes_folder() {
        assert_eq!(
            resolved("meshes/Actors/Dog/CharacterAssets", "skeleton.glb").as_deref(),
            Some("meshes/Actors/Dog/CharacterAssets/skeleton.glb")
        );
    }

    #[test]
    fn a_skeleton_may_climb_out_of_its_unit_but_not_out_of_the_meshes_folder() {
        assert_eq!(
            resolved(
                "meshes/Actors/Turret/CharacterAssets/Mounted",
                "..\\TurretMountedSkeleton.glb"
            )
            .as_deref(),
            Some("meshes/Actors/Turret/CharacterAssets/TurretMountedSkeleton.glb")
        );
        assert!(resolved("meshes/Actors/Dog", "../../../skeleton.glb").is_none());
    }

    #[test]
    fn a_unit_outside_the_meshes_folder_of_the_layout_is_rejected() {
        assert!(resolved("textures/Actors/Dog", "skeleton.glb").is_none());
    }
}
