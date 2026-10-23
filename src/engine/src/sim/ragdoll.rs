//! Ragdolls for skeleton props. A ragdoll-capable placement starts as its
//! legacy static colliders. Entering the simulation radius swaps them for one
//! rapier body per bone plus joint constraints, every body asleep in the REFR's
//! authored (XRGD) pose, or the model's rest pose without one ("frozen until
//! disturbed"); a contact or push wakes the island and it simulates.
//!
//! Leaving the radius reverts a ragdoll that never moved to the legacy
//! colliders. One that moved stays where it lies: its bodies simply sleep
//! (still `Dynamic`, so nothing pins it in mid-air when the ground under it
//! goes away). When its cell unloads, the last body poses are remembered for
//! the session; the ragdoll respawns at that pose when the cell returns, asleep
//! or awake as it was left.
//!
//! Joint limit mapping (Havok ragdoll atoms -> rapier spherical joint, child
//! frame measured in the parent frame): twist about frame X -> `AngX`; the plane
//! limit bounds the tilt of the child's twist axis toward the parent frame Y,
//! which is a rotation about Z -> `AngZ`; the cone bounds the total deviation of
//! the twist axis from the parent's, so it limits both tilt directions
//! (`AngY` = tilt toward Z, `AngZ` additionally intersected with the plane).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use glam::{Quat, Vec3};
use rapier3d::prelude::*;

use super::bake::{BakedRagdoll, CachedShape};
use super::collider_user_data::{ColliderTag, unpack_ref};
use super::collision::manifest_key;
use super::dynamic_body::{
    ENGINE_UNITS_PER_METER, body_builder, body_finite, density_for_mass, sanitize_body,
    state_is_bounded, within_radius,
};
use super::ragdoll_spec::{JointKind, RagdollConstraint, RagdollSpec, RigidFrame};
use super::ragdoll_tuning::{
    MAX_CONTACT_CORRECTION_DEPTH, RagdollSettings, RestBone, RestTracker, apply_inertia_floor,
    prepare_activation, soften_joint, widen_cone, widen_range,
};
use super::water_physics::{WaterBodyState, WaterTarget};
use super::{CellGrid, EntityId};

/// Most ragdolls the sim keeps live (spawned as rapier bodies) at once, moved or
/// not. Every ragdoll that can ever move is live, so this also bounds the skinned
/// instances the renderer needs. A ragdoll beyond the cap keeps its static
/// colliders and never becomes dynamic, so a moving body never lacks a skin.
pub const MAX_SKINNED_RAGDOLLS: usize = 64;

const POKE_ENV_VAR: &str = "OPENFO4_RAGDOLL_POKE";
const TRACE_ENV_VAR: &str = "OPENFO4_RAGDOLL_TRACE";
/// Seconds of awake simulation between two trace lines of one ragdoll.
const TRACE_INTERVAL: f32 = 1.0;
/// Trace interval while a ragdoll is settling, to show the last second before sleep.
const TRACE_SETTLE_INTERVAL: f32 = 0.25;
/// Speed change the scripted poke gives its body (engine units per second).
const POKE_SPEED: f32 = 30.0;
const POKE_DIRECTION: Vec3 = Vec3::new(0.4, 0.6, 0.3);
/// Ragdolls built per reconcile pass, so a cell crossing that brings hundreds
/// of them into the radius spreads the body builds over several frames.
const MAX_SPAWNS_PER_RECONCILE: usize = 8;
/// A body further than this from its spawn pose (engine units) has moved.
const MOVED_TRANSLATION: f32 = 0.05;
/// `1 - |dot|` of spawn and current rotation above which a body has moved.
const MOVED_ROTATION: f32 = 1.0e-4;
/// Moved ragdolls whose last pose is remembered after their cell unloads.
const MAX_CACHED_POSES: usize = 1024;
/// Steps a ragdoll that respawns frozen may run gravity-free while rapier folds
/// its bodies into one sleeping island.
const FOLD_STEPS: u8 = 3;
/// Awake simulation time between two checks whether a ragdoll touches anything.
const FALL_CHECK_INTERVAL: f32 = 0.5;
/// Awake seconds without a single contact after which a ragdoll is taken to be
/// falling through a hole in the world (its supporting cell unloaded) and is
/// reverted to its static collision instead of simulating forever.
const MAX_FREE_FALL_SECONDS: f32 = 8.0;
/// Margin (engine units) around the removed static colliders within which a
/// body counts as possibly resting on them.
const WAKE_MARGIN: f32 = ENGINE_UNITS_PER_METER;
/// Havok friction torque is N*m; the engine measures length in 1/7 m units, so
/// the same torque is 7^2 times larger in engine units.
const FRICTION_TORQUE_SCALE: f32 = ENGINE_UNITS_PER_METER * ENGINE_UNITS_PER_METER;
/// Joint friction below this torque (engine units) is far under a limb's
/// gravity torque (hinges: 0.01 N*m = 0.5) and is not worth the extra solver rows.
const MIN_FRICTION_TORQUE: f32 = 1.0;
/// Damping that makes the friction motor a hard velocity lock: its force is
/// clipped to the friction torque, so it only ever acts as dry friction.
const FRICTION_MOTOR_DAMPING: f32 = 1.0e4;
/// Penetration (engine units) a bone's contact manifold may already have the
/// first step it carries contacts and still count as an ordinary touch: ten
/// times rapier's allowed linear error. A bone that moves into a surface meets it
/// through predictive contacts at about zero depth, so only a bone that spawned
/// (or a surface that streamed in) inside another collider starts deeper.
const EMBEDDED_DEPTH: f32 = 0.01 * ENGINE_UNITS_PER_METER;
/// Contact manifold `user_data` of a pair involving a ragdoll bone; see `RagdollHooks`.
const MANIFOLD_UNCLASSIFIED: u32 = 0;
const MANIFOLD_TOUCHING: u32 = 1;
const MANIFOLD_EMBEDDED: u32 = 2;

type BodyPoses = Vec<Pose>;

/// A moved ragdoll's remembered state: where every body lay and whether the
/// ragdoll was still simulating when its cell unloaded.
#[derive(Clone)]
struct CachedPose {
    poses: BodyPoses,
    awake: bool,
}

/// Everything `PhysicsContext::add_cell` knows about one ragdoll-capable placement.
pub(crate) struct RagdollPlacement {
    pub grid: CellGrid,
    pub entity: EntityId,
    /// Mesh path of the placement's object group; the renderer names models the same way.
    pub mesh_path: String,
    pub ref_key: String,
    pub scale: f32,
    pub refr_pose: Pose,
    pub tag: ColliderTag,
    /// The legacy single-body shapes, restored when a never-moved ragdoll leaves the radius.
    pub legacy_shapes: Arc<Vec<CachedShape>>,
    pub baked: Arc<BakedRagdoll>,
    /// World pose of every body in the REFR's authored (XRGD) pose; `None`
    /// shows and simulates the model's rest pose.
    pub authored: Option<Arc<Vec<Pose>>>,
}

/// Why a ragdoll-capable placement keeps its static colliders.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RagdollRejection {
    #[error("the placement has no form key")]
    NoFormKey,
    #[error("the placement pose or scale is not finite and positive")]
    BadTransform,
    #[error("a ragdoll is already registered for the entity")]
    Duplicate,
    #[error("the renderer disabled this model's ragdoll")]
    ModelDisabled,
    #[error("the cell payload carries no baked ragdoll")]
    NotBaked,
    #[error("the ragdoll sidecar failed to bake: {0}")]
    BakeFailed(Arc<str>),
}

