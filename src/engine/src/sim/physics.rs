//! All rapier state lives here. Nothing outside `sim` may touch these types.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use glam::Vec3;
use rapier3d::prelude::*;

use super::bake::{BakedCellColliders, CachedShape, RagdollBake, ShapeKey, build_terrain_shape};
use super::collider_user_data::{
    ColliderTag, pack_grid, pack_path, pack_ref, unpack_path, unpack_ref,
};
use super::collision::{CellColliderBatch, ObjectColliderGroup, Placement, TerrainCollision};
use super::commands::KinematicPose;
use super::dynamic_body::{
    DYN_ANGULAR_DAMPING, DYN_LINEAR_DAMPING, ENGINE_UNITS_PER_METER, GRAVITY_DYN, body_builder,
    clamp_linvel, density_for_mass, sanitize_body, state_is_bounded, within_radius,
};
use super::kinematic::KinematicBodies;
use super::mirror::{KinematicMirror, scene_query_filter};
use super::ragdoll::{RagdollManager, RagdollPlacement, RagdollPoses, RagdollRejection};
use super::ragdoll_pose::authored_body_poses;
use super::ragdoll_tuning::RagdollSettings;
use super::sound_pose::{pose_is_sound, vec_is_sound};
use super::water_materials::{MaterialBuoyancy, WaterBodyProps};
use super::water_physics::{WaterBodyState, WaterPhysics, WaterQuery, WaterTarget};
use super::{CellGrid, DynamicPose, EntityId, FIXED_DT, RayHit, refr_entity_id};
use crate::footsteps::events::SurfaceMaterial;
use crate::utils::warn_once;

/// Max speed a player shove may leave on a pushed body. A hard impulse onto a
/// light prop can impart enough velocity that, in a single 1/60 s step, the
/// prop travels farther than soft-CCD's prediction window — punching it clean
/// through a wall before any contact is generated. Capping the push at the
/// source keeps per-step motion (`PUSH_MAX_LINVEL * FIXED_DT` ≈ 2.5 u) inside
/// `DYN_SOFT_CCD_PREDICTION`, so soft-CCD reliably catches the prop instead.
const PUSH_MAX_LINVEL: f32 = 150.0;
/// Fixed steps the world keeps stepping after the player mirror last moved
/// next to dynamic bodies: one for the kinematic body to reach its target, one
/// for the narrow phase to see the contact start there and wake the island.
const MIRROR_SETTLE_STEPS: u8 = 2;
/// How far around the mirror a dynamic body counts as "near" (engine units):
/// above the player's per-tick travel at sprint speed.
const MIRROR_NEAR_RANGE: f32 = 2.0;

/// How clutter entering the dynamic radius is activated
/// (`OPENFO4_PHYSICS_ACTIVATION`).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum ActivationMode {
    /// Flip to `Dynamic` and wake: every new body is simulated until it
    /// settles on its own (about 2 s), so each cell crossing runs full world
    /// steps over the freshly activated clutter.
    Wake,
    /// Flip to `Dynamic` but put the body straight to sleep on its first
    /// activation (vanilla Havok keeps placed clutter inactive until
    /// disturbed). A contact with an awake body or a player push wakes it
    /// through rapier's islands, so resting clutter costs no solver time.
    Sleep,
}

impl ActivationMode {
    const ENV_VAR: &'static str = "OPENFO4_PHYSICS_ACTIVATION";

    fn from_env() -> Self {
        match std::env::var(Self::ENV_VAR).as_deref() {
            Ok("wake") => Self::Wake,
            Ok("sleep") | Err(std::env::VarError::NotPresent) => Self::Sleep,
            Ok(other) => {
                tracing::warn!("{}={other:?} is not wake|sleep; using sleep", Self::ENV_VAR);
                Self::Sleep
            }
            Err(error) => {
                tracing::warn!("{} unreadable ({error}); using sleep", Self::ENV_VAR);
                Self::Sleep
            }
        }
    }
}

struct CacheEntry {
    /// Arc-shared with the worker's `ShapeBakeCache` (and any in-flight baked
    /// payloads): a shape resident in both caches costs its memory once.
    shapes: Arc<Vec<CachedShape>>,
    /// One count per placement using this entry; evicted at zero.
    refcount: usize,
}

/// A placed reference promoted to a rapier rigid body. Spawned `Fixed` by
/// `add_cell`, flipped `Dynamic`↔`Fixed` by the radius reconcile, despawned by
/// `remove_cell`. `prev/cur_pose` feed render interpolation.
struct DynBody {
    handle: RigidBodyHandle,
    /// Spawn cell — the radius test keys off this (not the live position), so a
    /// body that drifts a little stays owned by the cell that created it.
    grid: CellGrid,
    prev_pose: Pose,
    cur_pose: Pose,
    /// True while `Dynamic` (inside the radius); false while `Fixed` (frozen).
    active: bool,
    /// Whether the body has been `Dynamic` before. Only the first activation
    /// may force sleep: a body that already joined a rapier island must not be
    /// slept by hand (the island would stay awake around a sleeping body).
    ever_active: bool,
    /// Collision layer and materials from the collision manifest; `None` for a
    /// manifest written before water data existed.
    water_props: Option<Arc<WaterBodyProps>>,
    water: WaterBodyState,
    /// Built from the parts of an assembled weapon: further parts of the same
    /// REFR attach their colliders here instead of counting as a duplicate.
    compound: bool,
}

/// What a scene ray hit, before it is shaped for a caller.
struct RayProbe {
    mesh_path: String,
    material: u32,
    ref_id: Option<String>,
    dist: f32,
    terrain: bool,
}

/// Interns (mesh path, material) pairs into the 32-bit path slot of the
/// collider `user_data`. Slot 0 is reserved for terrain, so slots start at 1.
/// A mesh has one slot per distinct material it carries; shapes without one
/// use material 0.
#[derive(Default)]
struct PathRegistry {
    entries: Vec<(Arc<str>, u32)>,
    slots: HashMap<Arc<str>, Vec<(u32, u32)>>,
}

impl PathRegistry {
    fn intern(&mut self, path: &str, material: u32) -> u32 {
        if let Some(&(_, slot)) = self
            .slots
            .get(path)
            .and_then(|by_material| by_material.iter().find(|(known, _)| *known == material))
        {
            return slot;
        }
        let path: Arc<str> = Arc::from(path);
        self.entries.push((path.clone(), material));
        let slot = u32::try_from(self.entries.len()).expect("path slot space exhausted");
        self.slots.entry(path).or_default().push((material, slot));
        slot
    }

