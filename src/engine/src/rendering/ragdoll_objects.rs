//! Rendering of ragdolls. A ragdoll-capable placement draws its static rows
//! until the sim publishes a pose for it (its bodies moved, or the REFR carries
//! an authored XRGD pose); from then on the static rows are hidden and a
//! GPU-skinned instance of the same model, posed from the body motions, takes
//! over. The skinned instance keeps the last pose until the
//! placement's render cell retires, whatever the sim does meanwhile. The sim owns
//! the pose: a placement the renderer does not know yet is asked for again
//! (`republish`) once it becomes resident, and one the sim remembers while unloaded
//! hides its static rows at once so the rest pose never flashes.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use glam::Mat4;
use tracing::debug;

use super::ragdoll_sim_link as sim_link;
use super::row_pose::{PoseSource, RowPose};
use super::skinning::character::CharacterId;
use super::skinning::materials::CharacterGpuContext;
use super::skinning::{CharacterSystem, PropLook};
use crate::anim::Skeleton;
use crate::sim::ragdoll_spec::{RagdollBody, RagdollSpec};
use crate::sim::{BodyDelta, EntityId, RagdollPoses, SimWorld};
use crate::utils::warn_once;

/// Largest rest offset (engine units) between a sidecar body and its skeleton joint before it is reported.
const REST_TRANSLATION_TOLERANCE: f32 = 0.05;
/// Largest rest rotation difference (radians) between a sidecar body and its skeleton joint before it is reported.
const REST_ROTATION_TOLERANCE: f32 = 0.01;

/// How a ragdoll-capable REFR is drawn once its bodies move.
#[derive(Clone)]
pub enum SkinPlan {
    /// A skinned instance with the placement's material swaps.
    Skin(PropLook),
    /// The REFR's rows are effect or marker geometry the static path hides
    /// anyway, so there is nothing to replace.
    HiddenByDefault,
    /// A material swap the skinned path cannot express; the model is reverted
    /// to static collision instead of drawing it wrong.
    Unsupported(String),
}

/// One ragdoll-capable REFR a freshly integrated cell hands to the runtime.
pub struct RagdollRefr {
    pub entity: EntityId,
    /// Model path relative to the asset root.
    pub model_path: String,
    pub world: Mat4,
    pub skin: SkinPlan,
}

pub enum RagdollEvent {
    Added(RagdollRefr),
    Removed(EntityId),
    /// Every cell was torn down at once (worldspace or interior switch).
    Cleared,
}

/// A change to the static rows of one entity, applied through the row writer.
pub(crate) struct RowOps {
    ops: Vec<(EntityId, RowPose<'static>)>,
}

impl RowOps {
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

impl PoseSource for RowOps {
    fn len(&self) -> usize {
        self.ops.len()
    }

    fn entity(&self, index: usize) -> EntityId {
        self.ops[index].0
    }

    fn pose(&self, index: usize) -> Option<RowPose<'_>> {
        Some(match self.ops.get(index)?.1 {
            RowPose::Rigid {
                translation,
                rotation,
            } => RowPose::Rigid {
                translation,
                rotation,
            },
            RowPose::Hidden { at } => RowPose::Hidden { at },
            RowPose::Skeletal { .. } => return None,
        })
    }
}

/// The last pose the sim published for a resident ragdoll, kept so a skinned
/// instance that is still loading can be posed once ready.
struct MovedPose {
    spec: Arc<RagdollSpec>,
    deltas: Vec<BodyDelta>,
}

struct Skin {
    id: CharacterId,
    /// Skeleton joint -> index of the body driving it; resolved once the rig is known.
    body_of_joint: Option<Vec<Option<usize>>>,
    dirty: bool,
    static_rows_hidden: bool,
}

#[derive(Default)]
pub struct RagdollObjects {
    refrs: HashMap<EntityId, RagdollRefr>,
    moved: HashMap<EntityId, MovedPose>,
    skins: HashMap<EntityId, Skin>,
    /// Entities whose skinned replacement cannot be built.
    failed: HashSet<EntityId>,
    /// Models the sim was told to stop simulating.
    disabled_models: HashSet<String>,
    poses: RagdollPoses,
    row_ops: Vec<(EntityId, RowPose<'static>)>,
    model_pose: Vec<Mat4>,
}

impl RagdollObjects {
    /// Applies residency events and this frame's sim poses, poses the skinned
    /// instances, and returns the static-row changes to write. A skinned
    /// instance whose model is ready is spawned, posed and its rows hidden
    /// within this one call, so no frame draws both.
    pub(crate) fn update(
        &mut self,
        events: Vec<RagdollEvent>,
        sim: &mut SimWorld,
        characters: &mut CharacterSystem,
        gpu: &mut CharacterGpuContext,
    ) -> RowOps {
        self.row_ops.clear();
        self.apply_events(events, sim, characters);
        let mut poses = std::mem::take(&mut self.poses);
        sim.ragdoll_poses(&mut poses);
        self.apply_releases(&poses, characters);
        self.take_poses(&poses, characters);
        self.poses = poses;
        characters.apply_pending(gpu);
        self.forget_failed_spawns(sim, characters);
        self.pose_skins(characters);
        RowOps {
            ops: std::mem::take(&mut self.row_ops),
        }
    }