impl RagdollRejection {
    fn label(&self) -> &'static str {
        match self {
            Self::NoFormKey => "no form key",
            Self::BadTransform => "bad transform",
            Self::Duplicate => "duplicate entity",
            Self::ModelDisabled => "model disabled",
            Self::NotBaked => "not baked",
            Self::BakeFailed(_) => "bake failed",
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum RagdollBuildError {
    #[error("simulated body {0:?} has no collider")]
    BodyWithoutColliders(String),
    #[error("the remembered pose has {found} bodies, the ragdoll {expected}")]
    PoseCacheMismatch { found: usize, expected: usize },
}

/// Body deltas of ragdolls that moved, for the renderer's skinning.
#[derive(Default)]
pub struct RagdollPoses {
    /// One entry per ragdoll whose pose changed since the last collection (or
    /// whose remembered pose was just restored).
    pub entries: Vec<RagdollEntry>,
    /// `entries[i]` owns `spec.bodies.len()` consecutive items starting at `first_body`.
    pub body_deltas: Vec<BodyDelta>,
    /// Ragdolls that had moved and were despawned (their cell left the
    /// collision ring, or the model was disabled).
    pub released: Vec<EntityId>,
    /// The subset of `released` whose last pose the sim remembers: the ragdoll
    /// respawns at that pose, and is published again, when its cell returns.
    pub released_with_cached_pose: Vec<EntityId>,
}

pub struct RagdollEntry {
    pub entity: EntityId,
    pub spec: Arc<RagdollSpec>,
    pub first_body: usize,
}

/// World-space rigid motion of a body from its rest pose to now (no scale).
#[derive(Clone, Copy)]
pub struct BodyDelta {
    pub translation: Vec3,
    pub rotation: Quat,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Gravity is off while rapier folds the bodies into one sleeping island.
    Folding {
        steps_left: u8,
    },
    Active,
}

impl Phase {
    fn label(self) -> &'static str {
        match self {
            Self::Folding { .. } => "folding",
            Self::Active => "active",
        }
    }
}

/// How a live ragdoll is built.
enum SpawnKind<'a> {
    /// Asleep in the authored pose, or the rest pose without one.
    Frozen,
    /// Moved before: respawn at the remembered pose, asleep or awake as left.
    Restore(&'a CachedPose),
}

impl SpawnKind<'_> {
    fn label(&self) -> &'static str {
        match self {
            Self::Frozen => "frozen",
            Self::Restore(_) => "restored pose",
        }
    }
}

struct LiveRagdoll {
    baked: Arc<BakedRagdoll>,
    bodies: Vec<RigidBodyHandle>,
    joints: Vec<ImpulseJointHandle>,
    scale: f32,
    /// Awake simulation time since the last trace line.
    trace_elapsed: f32,
    /// Awake simulation time since the last contact check.
    fall_check_elapsed: f32,
    /// Awake simulation time the ragdoll has gone without touching anything.
    airborne_seconds: f32,
    rest: RestTracker,
    /// Rest pose of every body in world space (placement scale applied).
    rest_world: Vec<Pose>,
    /// Pose every body spawned at; "moved" is measured from here.
    spawn_world: Vec<Pose>,
    phase: Phase,
    ever_moved: bool,
    awake: bool,
    publish: bool,
    /// The scripted poke, applied once the ragdoll is folded into its island.
    poke_pending: bool,
    /// Water bookkeeping of every body (anchors included, never used).
    water: Vec<WaterBodyState>,
}

struct RagdollSlot {
    placement: RagdollPlacement,
    /// Legacy static colliders; empty while the ragdoll is live.
    legacy: Vec<ColliderHandle>,
    /// Reverted to static collision for good (invalid bodies, non-finite state,
    /// or the renderer could not skin the model).
    disabled: bool,
    poke: bool,
}

/// Last body poses of moved ragdolls, kept for the session so a reloaded cell
/// shows them where they lie. Poses only; the oldest entry goes first.
#[derive(Default)]
struct PoseCache {
    entries: HashMap<EntityId, (u64, CachedPose)>,
    clock: u64,
}

impl PoseCache {
    fn store(&mut self, entity: EntityId, pose: CachedPose) {
        self.clock += 1;
        self.entries.insert(entity, (self.clock, pose));
        if self.entries.len() > MAX_CACHED_POSES {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, (clock, _))| *clock)
                .map(|(entity, _)| *entity);
            if let Some(oldest) = oldest {
                self.entries.remove(&oldest);
            }
        }
    }

    fn get(&self, entity: EntityId) -> Option<&CachedPose> {
        self.entries.get(&entity).map(|(_, poses)| poses)
    }

    fn remove(&mut self, entity: EntityId) {
        self.entries.remove(&entity);
    }
}

/// Contact hooks of ragdoll bones; only ragdoll colliders ask for them.
///
/// `filter_contact_pair` rejects contacts between two simulated bones of one
/// ragdoll: bodies carry the ragdoll's entity id as `user_data`.
///
/// `modify_solver_contacts` handles bones authored inside other colliders. Each
/// manifold is classified the first step it has contacts, and the class is kept
/// in its `user_data` until the manifold has no contacts left:
/// - no deeper than `EMBEDDED_DEPTH`: TOUCHING, an ordinary contact.
/// - deeper: EMBEDDED, unless the bone approached fast enough to have travelled
///   that deep within the last step (a genuine impact, which is TOUCHING). An
///   EMBEDDED contact stops the bone from sinking further but never pushes it
///   out, until the bone has left the surface: pushing it out would fight the
///   joints holding it there and keep the ragdoll jittering.
pub(crate) struct RagdollHooks {
    /// Length of one step (seconds).
    dt: f32,
}

impl RagdollHooks {
    fn new(params: &IntegrationParameters) -> Self {
        Self { dt: params.dt }
    }

    fn classify(&self, context: &ContactModificationContext, depth: f32, point: Vec3) -> u32 {
        match *context.user_data {
            MANIFOLD_UNCLASSIFIED if depth <= EMBEDDED_DEPTH => MANIFOLD_TOUCHING,
            MANIFOLD_UNCLASSIFIED => {
                let impact = BoneContact::of(context)
                    .is_none_or(|bone| bone.approach_speed(point) * self.dt >= depth);
                if impact {
                    MANIFOLD_TOUCHING
                } else {
                    MANIFOLD_EMBEDDED
                }
            }
            MANIFOLD_EMBEDDED if depth <= 0.0 => MANIFOLD_TOUCHING,
            state => state,
        }
    }
}

/// The ragdoll bone of a contact manifold and the direction its contacts push it.
struct BoneContact<'a> {
    body: &'a RigidBody,
    partner: Option<&'a RigidBody>,
    push: Vec3,
}

impl<'a> BoneContact<'a> {
    /// The manifold normal points from collider 1 into collider 2; the bone is
    /// the side that asked for the hook.
    fn of(context: &ContactModificationContext<'a>) -> Option<Self> {
        let asks_for_hook = |collider: ColliderHandle| {
            context.colliders.get(collider).is_some_and(|collider| {
                collider
                    .active_hooks()
                    .contains(ActiveHooks::MODIFY_SOLVER_CONTACTS)
            })
        };
        let (bone, partner, push) = if asks_for_hook(context.collider1) {
            (context.rigid_body1, context.rigid_body2, -*context.normal)
        } else {
            (context.rigid_body2, context.rigid_body1, *context.normal)
        };
        Some(Self {
            body: context.bodies.get(bone?)?,
            partner: partner.and_then(|partner| context.bodies.get(partner)),
            push,
        })
    }