    fn get(&self, slot: u32) -> Option<(&str, u32)> {
        let index = usize::try_from(slot).ok()?.checked_sub(1)?;
        self.entries
            .get(index)
            .map(|(path, material)| (&**path, *material))
    }
}

/// The registry slots of one object group's materials, interned the first time a shape of the
/// group carries the material rather than once per placement.
struct GroupSlots<'a> {
    path: &'a str,
    by_material: HashMap<u32, u32>,
}

impl<'a> GroupSlots<'a> {
    fn new(path: &'a str) -> Self {
        Self {
            path,
            by_material: HashMap::new(),
        }
    }

    fn slot(&mut self, paths: &mut PathRegistry, material: u32) -> u32 {
        *self
            .by_material
            .entry(material)
            .or_insert_with(|| paths.intern(self.path, material))
    }

    /// The tag whose colliders carry the slot of their shape's material, for shapes with these
    /// `materials`.
    fn tag(
        &mut self,
        paths: &mut PathRegistry,
        base_user_data: u128,
        materials: impl IntoIterator<Item = u32>,
    ) -> ColliderTag {
        ColliderTag::new(base_user_data, materials, |material| {
            self.slot(paths, material)
        })
    }
}

pub(crate) struct PhysicsContext {
    pub world: PhysicsWorld,
    cell_colliders: HashMap<CellGrid, Vec<ColliderHandle>>,
    /// Cells whose collision batch is installed with its ground usable: the terrain trimesh is
    /// registered, or the cell has no terrain at all and stands on its objects. The player only
    /// moves inside ready cells, so it can never fall through unloaded world.
    ground_ready: HashSet<CellGrid>,
    /// The terrain colliders; any other collider is attributed through the path registry.
    terrain_colliders: HashSet<ColliderHandle>,
    shape_cache: HashMap<ShapeKey, CacheEntry>,
    /// Cache keys each cell holds a refcount on (one entry per placement).
    cell_cache_keys: HashMap<CellGrid, Vec<ShapeKey>>,
    /// (mesh path, material) intern table for hit attribution: collider
    /// `user_data` packs the slot into bits 64..96 (0 = terrain). Pairs are few
    /// and recurring, so this never shrinks.
    paths: PathRegistry,
    /// Plugin-name intern table for REFR hit attribution: `user_data` packs
    /// `index + 1` into bits 120..128 (0 = no ref). A handful of plugins
    /// exist game-wide, so this never shrinks either.
    plugin_registry: Vec<Arc<str>>,
    plugin_index: HashMap<Arc<str>, u32>,
    /// Interior view active: the world holds one interior cell (object colliders
    /// only, no LAND terrain). The player then grounds against object colliders
    /// and the per-grid ground-ready gate is bypassed (see `spawn_ready` and
    /// `step_player`). Toggled via `SimCommand::SetInteriorMode`.
    interior_mode: bool,
    /// Colliders changed since the last step. With a static-only world the
    /// pipeline step exists solely to fold insertions/removals into the
    /// broad-phase BVH, so clean ticks skip it entirely.
    dirty: bool,
    /// Bumped whenever a step folds collider changes into the broad phase —
    /// i.e. exactly when AABB queries start seeing the new collider set.
    /// Consumers (debug wireframe) rebuild when this moves.
    change_counter: u64,
    pub last_step_micros: u64,
    /// Dynamic rigid bodies keyed by REFR entity id (the same id the renderer
    /// derives, so poses map back to instances across the sim/render split).
    /// Built for `dynamic` collider groups (Havok-movable clutter); everything
    /// else stays a parentless static collider.
    dyn_bodies: HashMap<EntityId, DynBody>,
    /// Which dynamic bodies each cell owns, so `remove_cell` despawns them —
    /// every teardown path (unload, LOD demotion, worldspace switch, interior
    /// entry) routes through `RemoveCellColliders`, so this is the single
    /// despawn site.
    cell_dyn: HashMap<CellGrid, Vec<EntityId>>,
    /// Center + radius (Chebyshev cells) of the dynamic-simulation region,
    /// driven by `SimCommand::SetPhysicsCenter`. Bodies inside it are `Dynamic`.
    dyn_center: CellGrid,
    dyn_radius: i32,
    activation: ActivationMode,
    /// Kinematic stand-in for the player capsule, created on first sync. Lets
    /// rapier's contact-start wake handle sleeping clutter the player touches.
    player_mirror: Option<KinematicMirror>,
    /// Steps still owed so the mirror's latest move is simulated (see
    /// `MIRROR_SETTLE_STEPS`); stays 0 while the player is idle or far from
    /// any dynamic body, so the idle-step gate keeps its perf guarantee.
    mirror_settle_steps: u8,
    /// Kinematic bodies of animated parts, commanded by the renderer.
    kinematic: KinematicBodies,
    /// Skeleton props that become ragdolls inside the simulation radius.
    ragdolls: RagdollManager,
    /// FO4 water physics for clutter; `None` (the default) costs nothing.
    water: WaterPhysics,
}

impl PhysicsContext {
    pub fn new() -> Self {
        let mut world = PhysicsWorld::new();
        world.integration_parameters.dt = FIXED_DT;
        // Tell rapier the world is 7× metre-scale so its tolerances (contact
        // margin, sleep thresholds, CCD) are calibrated for this engine's units
        // rather than the metre-scale default — otherwise clutter jitters and
        // sleeps poorly.
        world.integration_parameters.length_unit = ENGINE_UNITS_PER_METER;
        // Dynamic clutter falls at the player's engine-unit gravity, not
        // rapier's metric default (the kinematic player still integrates its
        // own). Acts only on rigid bodies; static colliders are inert.
        world.gravity = Vec3::new(0.0, -GRAVITY_DYN, 0.0);
        // The default 4 solver iterations leave residual constraint error in
        // clutter piles and across large mass ratios that the next step
        // amplifies into jitter; rapier recommends 8-12 for stacks of objects.
        // The extra stabilization pass resolves penetration by position
        // correction rather than corrective-velocity ejection. The cost is paid
        // only while bodies are awake (radius- and sleep-gated by `needs_step`).
        world.integration_parameters.num_solver_iterations = 8;
        world
            .integration_parameters
            .num_internal_stabilization_iterations = 2;
        let ragdolls = RagdollManager::new(&world.integration_parameters);
        let activation = ActivationMode::from_env();
        tracing::info!("physics: clutter activation mode {activation:?}");
        Self {
            world,
            cell_colliders: HashMap::new(),
            ground_ready: HashSet::new(),
            terrain_colliders: HashSet::new(),
            shape_cache: HashMap::new(),
            cell_cache_keys: HashMap::new(),
            paths: PathRegistry::default(),
            plugin_registry: Vec::new(),
            plugin_index: HashMap::new(),
            interior_mode: false,
            dirty: false,
            change_counter: 0,
            last_step_micros: 0,
            dyn_bodies: HashMap::new(),
            cell_dyn: HashMap::new(),
            dyn_center: (0, 0),
            dyn_radius: 2,
            activation,
            player_mirror: None,
            mirror_settle_steps: 0,
            kinematic: KinematicBodies::default(),
            ragdolls,
            water: WaterPhysics::new(),
        }
    }