    fn apply_events(
        &mut self,
        events: Vec<RagdollEvent>,
        sim: &mut SimWorld,
        characters: &mut CharacterSystem,
    ) {
        for event in events {
            match event {
                RagdollEvent::Added(refr) => self.add_refr(refr, sim, characters),
                RagdollEvent::Removed(entity) => {
                    self.refrs.remove(&entity);
                    self.failed.remove(&entity);
                    if let Some(skin) = self.skins.remove(&entity) {
                        characters.queue_despawn(skin.id);
                    }
                    self.moved.remove(&entity);
                }
                RagdollEvent::Cleared => {
                    self.refrs.clear();
                    self.failed.clear();
                    self.moved.clear();
                    for (_, skin) in self.skins.drain() {
                        characters.queue_despawn(skin.id);
                    }
                }
            }
        }
    }

    fn add_refr(
        &mut self,
        refr: RagdollRefr,
        sim: &mut SimWorld,
        characters: &mut CharacterSystem,
    ) {
        let entity = refr.entity;
        self.failed.remove(&entity);
        if let SkinPlan::Unsupported(reason) = &refr.skin {
            warn_once(&format!("ragdoll-unsupported:{}", refr.model_path), || {
                format!(
                    "ragdoll: {} cannot be drawn skinned ({reason})",
                    refr.model_path
                )
            });
            self.failed.insert(entity);
            self.disable_model(sim, &refr.model_path);
        }
        self.refrs.insert(entity, refr);
        sim_link::republish(sim, entity);
        if sim_link::has_pose(sim, entity) {
            self.hide_rows_for_sim_pose(entity, characters);
        }
        self.refresh_skin(entity, characters);
    }

    /// The sim will publish a pose for the entity (authored, or remembered from an
    /// earlier visit): the skinned instance is queued and the static rows hidden
    /// now rather than drawn at rest first.
    fn hide_rows_for_sim_pose(&mut self, entity: EntityId, characters: &mut CharacterSystem) {
        if !self.ensure_skin(entity, characters) {
            return;
        }
        let (Some(refr), Some(skin)) = (self.refrs.get(&entity), self.skins.get_mut(&entity))
        else {
            return;
        };
        if !std::mem::replace(&mut skin.static_rows_hidden, true) {
            self.row_ops.push((
                entity,
                RowPose::Hidden {
                    at: refr.world.w_axis.truncate(),
                },
            ));
        }
    }

    fn apply_releases(&mut self, poses: &RagdollPoses, characters: &mut CharacterSystem) {
        for release in sim_link::releases(poses) {
            let entity = release.entity;
            if release.pose_cached && self.skins.contains_key(&entity) {
                continue;
            }
            self.moved.remove(&entity);
            let Some(skin) = self.skins.remove(&entity) else {
                continue;
            };
            characters.queue_despawn(skin.id);
            self.restore_static_rows(entity, skin.static_rows_hidden);
        }
    }