    /// Speed at which the bone moves into the surface at `point`.
    fn approach_speed(&self, point: Vec3) -> f32 {
        let partner_velocity = self
            .partner
            .map_or(Vec3::ZERO, |partner| partner.velocity_at_point(point));
        -(self.body.velocity_at_point(point) - partner_velocity).dot(self.push)
    }
}

impl PhysicsHooks for RagdollHooks {
    fn modify_solver_contacts(&self, context: &mut ContactModificationContext) {
        let deepest = context
            .solver_contacts
            .iter()
            .min_by(|first, second| first.dist.total_cmp(&second.dist));
        let Some((depth, point)) =
            deepest.map(|contact| (-contact.dist, contact.anchor1.midpoint(contact.anchor2)))
        else {
            *context.user_data = MANIFOLD_UNCLASSIFIED;
            return;
        };
        *context.user_data = self.classify(context, depth, point);
        let floor = if *context.user_data == MANIFOLD_EMBEDDED {
            0.0
        } else {
            -MAX_CONTACT_CORRECTION_DEPTH
        };
        for contact in context.solver_contacts.iter_mut() {
            contact.dist = contact.dist.max(floor);
        }
    }

    fn filter_contact_pair(&self, context: &PairFilterContext) -> Option<SolverFlags> {
        let same_ragdoll = match (context.rigid_body1, context.rigid_body2) {
            (Some(first), Some(second)) => {
                match (context.bodies.get(first), context.bodies.get(second)) {
                    (Some(first), Some(second)) => {
                        first.user_data != 0
                            && first.user_data == second.user_data
                            && !first.is_fixed()
                            && !second.is_fixed()
                    }
                    _ => false,
                }
            }
            _ => false,
        };
        (!same_ragdoll).then(SolverFlags::default)
    }
}

pub(crate) struct RagdollManager {
    slots: HashMap<EntityId, RagdollSlot>,
    by_cell: HashMap<CellGrid, Vec<EntityId>>,
    /// Ragdolls currently built from rapier bodies; at most `MAX_SKINNED_RAGDOLLS`.
    live: HashMap<EntityId, LiveRagdoll>,
    pose_cache: PoseCache,
    /// `manifest_key`s of models the renderer cannot skin.
    disabled_models: HashSet<String>,
    poke_ids: HashSet<u32>,
    trace: bool,
    released: Vec<EntityId>,
    released_with_cached_pose: Vec<EntityId>,
    /// Rejection reasons already reported.
    reported: HashSet<&'static str>,
    cap_reported: bool,
    region: (CellGrid, i32),
    reconcile_pending: bool,
    scratch: Vec<EntityId>,
    hooks: RagdollHooks,
    settings: RagdollSettings,
    /// Placements not live whose authored pose is published on the next collection.
    publish_authored: HashSet<EntityId>,
}

impl RagdollManager {
    #[cfg(test)]
    pub(super) fn set_refr_translation(&mut self, entity: EntityId, translation: Vec3) {
        let slot = self.slots.get_mut(&entity).expect("a placed ragdoll");
        slot.placement.refr_pose.translation = translation;
    }

    pub fn new(params: &IntegrationParameters) -> Self {
        Self {
            slots: HashMap::new(),
            by_cell: HashMap::new(),
            live: HashMap::new(),
            pose_cache: PoseCache::default(),
            disabled_models: HashSet::new(),
            poke_ids: poke_ids_from_env(),
            trace: std::env::var(TRACE_ENV_VAR).is_ok_and(|value| value == "1"),
            released: Vec::new(),
            released_with_cached_pose: Vec::new(),
            reported: HashSet::new(),
            cap_reported: false,
            region: ((0, 0), -1),
            reconcile_pending: false,
            scratch: Vec::new(),
            hooks: RagdollHooks::new(params),
            settings: RagdollSettings::default(),
            publish_authored: HashSet::new(),
        }
    }

    pub fn settings(&self) -> RagdollSettings {
        self.settings
    }

    /// Read by every step, so it applies to live and future ragdolls alike.
    pub fn set_settings(&mut self, settings: RagdollSettings) {
        self.settings = RagdollSettings::validated(settings, self.settings);
    }

    /// The contact hooks every physics step must run with.
    pub fn hooks(&self) -> &RagdollHooks {
        &self.hooks
    }

    pub fn contains(&self, entity: EntityId) -> bool {
        self.slots.contains_key(&entity)
    }

    pub fn owns_body(&self, handle: RigidBodyHandle) -> bool {
        self.live.values().any(|live| live.bodies.contains(&handle))
    }

    /// Reverts the ragdoll that owns `handle` to static collision for good; false when no live
    /// ragdoll owns the body.
    pub fn quarantine_body(&mut self, world: &mut PhysicsWorld, handle: RigidBodyHandle) -> bool {
        let Some(entity) = self
            .live
            .iter()
            .find_map(|(entity, live)| live.bodies.contains(&handle).then_some(*entity))
        else {
            return false;
        };
        self.pose_cache.remove(entity);
        self.revert_to_legacy(world, entity, true)
    }

    pub fn is_model_disabled(&self, mesh_path: &str) -> bool {
        !self.disabled_models.is_empty() && self.disabled_models.contains(&manifest_key(mesh_path))
    }

    /// True when the sim publishes a pose for the entity rather than leaving it
    /// at rest: an authored pose, or a remembered one it respawns at.
    pub fn has_pose(&self, entity: EntityId) -> bool {
        self.slots.get(&entity).is_some_and(|slot| {
            !slot.disabled
                && (slot.placement.authored.is_some() || self.pose_cache.get(entity).is_some())
        })
    }

    /// Logs a rejected placement once per reason.
    pub fn report_rejection(
        &mut self,
        rejection: &RagdollRejection,
        mesh_path: &str,
        ref_key: Option<&str>,
    ) {
        if matches!(rejection, RagdollRejection::ModelDisabled)
            || !self.reported.insert(rejection.label())
        {
            return;
        }
        tracing::warn!(
            "ragdoll: {mesh_path} ref={}: {rejection}; keeping static collision (logged once per reason)",
            ref_key.unwrap_or("?")
        );
    }

    /// Registers a placement. It shows its legacy colliders, posed as authored,
    /// unless the entity has a remembered pose: then it respawns at that pose.
    pub fn add_slot(&mut self, world: &mut PhysicsWorld, placement: RagdollPlacement) {
        let entity = placement.entity;
        let poke = form_id_of(&placement.ref_key).is_some_and(|id| self.poke_ids.contains(&id));
        self.by_cell.entry(placement.grid).or_default().push(entity);
        self.slots.insert(
            entity,
            RagdollSlot {
                placement,
                legacy: Vec::new(),
                disabled: false,
                poke,
            },
        );
        let remembered = self.pose_cache.get(entity).is_some();
        let respawned = remembered && self.has_capacity() && self.spawn(world, entity);
        if !respawned && let Some(slot) = self.slots.get_mut(&entity) {
            if slot.legacy.is_empty() {
                slot.legacy = insert_legacy(world, &slot.placement, !slot.disabled);
            }
            if !slot.disabled && !remembered && slot.placement.authored.is_some() {
                self.publish_authored.insert(entity);
            }
        }
        self.reconcile_pending = true;
    }