    /// Pack a REFR form key ("001ED5:Fallout4.esm") for hit attribution:
    /// 24-bit hex id + interned plugin slot. 0 (= no attribution) for keys
    /// that don't fit the scheme.
    fn pack_ref_key(&mut self, ref_key: Option<&str>) -> u128 {
        let Some((hex, plugin)) = ref_key.and_then(|k| k.split_once(':')) else {
            return 0;
        };
        let Ok(form_id) = u32::from_str_radix(hex, 16) else {
            return 0;
        };
        if form_id > 0xFF_FFFF {
            return 0;
        }
        let slot = if let Some(&slot) = self.plugin_index.get(plugin) {
            slot
        } else {
            let slot = self.plugin_registry.len() as u32 + 1; // index + 1
            if slot > 0xFF {
                return 0; // out of plugin slots — drop attribution
            }
            let arc: Arc<str> = Arc::from(plugin);
            self.plugin_registry.push(arc.clone());
            self.plugin_index.insert(arc, slot);
            slot
        };
        pack_ref(form_id, slot)
    }

    /// What the ray from `origin` along `dir` hits first: mesh path (or
    /// "terrain"), REFR form key when known, and distance.
    pub fn ray_attribution(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<RayHit> {
        let probe = self.probe(origin, dir, max_dist)?;
        Some(RayHit {
            mesh_path: probe.mesh_path,
            ref_id: probe.ref_id,
            dist: probe.dist,
        })
    }

    /// Surface under `origin` along a straight-down ray. Dynamic bodies are
    /// hit like static colliders (a crate under the foot reports its own
    /// material); the player's mirror capsule is excluded by the scene filter.
    pub(crate) fn ground_surface(&self, origin: Vec3, max_dist: f32) -> SurfaceMaterial {
        let Some(probe) = self.probe(origin, Vec3::NEG_Y, max_dist) else {
            return SurfaceMaterial::None;
        };
        if probe.terrain {
            let point = origin + Vec3::NEG_Y * probe.dist;
            return SurfaceMaterial::Terrain {
                point: point.to_array(),
            };
        }
        match probe.material {
            0 => SurfaceMaterial::None,
            hash => SurfaceMaterial::Hash(hash),
        }
    }

    fn probe(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<RayProbe> {
        let ray = Ray::new(origin, dir);
        let (handle, dist) = self
            .world
            .cast_ray(&ray, max_dist, true, scene_query_filter())?;
        let collider = self.world.colliders.get(handle)?;
        let terrain = self.terrain_colliders.contains(&handle);
        let (mesh_path, material) = if terrain {
            ("terrain".to_string(), 0)
        } else {
            let slot = unpack_path(collider.user_data);
            self.paths.get(slot).map_or_else(
                || {
                    warn_once(&format!("physics-slot:{slot}"), || {
                        format!(
                            "physics: a collider is neither terrain nor attributed to a mesh path \
                             (slot {slot}); it reports no material"
                        )
                    });
                    ("?".to_string(), 0)
                },
                |(path, material)| (path.to_string(), material),
            )
        };
        let (form_id, plugin_slot) = unpack_ref(collider.user_data);
        let ref_id = (plugin_slot != 0)
            .then(|| self.plugin_registry.get(plugin_slot as usize - 1))
            .flatten()
            .map(|plugin| format!("{form_id:06X}:{plugin}"));
        Some(RayProbe {
            mesh_path,
            material,
            ref_id,
            dist,
            terrain,
        })
    }

    /// Whether a fixed tick must step the pipeline. Steps on a pending collider
    /// change (the dirty-flag fold), while the player mirror's latest move is
    /// still owed a step, or while any in-radius body is awake.
    /// Settled clutter sleeps (rapier islands), so the world goes idle and stops
    /// re-stepping the whole collision ring every tick — restoring the
    /// dirty-flag's perf guarantee. A push impulse or a fresh radius activation
    /// wakes bodies and resumes stepping.
    pub fn needs_step(&self) -> bool {
        self.dirty
            || self.mirror_settle_steps > 0
            || self.kinematic.needs_step()
            || self.ragdolls.needs_step(&self.world)
            || self.dyn_bodies.values().any(|db| {
                db.active
                    && self
                        .world
                        .bodies
                        .get(db.handle)
                        .is_some_and(|rb| !rb.is_sleeping())
            })
    }

    /// Installs (or removes) the water source for clutter bodies. Replacing
    /// the source keeps which bodies are already in the water.
    pub fn set_water_query(&mut self, query: Option<WaterQuery>) {
        self.water.set_query(query);
    }

    /// Installs the MATT buoyancy table the bodies' materials resolve against.
    /// Install it before the first cell is added.
    pub fn set_material_buoyancy_table(&mut self, table: Arc<MaterialBuoyancy>) {
        self.water.set_materials(table);
    }

    /// Fixes the mean MATT `BNAM` buoyancy of a clutter body, overriding its
    /// materials; a no-op for an entity that has no body.
    pub fn set_material_buoyancy(&mut self, entity: EntityId, buoyancy: f32) {
        if let Some(body) = self.dyn_bodies.get_mut(&entity) {
            body.water.set_material_buoyancy(buoyancy);
        }
    }

    /// Moves the bodies' water entries since the last call into `out`.
    pub fn drain_water_entries(&mut self, out: &mut Vec<Vec3>) {
        self.water.drain_entries(out);
    }

    /// Runs the water pass for the fixed tick about to be stepped. Separate
    /// from [`step`](Self::step) so a sleeping body resting in water is woken
    /// even while the world is otherwise idle; dry scenes (no water source, or
    /// every body above the surface) only read bodies and never wake them.
    pub fn update_water(&mut self, lod_center: Vec3) {
        let clutter = self
            .dyn_bodies
            .iter_mut()
            .filter(|(_, db)| db.active)
            .map(|(entity, db)| WaterTarget {
                entity: *entity,
                handle: db.handle,
                props: db.water_props.as_deref(),
                state: &mut db.water,
            });
        let bones = self.ragdolls.water_targets();
        self.water
            .update(&mut self.world, clutter.chain(bones), lod_center, FIXED_DT);
    }

    pub fn step(&mut self) {
        let _span = tracing::info_span!("sim_physics_step", dirty = self.dirty).entered();
        let t = Instant::now();
        // Snapshot the previous pose of each active body before integrating, so
        // the render frame can interpolate prev->cur (render outruns the tick).
        for db in self.dyn_bodies.values_mut() {
            if db.active {
                db.prev_pose = db.cur_pose;
            }
        }
        self.audit_before_step();
        {
            let _span = tracing::info_span!("sim_world_step").entered();
            self.world.step_with_events(self.ragdolls.hooks(), &());
        }
        self.wake_bodies_touching_mirror();
        let _sync_span = tracing::info_span!("sim_body_sync").entered();
        for db in self.dyn_bodies.values_mut() {
            if !db.active {
                continue;
            }
            // Read-only fast path: a finite, in-bounds body (the common case)
            // must NOT go through `get_mut` — that flags every active body
            // `modified` each tick and defeats the idle-step gate. Only a body
            // that genuinely needs clamping or recovery is taken mutably.
            let (pos, ok) = match self.world.bodies.get(db.handle) {
                Some(rb) => (*rb.position(), state_is_bounded(rb)),
                None => continue,
            };
            if ok {
                db.cur_pose = pos;
                continue;
            }
            if let Some(rb) = self.world.bodies.get_mut(db.handle) {
                db.cur_pose = sanitize_body(rb, db.cur_pose);
            }
        }
        let ragdoll_changed_colliders = self.ragdolls.after_step(&mut self.world);
        if self.dirty || self.kinematic.needs_step() {
            self.change_counter += 1;
        }
        self.dirty = ragdoll_changed_colliders;
        self.mirror_settle_steps = self.mirror_settle_steps.saturating_sub(1);
        self.kinematic.step_done();
        self.last_step_micros = t.elapsed().as_micros() as u64;
    }

    /// Fold pending collider insertions/removals into the broad-phase BVH
    /// without advancing dynamics. A zero-dt pipeline step refreshes the BVH —
    /// so freshly streamed colliders become queryable — but integrates no
    /// motion, so a streaming frame's fold never double-steps awake clutter on
    /// top of the fixed `tick` (which owns time advancement). No-op when clean.
    pub fn fold(&mut self) {
        if !self.dirty {
            return;
        }
        let _span = tracing::info_span!("sim_fold_world_step").entered();
        self.audit_before_step();
        let dt = self.world.integration_parameters.dt;
        self.world.integration_parameters.dt = 0.0;
        self.world.step_with_events(self.ragdolls.hooks(), &());
        self.world.integration_parameters.dt = dt;
        self.change_counter += 1;
        self.dirty = false;
    }

    /// Generation of the queryable collider set (see `change_counter`).
    pub fn collider_generation(&self) -> u64 {
        self.change_counter
    }

    pub fn set_interior_mode(&mut self, enabled: bool) {
        self.interior_mode = enabled;
    }

    pub fn interior_mode(&self) -> bool {
        self.interior_mode
    }

    /// Whether the player may spawn/move at `grid` (the spawn ground-snap and
    /// the per-step ready gate both consult this). In a worldspace it is the
    /// per-cell ground-ready gate — the player only stands in cells whose collision
    /// is installed, whether that is streamed-in LAND or, for a landless cell, its objects.
    /// In an interior readiness is "the interior's collision is installed". The grid is
    /// intentionally ignored in that branch: the streamer freezes its ring and
    /// `teardown_resident` clears all residency on entry, so the ONLY cell ever resident in
    /// interior mode is the interior itself — hence a non-empty ready set means exactly
    /// "interior loaded", true at every grid the player roams to inside it. (Grid-keying this
    /// would wrongly freeze movement once the player crossed out of the interior's anchor
    /// grid in a large interior.)
    pub fn spawn_ready(&self, grid: CellGrid) -> bool {
        if self.interior_mode {
            !self.ground_ready.is_empty()
        } else {
            self.ground_ready.contains(&grid)
        }
    }

    pub fn collider_count(&self) -> usize {
        self.world.colliders.len()
    }

    pub fn cell_count(&self) -> usize {
        self.cell_colliders.len()
    }

    /// Tessellate every collider near `center` into world-space line
    /// segments (consecutive point pairs) for the debug wireframe overlay.
    /// The bool is true when output hit the segment cap.
    ///
    /// rapier's debug pipeline has no public per-collider entry point and
    /// `render_colliders` iterates the WHOLE set (180k+ here), so the AABB
    /// hits are cloned into a throwaway `ColliderSet` first — Collider clones
    /// are cheap (shapes are Arc-shared) and the cost stays proportional to
    /// what's actually near the camera. The clones' parent handles point at
    /// a deliberately empty body set; the pipeline treats them as
    /// parentless, which only affects its color suggestion (ignored here).
    pub fn wireframe_lines(&self, center: Vec3, half_extent: f32) -> (Vec<Vec3>, bool) {
        use rapier3d::pipeline::{
            DebugColor, DebugRenderBackend, DebugRenderMode, DebugRenderObject,
            DebugRenderPipeline, DebugRenderStyle,
        };

        /// Backstop against a pathological collider set building an
        /// unbounded vertex blob (8M segments ≈ 192 MB of line vertices).
        /// The query radius always spans the whole collision ring, so this
        /// must comfortably exceed the densest ring's segment count or
        /// colliders silently lose their overlay.
        const MAX_SEGMENTS: usize = 8_000_000;

        struct LineCollector {
            out: Vec<Vec3>,
            truncated: bool,
        }
        impl DebugRenderBackend for LineCollector {
            fn draw_line(
                &mut self,
                _object: DebugRenderObject,
                a: Vector,
                b: Vector,
                _color: DebugColor,
            ) {
                if self.out.len() >= MAX_SEGMENTS * 2 {
                    self.truncated = true;
                    return;
                }
                self.out.push(a);
                self.out.push(b);
            }
        }

        let aabb = Aabb::from_half_extents(center, Vec3::splat(half_extent));
        let mut nearby = ColliderSet::new();
        for (_, collider) in self
            .world
            .intersect_aabb_conservative(aabb, scene_query_filter())
        {
            nearby.insert(collider.clone());
        }
        if nearby.is_empty() {
            return (Vec::new(), false);
        }

        let mut backend = LineCollector {
            out: Vec::new(),
            truncated: false,
        };
        let mut pipeline = DebugRenderPipeline::new(
            DebugRenderStyle::default(),
            DebugRenderMode::COLLIDER_SHAPES,
        );
        pipeline.render_colliders(&mut backend, &RigidBodySet::new(), &nearby);
        if backend.truncated {
            tracing::warn!(
                "collision wireframe truncated at {MAX_SEGMENTS} segments — \
                 some colliders have no overlay"
            );
        }
        (backend.out, backend.truncated)
    }

    /// Cast a ray straight down from `origin` onto the first collider of any
    /// group (terrain or object); returns the hit's world-space Y. Used by the
    /// spawn ground-snap — worldspace spawn/switch and the freecam fly->walk
    /// handoff both stand the player on whatever surface is directly below.
    pub fn raycast_down(&self, origin: Vec3, max_dist: f32) -> Option<f32> {
        let ray = Ray::new(origin, -Vec3::Y);
        self.world
            .cast_ray(&ray, max_dist, true, scene_query_filter())
            .map(|(_, dist)| origin.y - dist)
    }

    /// `baked` is the streaming worker's pre-built shape payload for this
    /// batch (see `sim::bake`). Building shapes here is only the fallback —
    /// the trimesh BVH builds are exactly the main-thread stall the bake
    /// exists to avoid.
    pub fn add_cell(
        &mut self,
        grid: CellGrid,
        batch: &CellColliderBatch,
        baked: Option<&BakedCellColliders>,
    ) {
        if self.cell_colliders.contains_key(&grid) {
            // Shouldn't happen (the streamer removes before re-adding), but
            // never leak handles if it does.
            tracing::warn!("sim: add_cell on already-registered cell {grid:?}");
            self.remove_cell(grid);
        }

        let mut handles = Vec::new();
        let mut keys = Vec::new();

        match &batch.terrain {
            TerrainCollision::Present(terrain) => {
                let shape = match baked.and_then(|b| b.terrain.clone()) {
                    Some(shape) => Ok(shape),
                    None => build_terrain_shape(terrain),
                };
                match shape {
                    Ok(shape) => {
                        let collider = ColliderBuilder::new(shape).user_data(pack_grid(grid));
                        let handle = self.world.insert_collider(collider, None);
                        self.terrain_colliders.insert(handle);
                        handles.push(handle);
                        self.ground_ready.insert(grid);
                    }
                    Err(e) => {
                        tracing::warn!("collision: terrain trimesh failed for cell {grid:?}: {e}");
                    }
                }
            }
            TerrainCollision::Absent => {
                self.ground_ready.insert(grid);
            }
            TerrainCollision::Failed => {}
        }

        // (path, scale) -> pre-baked shapes, consulted on shape-cache misses.
        let baked_lookup: HashMap<(&str, u32), &Arc<Vec<CachedShape>>> = baked
            .map(|b| {
                b.objects
                    .iter()
                    .map(|g| ((g.path.as_str(), g.scale_bits), &g.shapes))
                    .collect()
            })
            .unwrap_or_default();
        let ragdoll_lookup: HashMap<(&str, u32), &RagdollBake> = baked
            .map(|b| {
                b.ragdolls
                    .iter()
                    .map(|g| ((g.path.as_str(), g.scale_bits), &g.outcome))
                    .collect()
            })
            .unwrap_or_default();

        let compound_dynamic = compound_dynamic_entities(batch);
        let dynamic_groups_first = batch
            .objects
            .iter()
            .filter(|group| group.dynamic)
            .chain(batch.objects.iter().filter(|group| !group.dynamic));
        for group in dynamic_groups_first {
            let mut slots = GroupSlots::new(&group.path);
            let ragdoll_capable = group.ragdoll_sidecar.is_some();
            for placement in &group.placements {
                if !placement.is_finite() {
                    warn_once(
                        &format!("collision: non-finite placement {}", group.path),
                        || {
                            format!(
                                "collision: {} has a placement with a non-finite pose or scale (degenerate REFR transform); skipped",
                                group.path
                            )
                        },
                    );
                    continue;
                }
                let ref_bits = self.pack_ref_key(placement.ref_key.as_deref());
                let key: ShapeKey = (group.path.clone(), placement.scale.to_bits());
                let entry = self.shape_cache.entry(key.clone()).or_insert_with(|| {
                    let shapes = baked_lookup
                        .get(&(group.path.as_str(), placement.scale.to_bits())).map_or_else(|| {
                            // bake_cell bakes every (path, scale) in the batch, so a
                            // miss here means the baked payload is incomplete. The raw
                            // shapes are no longer carried on the main thread (they load
                            // lazily at bake time), so surface it and spawn no collider
                            // rather than stalling the main thread with a BVH build —
                            // exactly what the worker-side bake exists to avoid.
                            tracing::warn!(
                                "collision: no baked shape for {} @ scale {} (incomplete bake payload)",
                                group.path,
                                placement.scale
                            );
                            Arc::new(Vec::new())
                        }, |arc| (*arc).clone());
                    CacheEntry { shapes: without_unsound_shapes(&group.path, shapes), refcount: 0 }
                });
                entry.refcount += 1;
                keys.push(key);

                let refr_pose = Pose::from_parts(placement.translation, placement.rotation);
                let part_pose = placement.part_local.map(|local| {
                    Pose::from_parts(local.translation * placement.scale, local.rotation)
                });
                let entity = placement.ref_key.as_deref().map(refr_entity_id);

                // Havok-dynamic clutter becomes one rigid body per REFR (keyed by
                // its stable entity id); everything else is a parentless static
                // collider, exactly as before. A dynamic placement falls back to
                // the static path (a collider, never a walk-through) when it: has
                // no form key (can't be addressed across the sim/render split);
                // or duplicates an already-spawned entity (FNV collision / a
                // doubly-placed ref).
                // A dynamic body with no colliders (every sub-shape degenerate,
                // or an empty sidecar) would never contact or sleep, pinning the
                // sim awake forever; fall back to the (colliderless) static path.
                // Surface it — an intended-dynamic mesh that bakes no shape is a
                // data problem the fallback would otherwise hide.
                let has_shapes = !entry.shapes.is_empty();
                if group.dynamic && !ragdoll_capable && !has_shapes {
                    tracing::warn!(
                        "collision: dynamic mesh {} baked no shapes; spawning intangible",
                        group.path
                    );
                }
                // A ragdoll-capable mesh is `dynamic` too (its sidecar has Havok motion),
                // but it is never a single rigid body: the ragdoll manager owns it, and
                // when that rejects the placement it stays static.
                let joins_compound = part_pose.is_some()
                    && entity.is_some_and(|entity| compound_dynamic.contains_key(&entity));
                let body_entity = ((group.dynamic || joins_compound) && !ragdoll_capable)
                    .then_some(entity)
                    .flatten()
                    .filter(|_| has_shapes);
                let compound_body = part_pose
                    .and(body_entity)
                    .and_then(|entity| self.dyn_bodies.get(&entity))
                    .filter(|body| body.compound && body.grid == grid)
                    .map(|body| body.handle);
                let collider_pose = |cached: &CachedShape| {
                    part_pose.map_or(cached.local_pose, |part| part * cached.local_pose)
                };

                if let Some(body_handle) = compound_body {
                    let density = density_for_mass(&entry.shapes, group.mass);
                    for cached in entry.shapes.iter() {
                        let path_slot = slots.slot(&mut self.paths, cached.material);
                        let cb = ColliderBuilder::new(cached.shape.clone())
                            .position(collider_pose(cached))
                            .density(density)
                            .user_data(pack_grid(grid) | pack_path(path_slot) | ref_bits);
                        self.world.insert_collider(cb, Some(body_handle));
                    }
                    self.dirty = true;
                } else if let Some(entity) =
                    body_entity.filter(|entity| !self.dyn_bodies.contains_key(entity))
                {
                    let water_props = compound_dynamic
                        .get(&entity)
                        .filter(|_| part_pose.is_some())
                        .map_or_else(|| group.water.clone(), Clone::clone);
                    // Spawn Fixed; the same-frame SetPhysicsCenter reconcile
                    // flips it Dynamic when it's inside the radius.
                    let body = body_builder(
                        RigidBodyType::Fixed,
                        DYN_LINEAR_DAMPING,
                        DYN_ANGULAR_DAMPING,
                    )
                    .pose(refr_pose);
                    let body_handle = self.world.insert_body(body);
                    let density = density_for_mass(&entry.shapes, group.mass);
                    for cached in entry.shapes.iter() {
                        let path_slot = slots.slot(&mut self.paths, cached.material);
                        let cb = ColliderBuilder::new(cached.shape.clone())
                            .position(collider_pose(cached))
                            .density(density)
                            .user_data(pack_grid(grid) | pack_path(path_slot) | ref_bits);
                        self.world.insert_collider(cb, Some(body_handle));
                    }
                    self.dyn_bodies.insert(
                        entity,
                        DynBody {
                            handle: body_handle,
                            grid,
                            prev_pose: refr_pose,
                            cur_pose: refr_pose,
                            active: false,
                            ever_active: false,
                            water_props,
                            water: WaterBodyState::default(),
                            compound: part_pose.is_some(),
                        },
                    );
                    self.cell_dyn.entry(grid).or_default().push(entity);
                    self.dirty = true;
                } else {
                    let base_user_data = pack_grid(grid) | ref_bits;
                    if ragdoll_capable {
                        let bake = ragdoll_lookup
                            .get(&(group.path.as_str(), placement.scale.to_bits()))
                            .copied();
                        let bone_materials = bake
                            .and_then(|bake| bake.as_ref().ok())
                            .into_iter()
                            .flat_map(|baked| baked.body_shapes.iter().flatten())
                            .map(|cached| cached.material);
                        let site = PlacementSite {
                            grid,
                            refr_pose,
                            tag: slots.tag(
                                &mut self.paths,
                                base_user_data,
                                entry
                                    .shapes
                                    .iter()
                                    .map(|cached| cached.material)
                                    .chain(bone_materials),
                            ),
                        };
                        match ragdoll_placement(
                            group,
                            placement,
                            site,
                            &entry.shapes,
                            bake,
                            &self.ragdolls,
                        ) {
                            Ok(ragdoll) => {
                                self.ragdolls.add_slot(&mut self.world, ragdoll);
                                self.dirty = true;
                                continue;
                            }
                            Err(rejection) => self.ragdolls.report_rejection(
                                &rejection,
                                &group.path,
                                placement.ref_key.as_deref(),
                            ),
                        }
                    }
                    // Shapes following an animated joint ride kinematic bodies
                    // keyed by the REFR entity; a placement without a usable
                    // entity keeps them static.
                    let kinematic_entity = placement.ref_key.as_deref().map(refr_entity_id);
                    let (moving, fixed): (Vec<&CachedShape>, Vec<&CachedShape>) = entry
                        .shapes
                        .iter()
                        .partition(|cached| cached.joint.is_some() && kinematic_entity.is_some());
                    for cached in fixed {
                        let path_slot = slots.slot(&mut self.paths, cached.material);
                        let collider = ColliderBuilder::new(cached.shape.clone())
                            .position(refr_pose * collider_pose(cached))
                            .user_data(pack_grid(grid) | pack_path(path_slot) | ref_bits);
                        handles.push(self.world.insert_collider(collider, None));
                    }
                    if let Some(entity) = kinematic_entity.filter(|_| !moving.is_empty()) {
                        let tag = slots.tag(
                            &mut self.paths,
                            base_user_data,
                            moving.iter().map(|cached| cached.material),
                        );
                        let bodies = self.kinematic.attach(
                            &mut self.world,
                            grid,
                            entity,
                            part_pose.map_or(refr_pose, |part| refr_pose * part),
                            &moving,
                            &tag,
                        );
                        tracing::debug!(
                            "kinematic: {} bodies={bodies}",
                            placement.ref_key.as_deref().unwrap_or("?")
                        );
                        self.dirty = true;
                    }
                }
            }
        }

        if !handles.is_empty() {
            self.dirty = true;
        }
        let dyn_count = self.cell_dyn.get(&grid).map_or(0, std::vec::Vec::len);
        tracing::debug!(
            "sim: add_cell {grid:?} colliders={} dynamic={} world_total={} bodies={}",
            handles.len(),
            dyn_count,
            self.world.colliders.len(),
            self.world.bodies.len()
        );
        self.cell_colliders.insert(grid, handles);
        self.cell_cache_keys.insert(grid, keys);
    }

    /// Parentless (static) colliders have no body for rapier to wake on
    /// removal, so bodies that rested on them would sleep on over a hole.
    /// Wake the active bodies in the cell and its neighbours.
    fn wake_bodies_near_cell(&mut self, grid: CellGrid) {
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(entities) = self.cell_dyn.get(&(grid.0 + dx, grid.1 + dy)) else {
                    continue;
                };
                let active_bodies = entities
                    .iter()
                    .filter_map(|entity| self.dyn_bodies.get(entity))
                    .filter(|db| db.active);
                for db in active_bodies {
                    if let Some(rb) = self.world.bodies.get_mut(db.handle)
                        && rb.is_sleeping()
                    {
                        rb.wake_up(true);
                    }
                }
            }
        }
    }