    fn restore_static_rows(&mut self, entity: EntityId, rows_hidden: bool) {
        if let (true, Some(refr)) = (rows_hidden, self.refrs.get(&entity)) {
            let (_, rotation, translation) = refr.world.to_scale_rotation_translation();
            self.row_ops.push((
                entity,
                RowPose::Rigid {
                    translation,
                    rotation,
                },
            ));
        }
    }

    fn take_poses(&mut self, poses: &RagdollPoses, characters: &mut CharacterSystem) {
        for entry in &poses.entries {
            if !self.is_skinnable(entry.entity) {
                continue; // the sim publishes once; `republish` brings the pose back when the placement becomes resident
            }
            let count = entry.spec.bodies.len();
            let Some(deltas) = poses
                .body_deltas
                .get(entry.first_body..entry.first_body + count)
            else {
                warn_once(&format!("ragdoll-deltas:{:?}", entry.entity), || {
                    format!(
                        "ragdoll: the sim published fewer body poses than the {count} bodies of {:?}",
                        entry.entity
                    )
                });
                continue;
            };
            let moved = self.moved.entry(entry.entity).or_insert_with(|| MovedPose {
                spec: Arc::clone(&entry.spec),
                deltas: Vec::new(),
            });
            moved.spec = Arc::clone(&entry.spec);
            moved.deltas.clear();
            moved.deltas.extend_from_slice(deltas);
            self.refresh_skin(entry.entity, characters);
        }
    }

    fn is_skinnable(&self, entity: EntityId) -> bool {
        !self.failed.contains(&entity)
            && self
                .refrs
                .get(&entity)
                .is_some_and(|refr| matches!(refr.skin, SkinPlan::Skin(_)))
    }

    /// Marks the entity's skinned instance for re-posing, queueing it first when
    /// it has none.
    fn refresh_skin(&mut self, entity: EntityId, characters: &mut CharacterSystem) {
        if self.moved.contains_key(&entity)
            && self.ensure_skin(entity, characters)
            && let Some(skin) = self.skins.get_mut(&entity)
        {
            skin.dirty = true;
        }
    }

    /// True when the entity has a skinned instance, queueing one when it has none.
    fn ensure_skin(&mut self, entity: EntityId, characters: &mut CharacterSystem) -> bool {
        if !self.is_skinnable(entity) {
            return false;
        }
        if self.skins.contains_key(&entity) {
            return true;
        }
        let Some(refr) = self.refrs.get(&entity) else {
            return false;
        };
        let SkinPlan::Skin(look) = &refr.skin else {
            return false;
        };
        let id = characters.queue_prop_spawn(refr.model_path.clone(), look, refr.world);
        self.skins.insert(
            entity,
            Skin {
                id,
                body_of_joint: None,
                dirty: true,
                static_rows_hidden: false,
            },
        );
        true
    }

    fn forget_failed_spawns(&mut self, sim: &mut SimWorld, characters: &mut CharacterSystem) {
        for id in characters.take_failed_spawns() {
            let Some(entity) = self
                .skins
                .iter()
                .find(|(_, skin)| skin.id == id)
                .map(|(entity, _)| *entity)
            else {
                continue;
            };
            if let Some(skin) = self.skins.remove(&entity) {
                self.restore_static_rows(entity, skin.static_rows_hidden);
            }
            self.moved.remove(&entity);
            self.failed.insert(entity);
            if let Some(model_path) = self.refrs.get(&entity).map(|refr| refr.model_path.clone()) {
                self.disable_model(sim, &model_path);
            }
        }
    }

    fn disable_model(&mut self, sim: &mut SimWorld, model_path: &str) {
        if self.disabled_models.insert(model_path.to_owned()) {
            sim_link::disable_model(sim, model_path);
        }
    }