    /// Removes every ragdoll of the cell. True when colliders were removed.
    pub fn remove_cell(&mut self, world: &mut PhysicsWorld, grid: CellGrid) -> bool {
        let Some(entities) = self.by_cell.remove(&grid) else {
            return false;
        };
        for entity in entities {
            let Some(slot) = self.slots.remove(&entity) else {
                continue;
            };
            self.publish_authored.remove(&entity);
            if let Some(live) = self.live.remove(&entity) {
                if live.ever_moved {
                    self.remember_pose(world, entity, &live);
                }
                despawn_bodies(world, &live);
                self.reconcile_pending = true;
            }
            for handle in slot.legacy {
                world.remove_collider(handle);
            }
        }
        true
    }

    /// Wakes the ragdolls with a body at or near `removed`, the bounds of the
    /// static colliders that just went away: a sleeping body may have been
    /// resting on them.
    pub fn wake_near_bounds(&mut self, world: &mut PhysicsWorld, removed: &Aabb) {
        let area = removed.loosened(WAKE_MARGIN);
        for live in self.live.values() {
            if body_bounds_intersect(world, live, &area) {
                for &handle in &live.bodies {
                    world.wake_up(handle, true);
                }
            }
        }
    }

    /// The renderer cannot skin `mesh_path`: every placement of the model goes
    /// back to static collision and is never a ragdoll again. True when
    /// colliders changed.
    pub fn disable_model(&mut self, world: &mut PhysicsWorld, mesh_path: &str) -> bool {
        let key = manifest_key(mesh_path);
        if !self.disabled_models.insert(key.clone()) {
            return false;
        }
        let entities: Vec<EntityId> = self
            .slots
            .iter()
            .filter(|(_, slot)| manifest_key(&slot.placement.mesh_path) == key)
            .map(|(entity, _)| *entity)
            .collect();
        tracing::warn!(
            "ragdoll: model {mesh_path} cannot be skinned; {} placements revert to static collision for the session",
            entities.len()
        );
        let mut changed = false;
        for entity in entities {
            self.pose_cache.remove(entity);
            changed |= self.revert_to_legacy(world, entity, true);
        }
        changed
    }

    /// Publishes the entity's pose again on the next collection, when it is not
    /// at rest (moved, or authored); for a consumer that learned of the placement
    /// after the first publish.
    pub fn republish(&mut self, entity: EntityId) {
        let Some(slot) = self.slots.get(&entity).filter(|slot| !slot.disabled) else {
            return;
        };
        match self.live.get_mut(&entity) {
            Some(live) => live.publish |= live.ever_moved || slot.placement.authored.is_some(),
            None if slot.placement.authored.is_some() => {
                self.publish_authored.insert(entity);
            }
            None => {}
        }
    }

    /// Swaps ragdolls in and out of simulation to match the radius. True when
    /// colliders were inserted or removed. Costs nothing while neither the
    /// region nor any ragdoll's state changed.
    pub fn reconcile(&mut self, world: &mut PhysicsWorld, center: CellGrid, radius: i32) -> bool {
        if self.region != (center, radius) {
            self.region = (center, radius);
            self.reconcile_pending = true;
        }
        if !self.reconcile_pending {
            return false;
        }
        self.reconcile_pending = false;
        let mut changed = false;

        let mut entities = std::mem::take(&mut self.scratch);
        entities.clear();
        entities.extend(self.live.iter().filter_map(|(entity, live)| {
            let slot = self.slots.get(entity)?;
            let leaves = !live.ever_moved
                && live.phase == Phase::Active
                && !within_radius(slot.placement.grid, center, radius)
                && !awake_bodies(world, live);
            leaves.then_some(*entity)
        }));
        for entity in entities.drain(..) {
            changed |= self.revert_to_legacy(world, entity, false);
        }

        let mut more_pending = false;
        entities.extend(
            self.slots
                .iter()
                .filter(|(entity, slot)| {
                    !slot.disabled
                        && !self.live.contains_key(*entity)
                        && within_radius(slot.placement.grid, center, radius)
                })
                .map(|(entity, _)| *entity)
                .take(MAX_SPAWNS_PER_RECONCILE + 1),
        );
        if entities.len() > MAX_SPAWNS_PER_RECONCILE {
            entities.truncate(MAX_SPAWNS_PER_RECONCILE);
            more_pending = true;
        }
        for entity in entities.drain(..) {
            if !self.has_capacity() {
                more_pending = false;
                break;
            }
            changed |= self.spawn(world, entity);
        }
        self.scratch = entities;
        self.reconcile_pending |= more_pending;
        changed
    }