    pub fn remove_cell(&mut self, grid: CellGrid) {
        let mut removed_bounds: Option<Aabb> = None;
        if let Some(handles) = self.cell_colliders.remove(&grid) {
            if !handles.is_empty() {
                self.dirty = true;
            }
            tracing::debug!("sim: remove_cell {grid:?} colliders={}", handles.len());
            removed_bounds = handles
                .iter()
                .filter_map(|handle| self.world.colliders.get(*handle))
                .map(rapier3d::geometry::Collider::compute_aabb)
                .reduce(|merged, bounds| merged.merged(&bounds));
            for h in handles {
                self.terrain_colliders.remove(&h);
                self.world.remove_collider(h);
            }
            self.wake_bodies_near_cell(grid);
        }
        // Despawn this cell's dynamic bodies (removing a body drops its
        // colliders too). This is the single despawn site: unload, LOD
        // demotion, worldspace switch and interior entry all reach it through
        // RemoveCellColliders.
        if let Some(entities) = self.cell_dyn.remove(&grid) {
            for entity in entities {
                if let Some(db) = self.dyn_bodies.remove(&entity) {
                    self.world.remove_body(db.handle);
                    self.dirty = true;
                }
            }
        }
        if self.kinematic.remove_cell(&mut self.world, grid) {
            self.dirty = true;
        }
        if self.ragdolls.remove_cell(&mut self.world, grid) {
            self.dirty = true;
        }
        if let Some(bounds) = removed_bounds {
            self.ragdolls.wake_near_bounds(&mut self.world, &bounds);
        }
        self.ground_ready.remove(&grid);
        if let Some(keys) = self.cell_cache_keys.remove(&grid) {
            for key in keys {
                if let Some(entry) = self.shape_cache.get_mut(&key) {
                    entry.refcount -= 1;
                    if entry.refcount == 0 {
                        self.shape_cache.remove(&key);
                    }
                }
            }
        }
    }

