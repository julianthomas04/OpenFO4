use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use glam::{Mat4, Vec3};

use super::gpu_mesh::GpuSkinnedMesh;
use super::materials::{CharacterGpuContext, MaterialOverride, PartPolicy};
use crate::anim::character_assets::{
    AnimationFileError, CharacterAssets, CharacterAssetsError, ClipPackEntry, SkeletonLoadError,
    load_animation_file, rest_model_pose,
};
use crate::anim::{Clip, PoseError, Skeleton};
use crate::assets::types::skinned_mesh::{SkinnedMesh, SkinnedMeshError};
use crate::rendering::frustum::Aabb;

pub const CHARACTER_ASSETS_DIR: &str = "CharacterAssets";
/// Slack around the animated joints for what the offset bound does not cover (engine units).
const BOUNDS_PAD: f32 = 1.0;

/// Asset paths of one creature. `assets_dir` is the folder holding the unit's
/// animation index, relative to the asset root; bodies are file names inside it.
pub struct CreatureDef {
    pub name: String,
    pub assets_dir: String,
    pub bodies: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CreatureLoadError {
    #[error(transparent)]
    Assets(#[from] CharacterAssetsError),
    #[error(transparent)]
    Skeleton(#[from] SkeletonLoadError),
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
    #[error("clip pack {0}")]
    File(#[from] AnimationFileError),
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
        let assets = CharacterAssets::open(asset_root, &def.assets_dir)?;
        let skeleton = assets.load_skeleton()?;

        let rest_model_pose = rest_model_pose(&skeleton)?;
        let meshes = upload_bodies(def, &assets.dir, &skeleton, gpu)?;
        Ok(Self::assemble(
            def.name.clone(),
            skeleton,
            rest_model_pose,
            meshes,
            assets.dir,
            assets.packs,
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
        let clips = load_animation_file(&format!("{}/{}", self.pack_dir, entry.file))?.clips;
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
    require_body_file(path)?;
    let into_error = |source| CreatureLoadError::Body {
        path: path.to_owned(),
        source,
    };
    let skinned = SkinnedMesh::load(path, skeleton).map_err(into_error)?;
    GpuSkinnedMesh::upload(&skinned, PartPolicy::Creature, gpu).map_err(into_error)
}

fn require_body_file(path: &str) -> Result<(), CreatureLoadError> {
    if Path::new(path).is_file() {
        Ok(())
    } else {
        Err(CreatureLoadError::BodyMissing {
            path: path.to_owned(),
        })
    }
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
        require_body_file(path)?;
        let (skeleton, mesh) =
            SkinnedMesh::load_with_skeleton(path).map_err(|source| CreatureLoadError::Body {
                path: path.to_owned(),
                source,
            })?;
        if skeleton.is_empty() {
            return Err(SkeletonLoadError::Empty(path.to_owned()).into());
        }
        let rest_model_pose = rest_model_pose(&skeleton)?;
        Ok(Self {
            skeleton,
            mesh,
            rest_model_pose,
        })
    }
}