    fn pose_skins(&mut self, characters: &mut CharacterSystem) {
        for (entity, skin) in &mut self.skins {
            if !skin.dirty {
                continue;
            }
            let Some(refr) = self.refrs.get(entity) else {
                warn_once(&format!("ragdoll-skin-orphan:{entity:?}"), || {
                    format!("ragdoll: skinned instance of {entity:?} lost its placement")
                });
                continue;
            };
            let Some(moved) = self.moved.get(entity) else {
                continue; // queued ahead of the sim's remembered pose; the rows stay as they are until it arrives
            };
            let Some(character) = characters.character_mut(skin.id) else {
                continue; // still loading: the static rows keep drawing
            };
            let rig = character.rig();
            let body_of_joint = skin.body_of_joint.get_or_insert_with(|| {
                joint_bodies(
                    &moved.spec,
                    &rig.skeleton,
                    rig.rest_model_pose(),
                    &refr.model_path,
                )
            });
            write_model_pose(
                &refr.world,
                &moved.deltas,
                body_of_joint,
                rig.rest_model_pose(),
                &mut self.model_pose,
            );
            character.set_model_pose(&self.model_pose);
            skin.dirty = false;
            if !skin.static_rows_hidden {
                skin.static_rows_hidden = true;
                self.row_ops.push((
                    *entity,
                    RowPose::Hidden {
                        at: refr.world.w_axis.truncate(),
                    },
                ));
                debug!("ragdoll: skinned instance live for {entity:?}");
            }
        }
    }
}

/// For every skeleton joint, the body of its nearest ancestor-or-self joint that has one.
/// A joint with no body at or above it (the model's root node, a mesh or shape node)
/// follows the ragdoll's root body, so every joint travels with the ragdoll rather
/// than staying at rest.
fn joint_bodies(
    spec: &RagdollSpec,
    skeleton: &Skeleton,
    rest_model: &[Mat4],
    model_path: &str,
) -> Vec<Option<usize>> {
    let mut resolved: Vec<Option<usize>> = vec![None; skeleton.len()];
    for (body_index, body) in spec.bodies.iter().enumerate() {
        match skeleton.index_of(&body.node) {
            Some(joint) => {
                let joint = usize::from(joint);
                resolved[joint].get_or_insert(body_index);
                warn_if_rest_differs(body, &rest_model[joint], model_path);
            }
            None => warn_once(&format!("ragdoll-node:{model_path}:{}", body.node), || {
                format!(
                    "ragdoll: {model_path} has no joint for body node {:?}; it drives nothing",
                    body.node
                )
            }),
        }
    }
    // `Skeleton` guarantees every parent precedes its children, so one forward
    // pass resolves each joint from an already-final parent.
    let root_body = spec.root_body();
    for joint in 0..resolved.len() {
        if resolved[joint].is_none() {
            let inherited =
                skeleton.parents()[joint].and_then(|parent| resolved[usize::from(parent)]);
            resolved[joint] = inherited.or(root_body);
        }
    }
    resolved
}

/// The deltas are relative to the sidecar's rest pose, but they move the skeleton's
/// rest pose; the two must agree or the skin tears away from the bodies.
fn warn_if_rest_differs(body: &RagdollBody, joint_rest: &Mat4, model_path: &str) {
    let (_, rotation, translation) = joint_rest.to_scale_rotation_translation();
    let offset = translation.distance(body.rest.translation);
    let angle = rotation.angle_between(body.rest.rotation);
    if offset > REST_TRANSLATION_TOLERANCE || angle > REST_ROTATION_TOLERANCE {
        warn_once(&format!("ragdoll-rest:{model_path}:{}", body.node), || {
            format!(
                "ragdoll: {model_path} body {:?} rests {offset:.3} units and {angle:.4} rad away from its skeleton joint",
                body.node
            )
        });
    }
}

/// `model[j] = W^-1 * D_b * W * rest[j]`: joint `j` follows the world motion of
/// its body, expressed in the model space the palette works in.
fn write_model_pose(
    world: &Mat4,
    deltas: &[BodyDelta],
    body_of_joint: &[Option<usize>],
    rest_model: &[Mat4],
    out: &mut Vec<Mat4>,
) {
    let world_inverse = world.inverse();
    out.clear();
    out.extend(rest_model.iter().zip(body_of_joint).map(|(rest, body)| {
        match body.and_then(|body| deltas.get(body)) {
            Some(delta) => {
                let motion = Mat4::from_rotation_translation(delta.rotation, delta.translation);
                world_inverse * motion * *world * *rest
            }
            None => *rest,
        }
    }));
}