    /// Apply a player shove to `handle` (at `point`, or through its centre of
    /// mass) and bound the speed it leaves, so the push can't drive a light
    /// prop faster than soft-CCD can catch in one step.
    pub fn shove_body(&mut self, handle: RigidBodyHandle, impulse: Vec3, point: Option<Vec3>) {
        let Some(rb) = self
            .world
            .bodies
            .get_mut(handle)
            .filter(|rb| rb.is_dynamic())
        else {
            return;
        };
        match point {
            Some(point) => rb.apply_impulse_at_point(impulse, point, true),
            None => rb.apply_impulse(impulse, true),
        }
        clamp_linvel(rb, PUSH_MAX_LINVEL, true);
    }

    /// Mirror the player capsule into the rapier world so contact-start wakes
    /// sleeping clutter it touches (see `mirror`). `teleport` repositions
    /// instantly (spawn/teleport/cell change); otherwise the mirror follows the
    /// player's move. Steps are only owed while the mirror changed AND a dynamic
    /// body is within reach, so an idle or clutter-free player adds no stepping.
    pub fn sync_player_mirror(&mut self, center: Vec3, capsule: &Capsule, teleport: bool) {
        if !(vec_is_sound(center)
            && capsule.radius.is_finite()
            && capsule.half_height().is_finite())
        {
            warn_once("physics: non-finite player mirror", || {
                format!(
                    "physics: player mirror update with a non-finite centre or capsule                      (centre {center:?}, radius {}, half height {}); ignored",
                    capsule.radius,
                    capsule.half_height()
                )
            });
            return;
        }
        let Some(mirror) = self.player_mirror.as_mut() else {
            self.player_mirror = Some(KinematicMirror::new(&mut self.world, center, capsule));
            self.dirty = true;
            return;
        };
        let changed = if teleport {
            mirror.teleport(&mut self.world, center, capsule);
            true
        } else {
            mirror.follow(&mut self.world, center, capsule)
        };
        if !changed {
            return;
        }
        let reach = capsule
            .compute_aabb(&Pose::from_translation(center))
            .loosened(MIRROR_NEAR_RANGE);
        if self.any_dynamic_body_within(reach) {
            self.mirror_settle_steps = MIRROR_SETTLE_STEPS;
        }
    }