    /// The bones the water pass should run, from ragdolls that have finished
    /// folding. A folding ragdoll runs gravity-free until `finish_folding`
    /// restores gravity, so it joins the water only afterwards; the water pass
    /// then owns the gravity scale of a bone while it is in the water. Anchors
    /// and bones whose sidecar carries no water data stay out.
    pub fn water_targets(&mut self) -> impl Iterator<Item = WaterTarget<'_>> {
        self.live
            .iter_mut()
            .filter(|(_, live)| live.phase == Phase::Active)
            .flat_map(|(entity, live)| {
                let entity = *entity;
                let LiveRagdoll {
                    baked,
                    bodies,
                    water,
                    ..
                } = live;
                let baked: &Arc<BakedRagdoll> = baked;
                bodies
                    .iter()
                    .zip(&baked.spec.bodies)
                    .zip(water.iter_mut())
                    .filter(|((_, body), _)| !body.anchor)
                    .filter_map(move |((handle, body), state)| {
                        Some(WaterTarget {
                            entity,
                            handle: *handle,
                            props: Some(body.water.as_ref()?),
                            state,
                        })
                    })
            })
    }

    pub fn needs_step(&self, world: &PhysicsWorld) -> bool {
        self.live
            .values()
            .any(|live| live.phase != Phase::Active || awake_bodies(world, live))
    }

    /// Per-step bookkeeping of live ragdolls: the gravity-free fold after a
    /// spawn, wake/sleep transitions, first movement and velocity bounds.
    /// True when colliders were inserted or removed.
    pub fn after_step(&mut self, world: &mut PhysicsWorld) -> bool {
        let dt = world.integration_parameters.dt;
        let settings = self.settings;
        let mut broken: Vec<EntityId> = Vec::new();
        let mut fallen: Vec<EntityId> = Vec::new();
        let mut changed = false;
        for (&entity, live) in &mut self.live {
            if !bound_bodies(world, live) {
                broken.push(entity);
                continue;
            }
            let ref_key = self
                .slots
                .get(&entity)
                .map_or("?", |slot| slot.placement.ref_key.as_str());
            if let Phase::Folding { steps_left } = live.phase {
                let asleep = !awake_bodies(world, live);
                if !asleep && steps_left > 0 {
                    live.phase = Phase::Folding {
                        steps_left: steps_left - 1,
                    };
                    continue;
                }
                finish_folding(world, live, asleep);
                self.reconcile_pending = true;
            }
            let awake_now = awake_bodies(world, live);
            if awake_now != live.awake {
                live.awake = awake_now;
                self.reconcile_pending |= !awake_now;
                tracing::info!(
                    "ragdoll: {} ref={ref_key}",
                    if awake_now { "wake" } else { "sleep" }
                );
                live.publish |= live.ever_moved;
                if !awake_now {
                    live.rest.end(world);
                }
            }
            if awake_now {
                if free_falling_too_long(world, live) {
                    fallen.push(entity);
                    continue;
                }
                live.ever_moved |= has_moved(world, live);
                live.publish |= live.ever_moved;
                live.rest.step(world, &settings, dt);
                if self.trace {
                    live.trace_elapsed += dt;
                    let interval = if live.rest.is_settling() {
                        TRACE_SETTLE_INTERVAL
                    } else {
                        TRACE_INTERVAL
                    };
                    if live.trace_elapsed >= interval {
                        live.trace_elapsed = 0.0;
                        log_trace(world, live, ref_key);
                    }
                }
            } else {
                live.fall_check_elapsed = 0.0;
                live.airborne_seconds = 0.0;
            }
        }
        for entity in fallen {
            if let Some(slot) = self.slots.get(&entity) {
                tracing::warn!(
                    "ragdoll: {} fell for {MAX_FREE_FALL_SECONDS}s without touching anything; reverting to static collision",
                    slot.placement.ref_key
                );
            }
            self.pose_cache.remove(entity);
            changed |= self.revert_to_legacy(world, entity, false);
        }
        for entity in broken {
            if let Some(slot) = self.slots.get(&entity) {
                tracing::error!(
                    "ragdoll: error {}: non-finite body state; reverting to static collision",
                    slot.placement.ref_key
                );
            }
            self.pose_cache.remove(entity);
            changed |= self.revert_to_legacy(world, entity, true);
        }
        changed
    }

    /// Appends the deltas of every ragdoll whose pose changed since the last call.
    pub fn collect_poses(&mut self, world: &PhysicsWorld, out: &mut RagdollPoses) {
        out.entries.clear();
        out.body_deltas.clear();
        out.released.clear();
        out.released.append(&mut self.released);
        out.released_with_cached_pose.clear();
        out.released_with_cached_pose
            .append(&mut self.released_with_cached_pose);
        for (&entity, live) in &mut self.live {
            if !live.publish {
                continue;
            }
            live.publish = false;
            let first_body = out.body_deltas.len();
            let Some(poses) = body_poses(world, live) else {
                continue;
            };
            out.body_deltas
                .extend(poses.iter().zip(&live.rest_world).map(|(pose, rest)| {
                    let delta = *pose * rest.inverse();
                    BodyDelta {
                        translation: delta.translation,
                        rotation: delta.rotation,
                    }
                }));
            out.entries.push(RagdollEntry {
                entity,
                spec: Arc::clone(&live.baked.spec),
                first_body,
            });
        }
        for entity in self.publish_authored.drain() {
            let Some(slot) = self
                .slots
                .get(&entity)
                .filter(|slot| !slot.disabled && !self.live.contains_key(&entity))
            else {
                continue;
            };
            let Some(authored) = &slot.placement.authored else {
                continue;
            };
            let first_body = out.body_deltas.len();
            out.body_deltas
                .extend(authored.iter().zip(rest_world_poses(&slot.placement)).map(
                    |(pose, rest)| {
                        let delta = *pose * rest.inverse();
                        BodyDelta {
                            translation: delta.translation,
                            rotation: delta.rotation,
                        }
                    },
                ));
            out.entries.push(RagdollEntry {
                entity,
                spec: Arc::clone(&slot.placement.baked.spec),
                first_body,
            });
        }
    }

    fn has_capacity(&mut self) -> bool {
        let room = self.live.len() < MAX_SKINNED_RAGDOLLS;
        if !room && !std::mem::replace(&mut self.cap_reported, true) {
            tracing::warn!(
                "ragdoll: {MAX_SKINNED_RAGDOLLS} live ragdolls reached; further skeletons keep their static collision"
            );
        }
        room
    }

    /// Builds the live ragdoll of a slot, replacing its legacy colliders: at the
    /// remembered pose of the entity if it has one, else frozen in its authored
    /// (or rest) pose. The remembered pose is used up either way; a failed restore
    /// also tells the renderer to draw the static rows again.
    fn spawn(&mut self, world: &mut PhysicsWorld, entity: EntityId) -> bool {
        let restored = self.pose_cache.get(entity).cloned();
        let kind = restored
            .as_ref()
            .map_or(SpawnKind::Frozen, SpawnKind::Restore);
        let Some(slot) = self.slots.get_mut(&entity) else {
            return false;
        };
        let had_pose = restored.is_some();
        let outcome = build_ragdoll(world, &slot.placement, &kind, slot.poke);
        if had_pose {
            self.pose_cache.remove(entity);
        }
        match outcome {
            Ok(live) => {
                for handle in slot.legacy.drain(..) {
                    world.remove_collider(handle);
                }
                tracing::info!(
                    "ragdoll: spawn ref={} bodies={} joints={} ({})",
                    slot.placement.ref_key,
                    live.bodies.len(),
                    live.baked.spec.constraints.len(),
                    kind.label()
                );
                self.live.insert(entity, live);
                true
            }
            Err(error) => {
                tracing::error!(
                    "ragdoll: error {}: {error}; keeping static collision",
                    slot.placement.ref_key
                );
                slot.disabled = true;
                if had_pose || slot.placement.authored.is_some() {
                    self.released.push(entity);
                }
                for handle in slot.legacy.drain(..) {
                    world.remove_collider(handle);
                }
                slot.legacy = insert_legacy(world, &slot.placement, false);
                false
            }
        }
    }

    /// Despawns the live ragdoll (if any) and shows the legacy colliders again.
    /// `disable` also stops the slot from ever becoming a ragdoll, and drops its
    /// authored pose: a disabled ragdoll is drawn by its static rows, at rest.
    /// True when colliders were inserted or removed.
    fn revert_to_legacy(
        &mut self,
        world: &mut PhysicsWorld,
        entity: EntityId,
        disable: bool,
    ) -> bool {
        let Some(slot) = self.slots.get_mut(&entity) else {
            return false;
        };
        slot.disabled |= disable;
        let Some(live) = self.live.remove(&entity) else {
            if !(disable && slot.placement.authored.is_some()) {
                return false;
            }
            self.publish_authored.remove(&entity);
            self.released.push(entity);
            for handle in slot.legacy.drain(..) {
                world.remove_collider(handle);
            }
            slot.legacy = insert_legacy(world, &slot.placement, false);
            return true;
        };
        let posed = !slot.disabled && slot.placement.authored.is_some();
        if posed {
            self.publish_authored.insert(entity);
        } else if live.ever_moved || slot.placement.authored.is_some() {
            self.released.push(entity);
        }
        despawn_bodies(world, &live);
        slot.legacy = insert_legacy(world, &slot.placement, !slot.disabled);
        self.reconcile_pending = true;
        true
    }

    fn remember_pose(&mut self, world: &PhysicsWorld, entity: EntityId, live: &LiveRagdoll) {
        self.released.push(entity);
        if let Some(poses) = body_poses(world, live) {
            self.pose_cache.store(
                entity,
                CachedPose {
                    poses,
                    awake: awake_bodies(world, live),
                },
            );
            self.released_with_cached_pose.push(entity);
        }
    }
}

fn poke_ids_from_env() -> HashSet<u32> {
    let Ok(list) = std::env::var(POKE_ENV_VAR) else {
        return HashSet::new();
    };
    list.split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .filter_map(|token| match u32::from_str_radix(token, 16) {
            Ok(id) => Some(id),
            Err(error) => {
                tracing::warn!("{POKE_ENV_VAR}: {token:?} is not a hex form id ({error}); ignored");
                None
            }
        })
        .collect()
}

/// The hex form id before the plugin name of "001ED5:Fallout4.esm".
fn form_id_of(ref_key: &str) -> Option<u32> {
    u32::from_str_radix(ref_key.split_once(':')?.0, 16).ok()
}

/// The placement's static colliders. `posed` with an authored pose: every
/// body's hulls where the authored pose puts that body, so the collision matches
/// what is drawn; otherwise the model's own collision at rest.
fn insert_legacy(
    world: &mut PhysicsWorld,
    placement: &RagdollPlacement,
    posed: bool,
) -> Vec<ColliderHandle> {
    let mut insert = |cached: &CachedShape, pose: Pose| {
        let collider = ColliderBuilder::new(cached.shape.clone())
            .position(pose)
            .user_data(placement.tag.user_data(cached.material));
        world.insert_collider(collider, None)
    };
    match placement.authored.as_deref().filter(|_| posed) {
        Some(authored) => authored
            .iter()
            .zip(&placement.baked.body_shapes)
            .flat_map(|(body_pose, shapes)| {
                shapes
                    .iter()
                    .map(move |cached| (cached, *body_pose * cached.local_pose))
            })
            .map(|(cached, pose)| insert(cached, pose))
            .collect(),
        None => placement
            .legacy_shapes
            .iter()
            .map(|cached| insert(cached, placement.refr_pose * cached.local_pose))
            .collect(),
    }
}

/// World pose of every body of the placement at the model's rest pose.
fn rest_world_poses(placement: &RagdollPlacement) -> impl Iterator<Item = Pose> + '_ {
    placement
        .baked
        .spec
        .bodies
        .iter()
        .map(|body| placement.refr_pose * frame_pose(&body.rest, placement.scale))
}

fn frame_pose(frame: &RigidFrame, scale: f32) -> Pose {
    Pose::from_parts(frame.translation * scale, frame.rotation)
}

/// Rapier bodies use the entity id as `user_data`.
fn entity_key(entity: EntityId) -> u128 {
    u128::from(entity.0)
}

fn build_ragdoll(
    world: &mut PhysicsWorld,
    placement: &RagdollPlacement,
    kind: &SpawnKind,
    poke: bool,
) -> Result<LiveRagdoll, RagdollBuildError> {
    let baked = Arc::clone(&placement.baked);
    let spec = &baked.spec;
    let restored = match kind {
        SpawnKind::Restore(cached) => Some(*cached),
        SpawnKind::Frozen => None,
    };
    if let Some(cached) = restored.filter(|cached| cached.poses.len() != spec.bodies.len()) {
        return Err(RagdollBuildError::PoseCacheMismatch {
            found: cached.poses.len(),
            expected: spec.bodies.len(),
        });
    }
    let simulate = restored.is_some_and(|cached| cached.awake);
    if let Some(body) = spec
        .bodies
        .iter()
        .zip(&baked.body_shapes)
        .find(|(body, shapes)| !body.anchor && shapes.is_empty())
    {
        return Err(RagdollBuildError::BodyWithoutColliders(body.0.node.clone()));
    }

    let mut rest_bones = Vec::with_capacity(spec.bodies.len());
    let mut bodies = Vec::with_capacity(spec.bodies.len());
    let mut rest_world = Vec::with_capacity(spec.bodies.len());
    let mut spawn_world = Vec::with_capacity(spec.bodies.len());
    for (index, (body, shapes)) in spec.bodies.iter().zip(&baked.body_shapes).enumerate() {
        let rest = placement.refr_pose * frame_pose(&body.rest, placement.scale);
        let authored = placement
            .authored
            .as_deref()
            .map_or(rest, |authored| authored[index]);
        let spawn = restored
            .filter(|_| !body.anchor)
            .map_or(authored, |cached| cached.poses[index]);
        let (builder, hooks, mass) = if body.anchor {
            (RigidBodyBuilder::fixed(), ActiveHooks::empty(), None)
        } else {
            // A frozen respawn has no gravity until the body sleeps: joints are
            // inserted without waking (see below), so the whole ragdoll starts as
            // one awake island that rapier puts to sleep within a step.
            let builder = body_builder(
                RigidBodyType::Dynamic,
                body.linear_damping,
                body.angular_damping,
            )
            .gravity_scale(if simulate { 1.0 } else { 0.0 })
            .user_data(entity_key(placement.entity));
            (
                builder,
                ActiveHooks::FILTER_CONTACT_PAIRS | ActiveHooks::MODIFY_SOLVER_CONTACTS,
                Some(body.mass),
            )
        };
        let handle = world.insert_body(builder.pose(spawn));
        let density = density_for_mass(shapes, mass);
        for cached in shapes {
            let collider = ColliderBuilder::new(cached.shape.clone())
                .position(cached.local_pose)
                .density(density)
                .active_hooks(hooks)
                .user_data(placement.tag.user_data(cached.material));
            world.insert_collider(collider, Some(handle));
        }
        if !body.anchor {
            if let Some(rb) = world.bodies.get_mut(handle) {
                prepare_activation(rb, !simulate);
            }
            rest_bones.push(RestBone::new(
                handle,
                shapes,
                body.linear_damping,
                body.angular_damping,
            ));
        }
        bodies.push(handle);
        rest_world.push(rest);
        spawn_world.push(spawn);
    }
    apply_inertia_floor(world, &bodies, spec);
    let mut joints = Vec::with_capacity(spec.constraints.len());
    for constraint in &spec.constraints {
        let joint = joint_for(constraint, placement.scale);
        // `PhysicsWorld::insert_impulse_joint` wakes both bodies at the next
        // step, which would keep a frozen ragdoll simulating for two seconds.
        // The islands still merge (`to_join`), only the wake-up is skipped.
        joints.push(world.impulse_joints.insert(
            bodies[constraint.parent],
            bodies[constraint.child],
            joint,
            false,
        ));
    }
    let restored = restored.is_some();
    let water = std::iter::repeat_with(WaterBodyState::default)
        .take(bodies.len())
        .collect();
    Ok(LiveRagdoll {
        baked,
        bodies,
        joints,
        scale: placement.scale,
        trace_elapsed: 0.0,
        fall_check_elapsed: 0.0,
        airborne_seconds: 0.0,
        rest: RestTracker::new(rest_bones),
        rest_world,
        spawn_world,
        phase: if simulate {
            Phase::Active
        } else {
            Phase::Folding {
                steps_left: FOLD_STEPS,
            }
        },
        ever_moved: restored,
        awake: simulate,
        publish: restored || placement.authored.is_some(),
        poke_pending: poke,
        water,
    })
}

/// Tilt limits of a Havok ragdoll joint: `AngY` (twist axis toward frame Z) is
/// bounded by the cone alone; `AngZ` (twist axis toward frame Y) by the plane
/// range intersected with the cone. No cone leaves `AngY` free.
fn swing_limits(plane: [f32; 2], cone: Option<f32>) -> (Option<[f32; 2]>, [f32; 2]) {
    match cone {
        Some(cone) => (
            Some([-cone, cone]),
            [plane[0].clamp(-cone, cone), plane[1].clamp(-cone, cone)],
        ),
        None => (None, plane),
    }
}

fn joint_for(constraint: &RagdollConstraint, scale: f32) -> GenericJoint {
    let frame_parent = frame_pose(&constraint.frame_parent, scale);
    let frame_child = frame_pose(&constraint.frame_child, scale);
    let (mut joint, axes): (GenericJoint, &[JointAxis]) = match constraint.kind {
        JointKind::Ragdoll { twist, plane, cone } => {
            let (limit_y, limit_z) = swing_limits(widen_range(plane), cone.map(widen_cone));
            let mut builder = SphericalJointBuilder::new()
                .local_frame1(frame_parent)
                .local_frame2(frame_child)
                .contacts_enabled(false)
                .limits(JointAxis::AngX, widen_range(twist))
                .limits(JointAxis::AngZ, limit_z);
            if let Some(limit_y) = limit_y {
                builder = builder.limits(JointAxis::AngY, limit_y);
            }
            (
                builder.build().into(),
                &[JointAxis::AngX, JointAxis::AngY, JointAxis::AngZ],
            )
        }
        JointKind::LimitedHinge { range } => (
            GenericJointBuilder::new(JointAxesMask::LOCKED_REVOLUTE_AXES)
                .local_frame1(frame_parent)
                .local_frame2(frame_child)
                .contacts_enabled(false)
                .limits(JointAxis::AngX, widen_range(range))
                .build(),
            &[JointAxis::AngX],
        ),
    };
    soften_joint(&mut joint);
    let torque = constraint.friction * FRICTION_TORQUE_SCALE;
    if torque >= MIN_FRICTION_TORQUE {
        for &axis in axes {
            joint
                .set_motor_model(axis, MotorModel::ForceBased)
                .set_motor_velocity(axis, 0.0, FRICTION_MOTOR_DAMPING)
                .set_motor_max_force(axis, torque);
        }
    }
    joint
}