    /// Contact-start wake is weak (it leaves the sleep timer running), and the
    /// mirror applies no impulse, so strong-wake what it touches or a body put
    /// to sleep by hand could doze off again mid-air.
    fn wake_bodies_touching_mirror(&mut self) {
        if self.mirror_settle_steps == 0 {
            return;
        }
        let Some(mirror) = &self.player_mirror else {
            return;
        };
        for handle in mirror.touched_bodies(&self.world) {
            self.world.wake_up(handle, true);
        }
    }

    fn any_dynamic_body_within(&self, aabb: Aabb) -> bool {
        let queries = self.world.query_pipeline_with_filter(scene_query_filter());
        queries
            .intersect_aabb_conservative(aabb)
            .filter_map(|(_, collider)| collider.parent())
            .any(|handle| {
                self.world
                    .bodies
                    .get(handle)
                    .is_some_and(rapier3d::dynamics::RigidBody::is_dynamic)
            })
    }

    /// Move the dynamic-simulation region. Bodies inside it are `Dynamic`
    /// (simulated), the rest `Fixed` (frozen at their last pose, still
    /// collidable). Pushed every frame by the streamer.
    pub fn set_physics_center(&mut self, grid: CellGrid, radius: i32) {
        self.dyn_center = grid;
        self.dyn_radius = radius.max(0);
        self.reconcile_dynamic_radius();
    }

    /// Flip each body `Dynamic`↔`Fixed` to match the current region. Only bodies
    /// whose membership actually changed are touched; steady state is a cheap
    /// scan over the (radius-bounded) dynamic set.
    fn reconcile_dynamic_radius(&mut self) {
        let r = self.dyn_radius;
        let wake = self.activation == ActivationMode::Wake;
        if self.ragdolls.reconcile(&mut self.world, self.dyn_center, r) {
            self.dirty = true;
        }
        for db in self.dyn_bodies.values_mut() {
            let in_radius = within_radius(db.grid, self.dyn_center, r);
            if in_radius == db.active {
                continue;
            }
            let Some(rb) = self.world.bodies.get_mut(db.handle) else {
                continue;
            };
            if in_radius {
                // Only the first activation may stay asleep (settled authored
                // pose); a body that has been simulated before must wake so it
                // cannot resume asleep mid-air.
                rb.set_body_type(RigidBodyType::Dynamic, wake || db.ever_active);
                if !wake && !db.ever_active {
                    rb.sleep();
                }
                db.ever_active = true;
                // Reset interpolation to the resting pose so the body doesn't
                // appear to lerp from a stale previous frame on activation.
                let pose = *rb.position();
                db.prev_pose = pose;
                db.cur_pose = pose;
                db.active = true;
            } else {
                db.cur_pose = *rb.position();
                rb.set_body_type(RigidBodyType::Fixed, false);
                db.active = false;
            }
        }
    }