fn simulated_bodies(live: &LiveRagdoll) -> impl Iterator<Item = RigidBodyHandle> + '_ {
    live.bodies
        .iter()
        .zip(&live.baked.spec.bodies)
        .filter(|(_, body)| !body.anchor)
        .map(|(handle, _)| *handle)
}

fn finish_folding(world: &mut PhysicsWorld, live: &mut LiveRagdoll, asleep: bool) {
    live.phase = Phase::Active;
    for handle in simulated_bodies(live) {
        if let Some(rb) = world.bodies.get_mut(handle) {
            rb.set_gravity_scale(1.0, false);
        }
    }
    live.rest.close_gate(world);
    if !asleep {
        tracing::debug!(
            "ragdoll: did not fall asleep while folding; it simulates from its spawn pose"
        );
    }
    if std::mem::take(&mut live.poke_pending) {
        poke_root(world, live);
    }
}

fn poke_root(world: &mut PhysicsWorld, live: &LiveRagdoll) {
    let Some(root) = live.baked.spec.root_body() else {
        return;
    };
    if let Some(body) = world.bodies.get_mut(live.bodies[root]) {
        let impulse = POKE_DIRECTION.normalize() * POKE_SPEED * body.mass();
        body.apply_impulse(impulse, true);
    }
}

/// Whether any simulated body is awake; fixed anchors never sleep or wake.
fn awake_bodies(world: &PhysicsWorld, live: &LiveRagdoll) -> bool {
    live.bodies.iter().any(|handle| {
        world
            .bodies
            .get(*handle)
            .is_some_and(|body| !body.is_fixed() && !body.is_sleeping())
    })
}

fn body_bounds_intersect(world: &PhysicsWorld, live: &LiveRagdoll, area: &Aabb) -> bool {
    live.bodies
        .iter()
        .filter_map(|handle| world.bodies.get(*handle))
        .any(|body| {
            body.colliders()
                .iter()
                .filter_map(|collider| world.colliders.get(*collider))
                .any(|collider| collider.compute_aabb().intersects(area))
        })
}

fn touches_anything(world: &PhysicsWorld, live: &LiveRagdoll) -> bool {
    live.bodies
        .iter()
        .filter_map(|handle| world.bodies.get(*handle))
        .filter(|body| !body.is_fixed())
        .any(|body| {
            body.colliders().iter().any(|&collider| {
                world
                    .narrow_phase
                    .contact_pairs_with(collider)
                    .any(rapier3d::geometry::ContactPair::has_any_active_contact)
            })
        })
}

/// Counts awake time without any contact; true once the ragdoll has fallen for
/// `MAX_FREE_FALL_SECONDS`. Contacts are only polled every `FALL_CHECK_INTERVAL`.
fn free_falling_too_long(world: &PhysicsWorld, live: &mut LiveRagdoll) -> bool {
    live.fall_check_elapsed += world.integration_parameters.dt;
    if live.fall_check_elapsed < FALL_CHECK_INTERVAL {
        return false;
    }
    live.fall_check_elapsed = 0.0;
    if touches_anything(world, live) {
        live.airborne_seconds = 0.0;
    } else {
        live.airborne_seconds += FALL_CHECK_INTERVAL;
    }
    live.airborne_seconds >= MAX_FREE_FALL_SECONDS
}

fn has_moved(world: &PhysicsWorld, live: &LiveRagdoll) -> bool {
    live.bodies
        .iter()
        .zip(&live.spawn_world)
        .any(|(handle, spawn)| {
            world.bodies.get(*handle).is_some_and(|body| {
                let pose = body.position();
                !body.is_fixed()
                    && (pose.translation.distance(spawn.translation) > MOVED_TRANSLATION
                        || 1.0 - pose.rotation.dot(spawn.rotation).abs() > MOVED_ROTATION)
            })
        })
}

fn body_poses(world: &PhysicsWorld, live: &LiveRagdoll) -> Option<BodyPoses> {
    live.bodies
        .iter()
        .map(|handle| world.bodies.get(*handle).map(|body| *body.position()))
        .collect()
}

/// Clamps runaway velocities; false when a body went non-finite. In-bounds
/// bodies (the common case) are only read.
fn bound_bodies(world: &mut PhysicsWorld, live: &LiveRagdoll) -> bool {
    for &handle in &live.bodies {
        let Some(body) = world.bodies.get(handle).filter(|body| !body.is_fixed()) else {
            continue;
        };
        if state_is_bounded(body) {
            continue;
        }
        if !body_finite(body.position(), body) {
            return false;
        }
        if let Some(body) = world.bodies.get_mut(handle) {
            let pose = *body.position();
            sanitize_body(body, pose);
        }
    }
    true
}

/// Active contacts of a ragdoll's bones with anything but its own bones.
#[derive(Default)]
struct ContactSummary {
    static_pairs: usize,
    dynamic_pairs: usize,
    embedded_manifolds: usize,
    /// Penetration depth, bone index and partner REFR form id of the deepest contact.
    deepest: Option<(f32, usize, u32)>,
}

fn contact_summary(world: &PhysicsWorld, live: &LiveRagdoll) -> ContactSummary {
    let mut summary = ContactSummary::default();
    for (index, handle) in live.bodies.iter().enumerate() {
        let Some(body) = world.bodies.get(*handle).filter(|body| !body.is_fixed()) else {
            continue;
        };
        for &collider in body.colliders() {
            for pair in world
                .narrow_phase
                .contact_pairs_with(collider)
                .filter(|pair| pair.has_any_active_contact())
            {
                let partner = if pair.collider1 == collider {
                    pair.collider2
                } else {
                    pair.collider1
                };
                let Some(partner) = world.colliders.get(partner) else {
                    continue;
                };
                let partner_is_static = partner
                    .parent()
                    .and_then(|parent| world.bodies.get(parent))
                    .is_none_or(rapier3d::dynamics::RigidBody::is_fixed);
                if partner_is_static {
                    summary.static_pairs += 1;
                } else {
                    summary.dynamic_pairs += 1;
                }
                let active = pair
                    .manifolds
                    .iter()
                    .filter(|manifold| !manifold.data.solver_contacts.is_empty());
                for manifold in active {
                    summary.embedded_manifolds +=
                        usize::from(manifold.data.user_data == MANIFOLD_EMBEDDED);
                    let depth = manifold
                        .points
                        .iter()
                        .map(|point| -point.dist)
                        .fold(0.0_f32, f32::max);
                    if summary
                        .deepest
                        .is_none_or(|(deepest, _, _)| depth > deepest)
                    {
                        summary.deepest = Some((depth, index, unpack_ref(partner.user_data).0));
                    }
                }
            }
        }
    }
    summary
}

/// One `OPENFO4_RAGDOLL_TRACE` line: what keeps an awake ragdoll from sleeping.
fn log_trace(world: &PhysicsWorld, live: &LiveRagdoll, ref_key: &str) {
    let spec = &live.baked.spec;
    let mut fastest_linear = (0.0_f32, 0);
    let mut fastest_angular = (0.0_f32, 0);
    for (index, handle) in live.bodies.iter().enumerate() {
        let Some(body) = world.bodies.get(*handle).filter(|body| !body.is_fixed()) else {
            continue;
        };
        let linear = body.linvel().length();
        let angular = body.angvel().length();
        if linear > fastest_linear.0 {
            fastest_linear = (linear, index);
        }
        if angular > fastest_angular.0 {
            fastest_angular = (angular, index);
        }
    }
    let contacts = contact_summary(world, live);
    let deepest_contact = contacts.deepest.map_or_else(
        || "none".to_owned(),
        |(depth, bone, partner)| {
            format!(
                "{depth:.3}u ({} vs ref {partner:06X})",
                spec.bodies[bone].node
            )
        },
    );
    let joint_limit_active = |joint: &GenericJoint| {
        [JointAxis::AngX, JointAxis::AngY, JointAxis::AngZ]
            .iter()
            .any(|&axis| joint.limits[axis as usize].impulse != 0.0)
    };
    let joints_at_limit = live
        .joints
        .iter()
        .filter_map(|handle| world.impulse_joints.get(*handle))
        .filter(|joint| joint_limit_active(&joint.data))
        .count();
    let anchor_error = spec
        .constraints
        .iter()
        .filter_map(|constraint| {
            let parent = world.bodies.get(live.bodies[constraint.parent])?.position()
                * frame_pose(&constraint.frame_parent, live.scale);
            let child = world.bodies.get(live.bodies[constraint.child])?.position()
                * frame_pose(&constraint.frame_child, live.scale);
            Some(parent.translation.distance(child.translation))
        })
        .fold(0.0_f32, f32::max);
    tracing::info!(
        "ragdoll: trace ref={ref_key} phase={} still={:.2}s window_range={:.4}u gate_open={} max_ke_per_mass={:.4} max_linear={:.2}u/s ({}) max_angular={:.2}rad/s ({}) contacts=static:{} dynamic:{} embedded_manifolds={} max_penetration={deepest_contact} joints_at_limit={}/{} max_anchor_error={anchor_error:.3}u",
        live.phase.label(),
        live.rest.still_seconds(),
        live.rest.range(),
        live.rest.gate_open(),
        live.rest.kinetic_energy(),
        fastest_linear.0,
        spec.bodies[fastest_linear.1].node,
        fastest_angular.0,
        spec.bodies[fastest_angular.1].node,
        contacts.static_pairs,
        contacts.dynamic_pairs,
        contacts.embedded_manifolds,
        joints_at_limit,
        live.joints.len(),
    );
}

fn despawn_bodies(world: &mut PhysicsWorld, live: &LiveRagdoll) {
    for &handle in &live.bodies {
        world.remove_body(handle);
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use super::*;

    fn frame() -> RigidFrame {
        RigidFrame {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
        }
    }

    fn constraint(kind: JointKind, friction: f32) -> RagdollConstraint {
        RagdollConstraint {
            child: 1,
            parent: 0,
            frame_child: frame(),
            frame_parent: frame(),
            kind,
            friction,
        }
    }

    #[test]
    fn the_cone_bounds_the_tilt_toward_z_and_clips_the_plane_range() {
        let (toward_z, toward_y) = swing_limits([-0.8, 0.3], Some(0.5));
        assert_eq!(toward_z, Some([-0.5, 0.5]));
        assert_eq!(toward_y, [-0.5, 0.3]);
    }

    #[test]
    fn without_a_cone_the_z_tilt_is_free_and_the_plane_range_is_kept() {
        let (toward_z, toward_y) = swing_limits([-0.8, 0.3], None);
        assert_eq!(toward_z, None);
        assert_eq!(toward_y, [-0.8, 0.3]);
    }

    #[test]
    fn form_id_is_the_hex_part_before_the_plugin_name() {
        assert_eq!(form_id_of("001ED5:Fallout4.esm"), Some(0x1ED5));
        assert_eq!(form_id_of("FF000A:DLCRobot.esm"), Some(0xFF_000A));
    }

    #[test]
    fn form_id_is_none_for_keys_without_a_hex_prefix() {
        assert_eq!(form_id_of("no-colon"), None);
        assert_eq!(form_id_of("ZZZZ:Fallout4.esm"), None);
        assert_eq!(form_id_of(":Fallout4.esm"), None);
    }

    #[test]
    fn joint_friction_becomes_a_velocity_zero_motor_scaled_to_engine_units() {
        let ragdoll = JointKind::Ragdoll {
            twist: [-0.5, 0.5],
            plane: [-0.5, 0.5],
            cone: Some(0.8),
        };
        let joint = joint_for(&constraint(ragdoll, 10.0), 1.0);
        for axis in [JointAxis::AngX, JointAxis::AngY, JointAxis::AngZ] {
            let motor = &joint.motors[axis as usize];
            assert_eq!(motor.max_force, 10.0 * FRICTION_TORQUE_SCALE, "{axis:?}");
            assert_eq!(motor.target_vel, 0.0);
        }
    }

    #[test]
    fn friction_below_the_threshold_installs_no_motor() {
        let hinge = JointKind::LimitedHinge { range: [-1.0, 1.0] };
        let joint = joint_for(&constraint(hinge, 0.01), 1.0);
        assert_ne!(
            joint.motors[JointAxis::AngX as usize].max_force,
            0.01 * FRICTION_TORQUE_SCALE
        );
        assert_ne!(joint.motors[JointAxis::AngX as usize].max_force, 0.0);
    }

    #[test]
    fn a_hinge_locks_every_axis_but_its_own_rotation_and_widens_a_narrow_range() {
        let hinge = JointKind::LimitedHinge { range: [0.0, 0.01] };
        let joint = joint_for(&constraint(hinge, 0.0), 1.0);
        assert_eq!(joint.locked_axes, JointAxesMask::LOCKED_REVOLUTE_AXES);
        let limits = joint.limits[JointAxis::AngX as usize];
        assert!((limits.max - limits.min - 2.0 * 5.0_f32.to_radians()).abs() < 1e-5);
    }

    #[test]
    fn a_ragdoll_joint_without_a_cone_leaves_the_y_swing_unlimited() {
        let free = JointKind::Ragdoll {
            twist: [-0.5, 0.5],
            plane: [-0.5, 0.5],
            cone: None,
        };
        let coned = JointKind::Ragdoll {
            twist: [-0.5, 0.5],
            plane: [-0.5, 0.5],
            cone: Some(0.8),
        };
        let y = JointAxis::AngY as usize;
        let free_range = joint_for(&constraint(free, 0.0), 1.0).limits[y];
        let coned_range = joint_for(&constraint(coned, 0.0), 1.0).limits[y];
        assert!(free_range.max - free_range.min > coned_range.max - coned_range.min);
        assert!((coned_range.max - 0.8).abs() < 1e-6);
    }

    #[test]
    fn joint_frames_are_scaled_with_the_placement() {
        let mut moved = constraint(JointKind::LimitedHinge { range: [-1.0, 1.0] }, 0.0);
        moved.frame_parent.translation = Vec3::new(0.0, 1.5, 0.0);
        let joint = joint_for(&moved, 2.0);
        assert_eq!(joint.local_frame1.translation, Vec3::new(0.0, 3.0, 0.0));
    }
}