    /// Interpolated world poses of bodies that MOVED this frame, for the render
    /// side to rewrite into the instance buffer. Only glam types cross out — the
    /// rapier `Pose` stays inside `sim`.
    ///
    /// Sleeping bodies are skipped: their instance already holds the pose from
    /// their last awake frame, so re-uploading it every frame is pure waste —
    /// doing so for all resident bodies cost a large idle frame-rate regression.
    /// A push or a fresh activation wakes a body and it reappears here.
    pub fn dynamic_poses(&self, alpha: f32, out: &mut Vec<DynamicPose>) {
        for (&entity, db) in &self.dyn_bodies {
            if !db.active {
                continue;
            }
            let awake = self
                .world
                .bodies
                .get(db.handle)
                .is_some_and(|rb| !rb.is_sleeping());
            if !awake {
                continue;
            }
            out.push(DynamicPose {
                entity,
                translation: db
                    .prev_pose
                    .translation
                    .lerp(db.cur_pose.translation, alpha),
                rotation: db.prev_pose.rotation.slerp(db.cur_pose.rotation, alpha),
            });
        }
    }

    pub fn ragdoll_settings(&self) -> RagdollSettings {
        self.ragdolls.settings()
    }

    pub fn set_ragdoll_settings(&mut self, settings: RagdollSettings) {
        self.ragdolls.set_settings(settings);
    }

    pub fn ragdoll_poses(&mut self, out: &mut RagdollPoses) {
        self.ragdolls.collect_poses(&self.world, out);
    }

    pub fn republish_ragdoll(&mut self, entity: EntityId) {
        self.ragdolls.republish(entity);
    }

    pub fn ragdoll_has_pose(&self, entity: EntityId) -> bool {
        self.ragdolls.has_pose(entity)
    }

    pub fn disable_ragdoll_model(&mut self, mesh_path: &str) {
        if self.ragdolls.disable_model(&mut self.world, mesh_path) {
            self.dirty = true;
        }
    }

    pub fn set_kinematic_poses(&mut self, poses: &[KinematicPose]) {
        self.kinematic.set_poses(&mut self.world, poses);
    }

    pub fn release_kinematic_poses(&mut self, entities: &[EntityId]) {
        self.kinematic.release_entities(entities);
    }

    pub fn release_all_kinematic_poses(&mut self) {
        self.kinematic.release_all();
    }

    /// Bodies currently simulating (within the radius). Computed on demand from
    /// the radius-bounded dynamic set, so it can't drift out of sync.
    pub fn active_dynamic_count(&self) -> usize {
        self.dyn_bodies.values().filter(|db| db.active).count()
    }

    pub fn dynamic_body_count(&self) -> usize {
        self.dyn_bodies.len()
    }
}

/// The shapes whose own pose is usable, so a corrupt baked shape costs that one collider rather
/// than the broad phase.
fn without_unsound_shapes(path: &str, shapes: Arc<Vec<CachedShape>>) -> Arc<Vec<CachedShape>> {
    if shapes
        .iter()
        .all(|cached| pose_is_sound(&cached.local_pose))
    {
        return shapes;
    }
    warn_once(&format!("collision: non-finite shape {path}"), || {
        format!("collision: {path} has a shape with a non-finite local pose; that shape is skipped")
    });
    Arc::new(
        shapes
            .iter()
            .filter(|cached| pose_is_sound(&cached.local_pose))
            .cloned()
            .collect(),
    )
}

/// REFRs with a dynamic weapon part, with the water properties of the first
/// such part: every part of such a REFR joins its body, also parts whose own
/// mesh is not dynamic, and the body takes its water properties from the
/// dynamic part whichever part ends up creating it.
fn compound_dynamic_entities(
    batch: &CellColliderBatch,
) -> HashMap<EntityId, Option<Arc<WaterBodyProps>>> {
    let mut entities = HashMap::new();
    for group in batch
        .objects
        .iter()
        .filter(|group| group.dynamic && group.ragdoll_sidecar.is_none())
    {
        let compound_entities = group
            .placements
            .iter()
            .filter(|placement| placement.part_local.is_some())
            .filter_map(|placement| placement.ref_key.as_deref().map(refr_entity_id));
        for entity in compound_entities {
            entities
                .entry(entity)
                .or_insert_with(|| group.water.clone());
        }
    }
    entities
}

/// Where a placement sits in the world and how its colliders are attributed.
struct PlacementSite {
    grid: CellGrid,
    refr_pose: Pose,
    tag: ColliderTag,
}

/// The ragdoll registration of a placement of a ragdoll-capable mesh, or why it
/// stays static: it must be addressable (form key), finite, unique, not disabled
/// by the renderer, and carry a successfully baked sidecar.
fn ragdoll_placement(
    group: &ObjectColliderGroup,
    placement: &Placement,
    site: PlacementSite,
    legacy_shapes: &Arc<Vec<CachedShape>>,
    bake: Option<&RagdollBake>,
    ragdolls: &RagdollManager,
) -> Result<RagdollPlacement, RagdollRejection> {
    let ref_key = placement
        .ref_key
        .clone()
        .ok_or(RagdollRejection::NoFormKey)?;
    if !(placement.is_finite() && placement.scale > 0.0) {
        return Err(RagdollRejection::BadTransform);
    }
    if ragdolls.is_model_disabled(&group.path) {
        return Err(RagdollRejection::ModelDisabled);
    }
    let entity = refr_entity_id(&ref_key);
    if ragdolls.contains(entity) {
        return Err(RagdollRejection::Duplicate);
    }
    let baked = match bake.ok_or(RagdollRejection::NotBaked)? {
        Ok(baked) => Arc::clone(baked),
        Err(reason) => return Err(RagdollRejection::BakeFailed(Arc::clone(reason))),
    };
    let authored = placement.ragdoll_pose.as_deref().and_then(|xrgd| {
        match authored_body_poses(&group.path, &baked.spec, xrgd, site.refr_pose, placement.scale) {
            Ok(poses) => Some(Arc::new(poses)),
            Err(error) => {
                warn_once(&format!("ragdoll-pose:{ref_key}"), || {
                    format!("ragdoll: {ref_key} ({}): authored pose unusable ({error}); using the rest pose", group.path)
                });
                None
            }
        }
    });
    Ok(RagdollPlacement {
        grid: site.grid,
        entity,
        mesh_path: group.path.clone(),
        ref_key,
        scale: placement.scale,
        refr_pose: site.refr_pose,
        tag: site.tag,
        legacy_shapes: Arc::clone(legacy_shapes),
        baked,
        authored,
    })
}

mod audit;

#[cfg(test)]
pub(super) mod test_support;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod water_tests;
