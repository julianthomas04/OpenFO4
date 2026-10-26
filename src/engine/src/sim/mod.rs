//! Simulation side of the engine: physics, the player, and (eventually) all
//! gameplay state. Rules (docs/engine/README.md): the sim
//! owns its state, mutations enter only through [`SimCommand`]s, entities are
//! identified by [`EntityId`] — render code submits commands and reads poses.
//! rapier types must never leak outside this module.

pub mod bake;
mod cache_purge;
mod collider_user_data;
pub mod collision;
mod command_budget;
pub mod commands;
mod dynamic_body;
mod dynamic_contact;
mod footstep_clock;
pub mod hud;
mod kinematic;
mod mirror;
mod physics;
pub mod pipboy;
mod player;
mod ragdoll;
mod ragdoll_pose;
pub mod ragdoll_spec;
mod ragdoll_tuning;
mod sidecar_hash;
mod sound_pose;
mod water_materials;
pub mod water_physics;
pub mod world_clock;

use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::{Quat, Vec3};

use crate::footsteps::{FootstepEvent, GaitTimings, SurfaceMaterial};

use command_budget::{Deferred, DeferredQueue};
use footstep_clock::{FootstepClock, TickObservation};

pub use commands::{KinematicPose, PlayerInput, SimCommand};
pub use footstep_clock::{JumpWindupMismatch, jump_windup_mismatch};
pub use hud::{CrosshairState, HudSnapshot, MeterFill};
pub use player::{PlayerMovement, STANDING_CENTER_ABOVE_FEET};
pub use ragdoll::{BodyDelta, MAX_SKINNED_RAGDOLLS, RagdollEntry, RagdollPoses};
pub use ragdoll_tuning::RagdollSettings;
pub use water_materials::{MaterialBuoyancy, WaterBodyProps};
pub use water_physics::{WaterCurrent, WaterQuery, WaterSurface};
pub use world_clock::WorldClock;

/// The longest step the world clock takes in one frame.
const MAX_CLOCK_STEP_SECONDS: f32 = 0.25;

/// What the frame loop tells the sim about one frame, for the world clock.
#[derive(Clone, Copy, Debug)]
pub struct ClockFrame {
    /// Whether the game flow is in gameplay (not a menu, loading or the reveal).
    pub in_game: bool,
    /// The world time of the frame, from [`SimWorld::world_dt`].
    pub world_dt: f32,
}

/// Stable identity for sim entities. The player is entity 1; static world
/// collision is tracked per-cell rather than per-entity until something needs
/// to address individual statics.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct EntityId(pub u64);

pub const PLAYER_ENTITY: EntityId = EntityId(1);

/// Stable entity id for a placed reference, derived from its globally-unique
/// form key ("001ED5:Fallout4.esm") by FNV-1a so the sim (which keys dynamic
/// bodies) and the renderer (which keys instance rows) derive the *same* id
/// independently, with no shared registry. Values `0`/`1` are reserved (the
/// sentinel + [`PLAYER_ENTITY`]); a hash landing there is bumped to 2.
pub fn refr_entity_id(form_key: &str) -> EntityId {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in form_key.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    EntityId(if h < 2 { 2 } else { h })
}

/// Interpolated world pose of one simulated dynamic body, in glam types so no
/// rapier type leaves the sim. The render side composes this with the
/// placement's uniform scale.
pub struct DynamicPose {
    pub entity: EntityId,
    pub translation: Vec3,
    pub rotation: Quat,
}

const FOLD_DEFER_ENV: &str = "OPENFO4_SIM_FOLD_DEFER";

fn fold_deferral_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| crate::utils::env_flag(FOLD_DEFER_ENV, true))
}

/// Per-frame time the render side gives
/// [`SimWorld::drain_pending_colliders`] while collider adds are queued.
pub const COLLIDER_APPLY_BUDGET: Duration = Duration::from_millis(6);

/// Fixed sim tick: deterministic across frame rates.
pub const FIXED_DT: f32 = 1.0 / 60.0;

/// Most fixed steps one frame may run (4 steps = 15 fps floor before the sim
/// slows down). Normal frame rates never reach it, so stepping is unchanged.
const MAX_SUBSTEPS_PER_TICK: u32 = 4;

/// World-space size of one exterior cell. The streamer asserts its own
/// constant equals this one (single contract, two modules).
pub const CELL_SIZE_WORLD: f32 = 409.6;

pub type CellGrid = (i32, i32);

/// Engine-space position -> exterior cell grid coordinate. Mirrors
/// `WorldStreamer::camera_to_grid` (Bethesda +Y is engine -Z).
pub fn world_to_grid(pos: Vec3) -> CellGrid {
    (
        (pos.x / CELL_SIZE_WORLD).floor() as i32,
        (-pos.z / CELL_SIZE_WORLD).floor() as i32,
    )
}

/// The spawned player's body, for what reacts to where its feet are.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct PlayerBody {
    /// Engine-space position of the soles.
    pub feet: Vec3,
    /// Engine units from the soles to the top of the capsule.
    pub height: f32,
    /// Engine units, horizontal radius of the capsule.
    pub radius: f32,
}

/// Read-only player snapshot for the debug overlay.
pub struct PlayerDebug {
    pub pos: Vec3,
    pub grounded: bool,
    pub vertical_vel: f32,
    pub crouching: bool,
    pub spawned: bool,
    /// A spawn/teleport is pending a ground raycast (ground not ready yet).
    pub waiting_for_ground: bool,
    pub cell: CellGrid,
    pub cell_ready: bool,
}

/// What the debug aim ray hit, for the overlay readout.
pub struct RayHit {
    /// GLB mesh path, or "terrain".
    pub mesh_path: String,
    /// REFR form key ("001ED5:Fallout4.esm") — the game-wide unique
    /// placement id, distinguishing copies of the same mesh. `None` for
    /// terrain.
    pub ref_id: Option<String>,
    pub dist: f32,
}

/// Read-only physics stats for the debug overlay.
pub struct SimStats {
    pub collider_count: usize,
    pub cells_with_colliders: usize,
    pub last_step_micros: u64,
    pub steps_last_tick: u32,
    /// Dynamic bodies currently simulating (within the physics radius).
    pub active_dynamic: usize,
    /// Dynamic bodies resident (registered, active or frozen).
    pub dyn_total: usize,
}

/// A cell's collider payload waiting in the budgeted queue.
type CellPayload = (
    Arc<collision::CellColliderBatch>,
    Option<bake::BakedCellColliders>,
);

pub struct SimWorld {
    physics: physics::PhysicsContext,
    player: player::PlayerState,
    input: commands::PlayerInput,
    accumulator: f32,
    steps_last_tick: u32,
    warned_substep_clamp: bool,
    pipboy: pipboy::PipBoyModel,
    world_clock: WorldClock,
    /// While set, `tick` does not advance and collider adds are queued.
    fold_hold: bool,
    pending_colliders: DeferredQueue<CellPayload>,
    footstep_clock: FootstepClock,
    footsteps: Vec<FootstepEvent>,
}

impl SimWorld {
    pub fn new(movement: PlayerMovement, world_clock: WorldClock) -> Self {
        let mut physics = physics::PhysicsContext::new();
        physics.set_water_query(water_physics::test_plane_from_env());
        Self {
            physics,
            player: player::PlayerState::new(movement),
            input: commands::PlayerInput::default(),
            accumulator: 0.0,
            steps_last_tick: 0,
            warned_substep_clamp: false,
            pipboy: pipboy::PipBoyModel::new(Arc::default()),
            world_clock,
            fold_hold: false,
            pending_colliders: DeferredQueue::new(),
            footstep_clock: FootstepClock::default(),
            footsteps: Vec::new(),
        }
    }

    /// Installs the cycle timings the footstep clock replays; until then the
    /// player makes no footstep events.
    pub fn install_gait_timings(&mut self, timings: GaitTimings) {
        self.footstep_clock.install(timings);
    }

    /// Footstep events since the last drain, in the order they happened.
    pub fn drain_footsteps(&mut self) -> Vec<FootstepEvent> {
        std::mem::take(&mut self.footsteps)
    }

    /// Holds (or releases) the physics step and the player step. While held,
    /// `tick` runs nothing and `AddCellColliders` are queued for
    /// [`drain_pending_colliders`](Self::drain_pending_colliders) instead of
    /// inserted at once, so a bulk load pays no broad-phase fold per cell and
    /// no insert burst in one frame. Releasing lets the next `tick` fold every
    /// insert in one pass before any player query; the player spawn resumes
    /// from there, still gated on its cell's collision being installed.
    pub fn set_fold_hold(&mut self, hold: bool) {
        self.fold_hold = hold;
    }

    /// Whether the fold is held: a bulk load is under way and nothing simulates.
    pub fn fold_held(&self) -> bool {
        self.fold_hold
    }

    /// Installs the clock the world starts from, loaded from the game data at startup.
    pub fn world_clock(&self) -> &WorldClock {
        &self.world_clock
    }

    /// The seconds of world time a frame of `dt` seconds carries: none while the Pip-Boy is open.
    pub fn world_dt(&self, dt: f32) -> f32 {
        if self.pipboy.open { 0.0 } else { dt }
    }

    /// Advances the world clock for one frame. The clock runs only in gameplay and not while a
    /// load holds the simulation; the step is bounded so a hitch cannot jump the day, and a
    /// negative or NaN step is no step.
    #[allow(
        clippy::manual_clamp,
        reason = "f32::clamp passes NaN through, max/min turn a NaN frame time into no progress"
    )]
    pub fn advance_world_clock(&mut self, frame: ClockFrame) {
        if !frame.in_game || self.fold_hold {
            return;
        }
        self.world_clock
            .advance(frame.world_dt.max(0.0).min(MAX_CLOCK_STEP_SECONDS));
    }

    /// Inserts queued collider work in FIFO order until `budget` is spent, at
    /// least one entry per call. The time of an insert includes any fallback
    /// shape build it has to do, so that cost is bounded by the same budget.
    pub fn drain_pending_colliders(&mut self, budget: Duration) {
        if self.pending_colliders.is_empty() {
            return;
        }
        let span = tracing::info_span!(
            "sim_drain_colliders",
            queued = self.pending_colliders.len(),
            applied = tracing::field::Empty
        )
        .entered();
        let started = Instant::now();
        let applied = self.pending_colliders.drain_within(
            budget,
            || started.elapsed(),
            |entry| apply_collider_work(&mut self.physics, entry),
        );
        span.record("applied", applied);
    }

    /// Drops every queued collider add and physics-centre move. A teleport
    /// into another worldspace calls this after its teardown removes, which
    /// already cover every registered grid: the queued work belongs to the
    /// world being left, and a stale centre move must not apply in the new one.
    pub fn discard_pending_colliders(&mut self) {
        self.pending_colliders.clear();
    }

    /// Whether collider work is still waiting in the queue.
    pub fn has_pending_colliders(&self) -> bool {
        !self.pending_colliders.is_empty()
    }

    /// Adds go to the queue while held, and while the queue still holds work:
    /// an add must not overtake the entries ahead of it.
    fn defers_colliders(&self) -> bool {
        self.fold_hold || !self.pending_colliders.is_empty()
    }

    /// Installs (or clears) the water source the clutter bodies react to.
    pub fn set_water_query(&mut self, query: Option<WaterQuery>) {
        self.physics.set_water_query(query);
    }

    /// Installs the MATT buoyancy table (by material hash) that dynamic bodies
    /// and ragdoll bones resolve their materials against. Call it before the
    /// first cell is added; without a table every body has buoyancy 1.0.
    pub fn set_material_buoyancy_table(&mut self, table: Arc<MaterialBuoyancy>) {
        self.physics.set_material_buoyancy_table(table);
    }

    /// Fixes the mean MATT `BNAM` buoyancy of a spawned dynamic body,
    /// overriding what its materials resolve to.
    pub fn set_material_buoyancy(&mut self, entity: EntityId, buoyancy: f32) {
        self.physics.set_material_buoyancy(entity, buoyancy);
    }

    /// Aims the kinematic bodies of animated parts at their latest world
    /// poses. Takes a slice instead of a command so the renderer's per-frame
    /// output reuses one buffer.
    pub fn set_kinematic_poses(&mut self, poses: &[KinematicPose]) {
        self.physics.set_kinematic_poses(poses);
    }

    /// Mutation entry point for sim commands.
    pub fn apply(&mut self, cmd: SimCommand) {
        match cmd {
            SimCommand::AddCellColliders { grid, batch, baked } => {
                if self.defers_colliders() {
                    self.pending_colliders.push_add(grid, (batch, baked));
                } else {
                    apply_collider_work(
                        &mut self.physics,
                        Deferred::AddCell {
                            grid,
                            payload: (batch, baked),
                        },
                    );
                }
            }
            SimCommand::RemoveCellColliders { grid } => {
                self.pending_colliders.cancel_add(grid);
                let _span =
                    tracing::trace_span!("sim_remove_cell", gx = grid.0, gy = grid.1).entered();
                self.physics.remove_cell(grid);
            }
            SimCommand::SetPhysicsCenter { grid, radius } => {
                if self.pending_colliders.is_empty() {
                    apply_collider_work(
                        &mut self.physics,
                        Deferred::PhysicsCenter { grid, radius },
                    );
                } else {
                    self.pending_colliders.push_physics_center(grid, radius);
                }
            }
            SimCommand::SpawnPlayer { pos } => {
                // Worldspace content grounds onto the first surface below the
                // start; an interior is placed at its authored start and settles
                // onto the object-mesh floor under gravity (no snap).
                let snap = if self.physics.interior_mode() {
                    player::SpawnSnap::Settle
                } else {
                    player::SpawnSnap::Down
                };
                self.player.request_spawn(pos, snap);
            }
            SimCommand::TeleportPlayer { pos } => {
                self.player.request_spawn(pos, player::SpawnSnap::Down);
            }
            SimCommand::SetPlayerInput(input) => {
                self.input = if self.pipboy.open {
                    input.without_movement()
                } else {
                    input
                };
            }
            SimCommand::SetInteriorMode(enabled) => self.physics.set_interior_mode(enabled),
            SimCommand::ReleaseKinematicPoses(entities) => {
                self.physics.release_kinematic_poses(&entities);
            }
            SimCommand::ReleaseAllKinematicPoses => self.physics.release_all_kinematic_poses(),
            SimCommand::SetRagdollSettings(settings) => self.physics.set_ragdoll_settings(settings),
            SimCommand::PipBoy(action) => self.apply_pipboy(action),
            SimCommand::SetWorldClock(edit) => self.apply_clock_edit(edit),
        }
    }

    fn apply_clock_edit(&mut self, edit: commands::ClockEdit) {
        let result = match edit {
            commands::ClockEdit::SetHour(hour) => self.world_clock.set_hour(hour),
            commands::ClockEdit::SetPaused(paused) => {
                self.world_clock.set_paused(paused);
                Ok(())
            }
            commands::ClockEdit::SetTimeScale(scale) => self.world_clock.set_time_scale(scale),
        };
        if let Err(error) = result {
            tracing::warn!("sim: world clock edit {edit:?} rejected: {error}");
        }
    }

    fn apply_pipboy(&mut self, action: pipboy::PipBoyAction) {
        let effects = self.pipboy.submit(action);
        self.player.heal(effects.heal_hp);
        if self.pipboy.open {
            self.input = self.input.without_movement();
        }
    }

    /// Fixed-step accumulator loop. While the fold is held it does nothing and
    /// leaves the accumulator as it was, so held frames neither add time that
    /// would be caught up after the release nor disturb the interpolation
    /// state. Capped at [`MAX_SUBSTEPS_PER_TICK`] steps
    /// worth of time so a slow frame never spirals into ever more catch-up
    /// steps (each step makes the next frame slower); beyond the cap the sim
    /// runs in slow motion rather than stalling the frame. The per-substep and
    /// per-command spans are `trace` level: see them with
    /// `RUST_LOG=openfo4=trace OPENFO4_LOG_SPANS=1`.
    pub fn tick(&mut self, frame_dt: f32) {
        if self.pipboy.open {
            self.accumulator = 0.0;
            self.steps_last_tick = 0;
            return;
        }
        if self.fold_hold {
            self.steps_last_tick = 0;
            return;
        }
        let max_accumulated = MAX_SUBSTEPS_PER_TICK as f32 * FIXED_DT;
        let wanted = self.accumulator + frame_dt.max(0.0);
        let dropped = wanted - max_accumulated;
        if dropped > f32::EPSILON && !self.warned_substep_clamp {
            self.warned_substep_clamp = true;
            tracing::warn!(
                "sim: frame took {:.1} ms; clamping to {MAX_SUBSTEPS_PER_TICK} fixed steps per frame (physics slows down while frames are this slow; logged once)",
                frame_dt * 1000.0
            );
        }
        self.accumulator = wanted.min(max_accumulated);
        let substeps_span =
            tracing::info_span!("sim_substeps", steps = tracing::field::Empty).entered();
        let mut steps = 0u32;
        while self.accumulator >= FIXED_DT {
            self.accumulator -= FIXED_DT;
            // Fold collider changes into the broad-phase BVH before the
            // character controller queries. Clean ticks skip the pipeline
            // entirely — with a static-only world there is nothing to
            // simulate, and stepping 200k+ static colliders per tick was a
            // frame killer (the broad phase re-optimizes its tree per step).
            self.physics.update_water(self.player.pos);
            let needs_step = {
                let _span = tracing::trace_span!("sim_needs_step").entered();
                self.physics.needs_step()
            };
            if needs_step {
                self.physics.step();
            }
            {
                let _span = tracing::trace_span!("sim_player_step").entered();
                player::step_player(&mut self.player, &self.input, &mut self.physics, FIXED_DT);
            }
            self.step_footsteps();
            steps += 1;
        }
        substeps_span.record("steps", steps);
        self.steps_last_tick = steps;
    }

    fn step_footsteps(&mut self) {
        let player = &self.player;
        let displacement = (player.pos - player.prev_pos) * Vec3::new(1.0, 0.0, 1.0);
        let observation = TickObservation {
            completed: player.step_completed,
            grounded: player.grounded,
            crouching: player.crouching,
            sprint: self.input.sprint,
            horizontal_displacement: displacement.length(),
            launched: player.launched,
        };
        let output = self.footstep_clock.update(&observation);
        let Some(gait) = output.gait.filter(|_| !output.tags.is_empty()) else {
            return;
        };
        let surface = self.surface_under_feet();
        self.footsteps
            .extend(
                output
                    .tags
                    .into_iter()
                    .map(|tag| FootstepEvent { tag, gait, surface }),
            );
    }

    /// What the soles stand on, sampled once per call.
    fn surface_under_feet(&self) -> SurfaceMaterial {
        let (origin, reach) = self.player.sole_probe();
        self.physics.ground_surface(origin, reach)
    }

    /// First-person eye position, interpolated between the last two fixed
    /// steps. `None` until the player has spawned onto the ground, and again
    /// while a teleport waits for its destination collision: returning the
    /// stale pre-teleport position would snap the camera (and with it the
    /// streamer) back to the old cell, unloading the very colliders the
    /// pending ground-snap is waiting on.
    pub fn player_eye(&self) -> Option<Vec3> {
        if !self.player.spawned || self.player.pending_spawn.is_some() {
            return None;
        }
        let center = self
            .player
            .prev_pos
            .lerp(self.player.pos, self.interp_alpha());
        Some(center + Vec3::Y * self.player.eye_above_center())
    }

    /// The interpolated body of the player; `None` under the same conditions as
    /// [`player_eye`](Self::player_eye).
    pub fn player_body(&self) -> Option<PlayerBody> {
        let eye = self.player_eye()?;
        let height = self.player.height();
        let feet = eye - Vec3::Y * (self.player.eye_above_center() + height * 0.5);
        Some(PlayerBody {
            feet,
            height,
            radius: player::RADIUS,
        })
    }

    /// Moves the bodies' water entries since the last call into `out`
    /// (engine-space body origins).
    pub fn drain_water_entries(&mut self, out: &mut Vec<Vec3>) {
        self.physics.drain_water_entries(out);
    }

    /// Render-frame interpolation factor between the last two fixed steps
    /// (render outruns the 60 Hz tick). Shared by the player eye and the
    /// dynamic-body pose readback so both interpolate identically.
    fn interp_alpha(&self) -> f32 {
        (self.accumulator / FIXED_DT).clamp(0.0, 1.0)
    }

    /// Interpolated poses of every simulating dynamic body, for the renderer to
    /// write into the instance buffer. Read-only (`&self`), glam-typed — it must
    /// never feed back into the sim. Empty when nothing is simulating.
    pub fn dynamic_poses(&self, out: &mut Vec<DynamicPose>) {
        out.clear();
        self.physics.dynamic_poses(self.interp_alpha(), out);
    }

    pub fn ragdoll_settings(&self) -> RagdollSettings {
        self.physics.ragdoll_settings()
    }

    /// Body motions of the ragdolls whose pose changed since the last call.
    pub fn ragdoll_poses(&mut self, out: &mut RagdollPoses) {
        self.physics.ragdoll_poses(out);
    }

    /// Publishes a ragdoll's pose again on the next
    /// [`ragdoll_poses`](Self::ragdoll_poses) when it is not at rest (moved, or
    /// authored by the REFR): for a consumer that learned of the placement after
    /// the sim first published it. No-op for any other entity.
    pub fn republish_ragdoll(&mut self, entity: EntityId) {
        self.physics.republish_ragdoll(entity);
    }

    /// Whether the sim publishes a pose for this entity instead of leaving it at
    /// rest: the REFR's authored pose, or where its ragdoll last came to rest
    /// (it respawns there when its cell returns).
    pub fn ragdoll_has_pose(&self, entity: EntityId) -> bool {
        self.physics.ragdoll_has_pose(entity)
    }

    /// The renderer cannot skin `model_path` (the `ObjectColliderGroup::path` of
    /// the placements): they all revert to static collision and the model is
    /// never a ragdoll again this session.
    pub fn disable_ragdoll_model(&mut self, model_path: &str) {
        self.physics.disable_ragdoll_model(model_path);
    }

    pub fn player_debug(&self) -> PlayerDebug {
        let cell = world_to_grid(self.player.pos);
        PlayerDebug {
            pos: self.player.pos,
            grounded: self.player.grounded,
            vertical_vel: self.player.vertical_vel,
            crouching: self.player.crouching,
            spawned: self.player.spawned,
            waiting_for_ground: self.player.pending_spawn.is_some(),
            cell,
            cell_ready: self.physics.spawn_ready(cell),
        }
    }

    /// Read-only HUD feed for the native overlay (HP/AP/crosshair). Mirrors
    /// [`player_debug`](Self::player_debug): gameplay values are derived here,
    /// the render layer reads only final numbers. The crosshair carries the
    /// movement-bloom radius; the render layer may override the state to
    /// `Activate` from the door-aim ray (the sim is camera-agnostic).
    pub fn hud_snapshot(&self) -> HudSnapshot {
        let (hp, hp_max) = self.player.health();
        let (ap, ap_max) = self.player.action_points();
        // Unarmed (there is no weapon system yet) → FO4's small Dot reticle.
        // The Standard 4-tick spread reticle is the armed state and returns when
        // weapons land; the render layer overrides this to Activate on door-aim.
        let crosshair = if self.player.spawned {
            CrosshairState::Dot
        } else {
            CrosshairState::Hidden
        };
        HudSnapshot {
            visible: self.player.spawned,
            hp: MeterFill::new(hp, hp_max),
            ap: MeterFill::new(ap, ap_max),
            crosshair,
        }
    }

    /// Installs the Pip-Boy definitions loaded at startup and builds the model over them.
    pub fn set_pipboy_catalog(&mut self, catalog: Arc<pipboy::PipBoyCatalog>) {
        self.pipboy = pipboy::PipBoyModel::new(catalog);
    }

    /// Read-only Pip-Boy feed for the render layer, valid for this frame.
    pub fn pipboy_snapshot(&self) -> pipboy::PipBoySnapshot<'_> {
        let (hp, hp_max) = self.player.health();
        let (ap, ap_max) = self.player.action_points();
        let position = if self.physics.interior_mode() {
            None
        } else {
            self.player_body().map(|body| body.feet)
        };
        pipboy::PipBoySnapshot {
            model: &self.pipboy,
            clock: self.world_clock.date(),
            vitals: pipboy::PlayerVitals {
                hp,
                hp_max,
                ap,
                ap_max,
                position,
                heading_yaw: self.input.yaw,
            },
        }
    }

    pub fn stats(&self) -> SimStats {
        SimStats {
            collider_count: self.physics.collider_count(),
            cells_with_colliders: self.physics.cell_count(),
            last_step_micros: self.physics.last_step_micros,
            steps_last_tick: self.steps_last_tick,
            active_dynamic: self.physics.active_dynamic_count(),
            dyn_total: self.physics.dynamic_body_count(),
        }
    }

    /// Generation of the queryable collider set: moves when a physics step
    /// folds collider changes into the broad phase. The debug wireframe
    /// rebuilds when this differs from its cached value.
    pub fn collider_generation(&self) -> u64 {
        self.physics.collider_generation()
    }

    /// The per-frame fold after streamer commands. With
    /// deferral (the default; `OPENFO4_SIM_FOLD_DEFER=0` opts out) it is skipped:
    /// the collider set stays dirty and the next fixed `tick` refreshes the
    /// broad phase inside its regular step (`needs_step` is true while dirty)
    /// before any player query, so several frames of streamed cells share one
    /// refresh. Only the debug wireframe and the aim rays (door activation,
    /// debug readout) lag, by at most one tick. They stay correct: a ray
    /// resolves the hit leaf's handle to the collider currently in that slot
    /// and tests its real shape, so a stale leaf over a reused slot can at
    /// worst miss a just-streamed collider for a tick, never report a wrong one.
    pub fn flush_colliders_per_frame(&mut self) {
        if fold_deferral_enabled() {
            return;
        }
        self.flush_colliders();
    }

    /// Fold pending collider changes into the broad phase without running a
    /// fixed tick. The render side calls this every frame after applying
    /// streamer commands: a frame that accumulates less than one fixed
    /// step runs no substep and folds nothing, and without the fold AABB
    /// queries (the collision wireframe) read a frozen BVH — streamed cells
    /// never appear, and stale leaves can alias reused collider slots. A clean
    /// frame is a no-op. While the fold is held this does nothing at all, even
    /// for an explicit teleport fold: the next unheld `tick` folds every
    /// pending change in one pass before the first player query.
    pub fn flush_colliders(&mut self) {
        if self.fold_hold {
            return;
        }
        // Fold pending collider changes into the broad phase WITHOUT advancing
        // dynamics — the fixed `tick` owns time advancement. A full `step()`
        // here would double-step awake clutter on every streaming frame; the
        // zero-dt `fold` refreshes the BVH only.
        self.physics.fold();
    }

    /// World-space line segments (consecutive point pairs) outlining every
    /// collider within `half_extent` of `center` — the collision wireframe
    /// overlay's geometry source. The bool is true when the output was
    /// truncated at the segment cap.
    pub fn debug_wireframe(&self, center: Vec3, half_extent: f32) -> (Vec<Vec3>, bool) {
        self.physics.wireframe_lines(center, half_extent)
    }

    /// What the ray from `origin` along `dir` hits first: mesh path (or
    /// "terrain"), REFR form key, and distance. Debug-UI aim attribution —
    /// turns "stuck on nothing" into a mesh name.
    pub fn debug_ray_hit(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<RayHit> {
        self.physics.ray_attribution(origin, dir, max_dist)
    }

    /// The first surface straight below `origin` within `max_dist`.
    pub fn ground_point_below(&self, origin: Vec3, max_dist: f32) -> Option<Vec3> {
        let hit = self.debug_ray_hit(origin, Vec3::NEG_Y, max_dist)?;
        Some(origin - Vec3::Y * hit.dist)
    }
}

fn apply_collider_work(physics: &mut physics::PhysicsContext, work: Deferred<CellPayload>) {
    match work {
        Deferred::AddCell {
            grid,
            payload: (batch, baked),
        } => {
            let _span = tracing::trace_span!("sim_add_cell", gx = grid.0, gy = grid.1).entered();
            physics.add_cell(grid, &batch, baked.as_ref());
        }
        Deferred::PhysicsCenter { grid, radius } => {
            let _span = tracing::trace_span!("sim_set_physics_center").entered();
            physics.set_physics_center(grid, radius);
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use glam::Vec2;

    use super::*;

    #[test]
    fn entity_ids_are_the_fnv1a_64_hash_of_the_form_key() {
        // Reference vectors of FNV-1a 64: the empty string is the offset basis, "a" is 0xaf63dc4c8601ec8c.
        assert_eq!(refr_entity_id("a"), EntityId(0xaf63_dc4c_8601_ec8c));
        assert_eq!(refr_entity_id(""), EntityId(0xcbf2_9ce4_8422_2325));
    }

    #[test]
    fn entity_ids_differ_per_form_key_and_never_hit_the_reserved_values() {
        let keys = [
            "001ED5:Fallout4.esm",
            "001ED6:Fallout4.esm",
            "001ED5:DLCRobot.esm",
        ];
        let ids: Vec<EntityId> = keys.iter().map(|k| refr_entity_id(k)).collect();
        assert_ne!(ids[0], ids[1]);
        assert_ne!(ids[0], ids[2]);
        assert!(ids.iter().all(|id| id.0 >= 2 && *id != PLAYER_ENTITY));
    }

    #[test]
    fn world_to_grid_floors_towards_negative_infinity() {
        assert_eq!(world_to_grid(Vec3::new(0.0, 5.0, 0.0)), (0, 0));
        assert_eq!(
            world_to_grid(Vec3::new(
                CELL_SIZE_WORLD - 0.1,
                0.0,
                -(CELL_SIZE_WORLD - 0.1)
            )),
            (0, 0)
        );
        assert_eq!(
            world_to_grid(Vec3::new(CELL_SIZE_WORLD, 0.0, -CELL_SIZE_WORLD)),
            (1, 1)
        );
        assert_eq!(world_to_grid(Vec3::new(-0.1, 0.0, 0.0)), (-1, 0));
    }

    #[test]
    fn engine_minus_z_is_the_positive_grid_y_axis() {
        assert_eq!(world_to_grid(Vec3::new(0.0, 0.0, 10.0)).1, -1);
        assert_eq!(world_to_grid(Vec3::new(0.0, 0.0, -10.0)).1, 0);
    }

    #[test]
    fn one_fixed_step_of_time_runs_exactly_one_step() {
        let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
        sim.tick(FIXED_DT);
        assert_eq!(sim.stats().steps_last_tick, 1);
    }

    #[test]
    fn partial_steps_accumulate_across_ticks() {
        let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
        sim.tick(FIXED_DT * 0.6);
        assert_eq!(sim.stats().steps_last_tick, 0);
        sim.tick(FIXED_DT * 0.6);
        assert_eq!(sim.stats().steps_last_tick, 1);
    }

    #[test]
    fn a_long_frame_is_clamped_to_the_substep_cap() {
        let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
        sim.tick(10.0);
        assert_eq!(sim.stats().steps_last_tick, MAX_SUBSTEPS_PER_TICK);
        sim.tick(0.0);
        assert_eq!(
            sim.stats().steps_last_tick,
            0,
            "the dropped time is not caught up later"
        );
    }

    #[test]
    fn negative_frame_time_runs_no_steps() {
        let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
        sim.tick(-1.0);
        assert_eq!(sim.stats().steps_last_tick, 0);
    }

    fn pipboy(sim: &mut SimWorld, action: pipboy::PipBoyAction) {
        sim.apply(SimCommand::PipBoy(action));
    }

    #[test]
    fn no_steps_run_while_the_pipboy_is_open() {
        let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
        pipboy(&mut sim, pipboy::PipBoyAction::Open);
        sim.tick(FIXED_DT * 3.0);
        assert_eq!(sim.stats().steps_last_tick, 0);
    }

    #[test]
    fn closing_the_pipboy_resumes_stepping_without_a_catch_up_burst() {
        let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
        pipboy(&mut sim, pipboy::PipBoyAction::Open);
        sim.tick(5.0);
        pipboy(&mut sim, pipboy::PipBoyAction::Close);
        sim.tick(FIXED_DT);
        assert_eq!(sim.stats().steps_last_tick, 1);
    }

    #[test]
    fn opening_the_pipboy_clears_the_latched_movement_input() {
        let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
        sim.apply(SimCommand::SetPlayerInput(PlayerInput {
            move_dir: Vec2::Y,
            jump: true,
            sprint: true,
            yaw: 1.5,
            ..PlayerInput::default()
        }));
        pipboy(&mut sim, pipboy::PipBoyAction::Open);
        assert_eq!(sim.input.move_dir, Vec2::ZERO);
        assert!(!sim.input.jump && !sim.input.sprint);
        assert_eq!(sim.input.yaw, 1.5, "the camera heading is kept");
    }

    #[test]
    fn a_rejected_pipboy_action_is_recorded_on_the_snapshot_model() {
        let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
        pipboy(&mut sim, pipboy::PipBoyAction::UseFirstStimpak);
        assert_eq!(
            sim.pipboy_snapshot().model.last_rejection,
            Some(pipboy::PipBoyActionError::NoneLeft)
        );
    }

    #[test]
    fn the_snapshot_reports_full_vitals_the_heading_and_no_position_before_spawn() {
        let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
        sim.apply(SimCommand::SetPlayerInput(PlayerInput {
            yaw: 0.75,
            ..PlayerInput::default()
        }));
        let vitals = sim.pipboy_snapshot().vitals;
        assert!(vitals.position.is_none());
        assert_eq!(vitals.hp, vitals.hp_max);
        assert_eq!(vitals.ap, vitals.ap_max);
        assert_eq!(vitals.heading_yaw, 0.75);
    }

    fn sim_with_stimpaks(heal: f32, count: u32) -> SimWorld {
        let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
        let mut model = pipboy::PipBoyModel::new(Arc::default());
        model.inventory.insert(pipboy::InventoryItem {
            id: pipboy::ItemId(0),
            form_key: Some(pipboy::actions::STIMPAK_FORM_KEY.to_owned()),
            name: "Stimpak".to_owned(),
            category: pipboy::InventoryCategory::Aid,
            count,
            weight: 0.5,
            value: 48,
            equipped: false,
            favourite: None,
            legendary: false,
            details: pipboy::inventory::ItemDetails::Aid(vec![pipboy::inventory::AidEffect {
                description: format!("Restores {heal} hit points"),
                restores_hp: heal,
                duration_seconds: None,
            }]),
        });
        sim.pipboy = model;
        sim
    }

    #[test]
    fn using_a_healing_item_restores_the_wounded_players_hit_points() {
        let mut sim = sim_with_stimpaks(40.0, 2);
        let hp_max = sim.player.health().1;
        sim.player.set_health(hp_max - 100.0);
        pipboy(&mut sim, pipboy::PipBoyAction::UseFirstStimpak);
        assert_eq!(sim.pipboy_snapshot().vitals.hp, hp_max - 60.0);
        assert_eq!(sim.pipboy.inventory.items[0].count, 1);
    }

    #[test]
    fn healing_items_never_push_hit_points_past_the_maximum() {
        let mut sim = sim_with_stimpaks(60.0, 1);
        let hp_max = sim.player.health().1;
        sim.player.set_health(hp_max - 25.0);
        pipboy(&mut sim, pipboy::PipBoyAction::UseFirstStimpak);
        assert_eq!(sim.pipboy_snapshot().vitals.hp, hp_max);
        assert_eq!(sim.pipboy.inventory.items, []);
    }

    #[test]
    fn a_rejected_use_heals_nothing() {
        let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
        let hp_max = sim.player.health().1;
        sim.player.set_health(hp_max - 50.0);
        pipboy(&mut sim, pipboy::PipBoyAction::UseFirstStimpak);
        assert_eq!(sim.pipboy_snapshot().vitals.hp, hp_max - 50.0);
    }

    #[test]
    fn player_input_sent_while_the_pipboy_is_open_keeps_the_heading_but_not_the_movement() {
        let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
        pipboy(&mut sim, pipboy::PipBoyAction::Open);
        sim.apply(SimCommand::SetPlayerInput(PlayerInput {
            move_dir: Vec2::Y,
            jump: true,
            sprint: true,
            yaw: 2.0,
            ..PlayerInput::default()
        }));
        assert_eq!(sim.input.move_dir, Vec2::ZERO);
        assert!(!sim.input.jump && !sim.input.sprint);
        assert_eq!(sim.input.yaw, 2.0);
    }

    #[test]
    fn player_input_is_applied_again_once_the_pipboy_closed() {
        let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
        pipboy(&mut sim, pipboy::PipBoyAction::Open);
        pipboy(&mut sim, pipboy::PipBoyAction::Close);
        sim.apply(SimCommand::SetPlayerInput(PlayerInput {
            move_dir: Vec2::Y,
            ..PlayerInput::default()
        }));
        assert_eq!(sim.input.move_dir, Vec2::Y);
    }

    mod collider_queue {
        use super::*;
        use crate::sim::collision::{CellColliderBatch, PartShape};
        use crate::sim::physics::test_support::{
            bake, box_shape, ground_batch, object_group, placement,
        };

        const SPAWN_AT: Vec3 = Vec3::new(200.0, 3.0, -100.0);
        const SPAWN_GRID: CellGrid = (0, 0);

        fn add_ground(sim: &mut SimWorld, grid: CellGrid) {
            sim.apply(SimCommand::AddCellColliders {
                grid,
                batch: Arc::new(ground_batch()),
                baked: None,
            });
        }

        fn held_sim() -> SimWorld {
            let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
            sim.set_fold_hold(true);
            sim
        }

        fn inserted_cells(sim: &SimWorld) -> usize {
            sim.stats().cells_with_colliders
        }

        #[test]
        fn without_a_hold_and_with_an_empty_queue_an_add_is_inserted_at_once() {
            let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
            add_ground(&mut sim, SPAWN_GRID);
            assert_eq!(inserted_cells(&sim), 1);
            assert!(!sim.has_pending_colliders());
        }

        #[test]
        fn while_held_an_add_is_queued_until_drained() {
            let mut sim = held_sim();
            add_ground(&mut sim, SPAWN_GRID);
            assert!(sim.has_pending_colliders());
            assert_eq!(inserted_cells(&sim), 0);
            sim.drain_pending_colliders(COLLIDER_APPLY_BUDGET);
            assert_eq!(inserted_cells(&sim), 1);
            assert!(!sim.has_pending_colliders());
        }

        #[test]
        fn a_drain_inserts_at_least_one_cell_per_call_in_queue_order() {
            let mut sim = held_sim();
            for grid in [(0, 0), (1, 0), (2, 0)] {
                add_ground(&mut sim, grid);
            }
            sim.drain_pending_colliders(Duration::ZERO);
            assert_eq!(inserted_cells(&sim), 1);
            assert!(sim.physics.spawn_ready((0, 0)) && !sim.physics.spawn_ready((1, 0)));
            sim.drain_pending_colliders(Duration::ZERO);
            assert!(sim.physics.spawn_ready((1, 0)) && !sim.physics.spawn_ready((2, 0)));
            sim.drain_pending_colliders(Duration::MAX);
            assert_eq!(inserted_cells(&sim), 3);
            assert!(!sim.has_pending_colliders());
        }

        #[test]
        fn removing_a_queued_cell_cancels_its_insert() {
            let mut sim = held_sim();
            add_ground(&mut sim, (0, 0));
            add_ground(&mut sim, (1, 0));
            sim.apply(SimCommand::RemoveCellColliders { grid: (0, 0) });
            sim.drain_pending_colliders(Duration::MAX);
            assert!(!sim.physics.spawn_ready((0, 0)));
            assert!(sim.physics.spawn_ready((1, 0)));
        }

        #[test]
        fn removing_a_resident_cell_still_removes_it_while_held() {
            let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
            add_ground(&mut sim, (0, 0));
            sim.set_fold_hold(true);
            sim.apply(SimCommand::RemoveCellColliders { grid: (0, 0) });
            assert_eq!(inserted_cells(&sim), 0);
        }

        #[test]
        fn an_add_never_overtakes_work_still_queued_after_the_hold_is_released() {
            let mut sim = held_sim();
            add_ground(&mut sim, (0, 0));
            add_ground(&mut sim, (1, 0));
            sim.drain_pending_colliders(Duration::ZERO);
            sim.set_fold_hold(false);
            add_ground(&mut sim, (2, 0));
            assert_eq!(
                inserted_cells(&sim),
                1,
                "the new add waits behind the queued one"
            );
            sim.drain_pending_colliders(Duration::MAX);
            assert_eq!(inserted_cells(&sim), 3);
        }

        #[test]
        fn a_physics_centre_move_runs_after_the_adds_queued_before_it() {
            let mut batch: CellColliderBatch = ground_batch();
            batch.objects.push(object_group(
                "meshes\\crate.glb",
                vec![placement(
                    "0A0000:Fallout4.esm",
                    Vec3::new(200.0, 1.0, -100.0),
                )],
                true,
                Some(10.0),
            ));
            let shapes: Vec<(&str, Vec<PartShape>)> =
                vec![("meshes\\crate.glb", vec![box_shape(0.5)])];
            let baked = bake(&batch, &shapes);
            let mut sim = held_sim();
            sim.apply(SimCommand::AddCellColliders {
                grid: (0, 0),
                batch: Arc::new(batch),
                baked: Some(baked),
            });
            sim.apply(SimCommand::SetPhysicsCenter {
                grid: (0, 0),
                radius: 2,
            });
            assert_eq!(sim.stats().active_dynamic, 0);
            sim.drain_pending_colliders(Duration::MAX);
            assert_eq!(sim.stats().dyn_total, 1);
            assert_eq!(
                sim.stats().active_dynamic,
                1,
                "the centre reconcile saw the body its add created"
            );
        }

        #[test]
        fn a_hold_stops_the_fixed_steps_and_release_resumes_them() {
            let mut sim = held_sim();
            sim.tick(FIXED_DT);
            assert_eq!(sim.stats().steps_last_tick, 0);
            sim.set_fold_hold(false);
            sim.tick(FIXED_DT);
            assert_eq!(sim.stats().steps_last_tick, 1);
        }

        #[test]
        fn time_that_passes_during_a_hold_is_not_caught_up_after_release() {
            let mut sim = held_sim();
            sim.tick(1.0);
            sim.set_fold_hold(false);
            sim.tick(0.0);
            assert_eq!(sim.stats().steps_last_tick, 0);
        }

        #[test]
        fn a_held_tick_leaves_the_sub_step_remainder_untouched() {
            let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
            sim.tick(FIXED_DT * 0.5);
            let before = sim.interp_alpha();
            assert!((before - 0.5).abs() < 1e-4, "{before}");
            sim.set_fold_hold(true);
            sim.tick(FIXED_DT * 3.0);
            sim.set_fold_hold(false);
            assert_eq!(sim.interp_alpha(), before);
            sim.tick(FIXED_DT * 0.5);
            assert_eq!(
                sim.stats().steps_last_tick,
                1,
                "the remainder completes a step"
            );
        }

        #[test]
        fn discarding_drops_queued_adds_and_centre_moves_of_the_world_being_left() {
            let mut sim = held_sim();
            add_ground(&mut sim, (0, 0));
            sim.apply(SimCommand::SetPhysicsCenter {
                grid: (0, 0),
                radius: 2,
            });
            sim.discard_pending_colliders();
            assert!(!sim.has_pending_colliders());
            sim.drain_pending_colliders(Duration::MAX);
            assert_eq!(inserted_cells(&sim), 0);
        }

        #[test]
        fn after_a_discard_and_a_release_adds_are_inserted_at_once_again() {
            let mut sim = held_sim();
            add_ground(&mut sim, (0, 0));
            sim.discard_pending_colliders();
            sim.set_fold_hold(false);
            add_ground(&mut sim, (7, 7));
            assert_eq!(inserted_cells(&sim), 1);
            assert!(!sim.has_pending_colliders());
        }

        #[test]
        fn a_hold_defers_the_broad_phase_fold_to_the_first_tick_after_release() {
            let mut sim = held_sim();
            add_ground(&mut sim, SPAWN_GRID);
            sim.drain_pending_colliders(Duration::MAX);
            let before = sim.collider_generation();
            sim.tick(FIXED_DT);
            sim.flush_colliders();
            assert_eq!(
                sim.collider_generation(),
                before,
                "nothing folded while held"
            );
            sim.set_fold_hold(false);
            sim.tick(FIXED_DT);
            assert_ne!(
                sim.collider_generation(),
                before,
                "release folds in one pass"
            );
        }

        #[test]
        fn the_player_spawns_only_after_the_hold_is_released_and_the_collision_is_installed() {
            let mut sim = held_sim();
            sim.apply(SimCommand::SpawnPlayer { pos: SPAWN_AT });
            add_ground(&mut sim, SPAWN_GRID);
            sim.drain_pending_colliders(Duration::MAX);
            for _ in 0..5 {
                sim.tick(FIXED_DT);
            }
            assert!(
                !sim.player_debug().spawned,
                "held: collision installed but no step"
            );
            assert!(sim.player_eye().is_none());
            sim.set_fold_hold(false);
            sim.tick(FIXED_DT);
            sim.tick(FIXED_DT);
            let player = sim.player_debug();
            assert!(player.spawned && player.grounded);
            assert!(player.pos.y > 0.0, "stands on the ground: {}", player.pos.y);
        }

        #[test]
        fn a_release_with_collision_still_queued_keeps_the_player_waiting_instead_of_falling() {
            let mut sim = held_sim();
            sim.apply(SimCommand::SpawnPlayer { pos: SPAWN_AT });
            add_ground(&mut sim, SPAWN_GRID);
            sim.set_fold_hold(false);
            for _ in 0..5 {
                sim.tick(FIXED_DT);
            }
            let waiting = sim.player_debug();
            assert!(!waiting.spawned && waiting.waiting_for_ground);
            sim.drain_pending_colliders(Duration::MAX);
            sim.tick(FIXED_DT);
            sim.tick(FIXED_DT);
            assert!(sim.player_debug().spawned);
        }
    }

    #[test]
    fn there_is_no_eye_and_no_hud_before_the_player_spawns() {
        let sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
        assert!(sim.player_eye().is_none());
        let hud = sim.hud_snapshot();
        assert!(!hud.visible);
        assert_eq!(hud.crosshair, CrosshairState::Hidden);
    }

    mod world_clock_commands {
        use super::*;
        use commands::ClockEdit;

        fn edit(sim: &mut SimWorld, edit: ClockEdit) {
            sim.apply(SimCommand::SetWorldClock(edit));
        }

        fn frame(sim: &mut SimWorld, in_game: bool, dt: f32) {
            let world_dt = sim.world_dt(dt);
            sim.advance_world_clock(ClockFrame { in_game, world_dt });
        }

        /// Whole quarter-second in-game frames, as exact in f32 as the step itself.
        fn run_seconds(sim: &mut SimWorld, seconds: f32) {
            for _ in 0..(seconds / 0.25) as u32 {
                frame(sim, true, 0.25);
            }
        }

        fn minutes_advanced_by(sim: &mut SimWorld, step: impl FnOnce(&mut SimWorld)) -> f64 {
            let before = sim.world_clock().minute_of_day();
            step(sim);
            sim.world_clock().minute_of_day() - before
        }

        #[test]
        fn one_in_game_frame_advances_the_clock_exactly_once() {
            let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
            let advanced = minutes_advanced_by(&mut sim, |sim| frame(sim, true, 0.15));
            assert!((advanced - 0.15 * 20.0 / 60.0).abs() < 1e-6, "{advanced}");
        }

        #[test]
        fn a_five_second_hitch_advances_a_quarter_second_of_game_time_at_scale_20() {
            let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
            let advanced = minutes_advanced_by(&mut sim, |sim| frame(sim, true, 5.0));
            let quarter_second = 0.25 * 20.0 / 60.0;
            assert!((advanced - quarter_second).abs() < 1e-6, "{advanced}");
            assert!(advanced < 5.0 * 20.0 / 60.0 / 10.0);
        }

        #[test]
        fn outside_gameplay_the_clock_stands_still() {
            let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
            assert_eq!(
                minutes_advanced_by(&mut sim, |sim| frame(sim, false, 0.1)),
                0.0
            );
        }

        #[test]
        fn a_held_load_stops_the_clock_until_released() {
            let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
            sim.set_fold_hold(true);
            assert_eq!(
                minutes_advanced_by(&mut sim, |sim| frame(sim, true, 0.1)),
                0.0
            );
            sim.set_fold_hold(false);
            assert!(minutes_advanced_by(&mut sim, |sim| frame(sim, true, 0.1)) > 0.0);
        }

        #[test]
        fn an_open_pipboy_stops_the_clock_and_the_world_time_of_the_frame() {
            let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
            pipboy(&mut sim, pipboy::PipBoyAction::Open);
            assert_eq!(sim.world_dt(0.1), 0.0);
            assert_eq!(
                minutes_advanced_by(&mut sim, |sim| frame(sim, true, 0.1)),
                0.0
            );
            pipboy(&mut sim, pipboy::PipBoyAction::Close);
            assert_eq!(sim.world_dt(0.1), 0.1);
            assert!(minutes_advanced_by(&mut sim, |sim| frame(sim, true, 0.1)) > 0.0);
        }

        #[test]
        fn a_negative_or_nan_frame_time_does_not_move_the_clock() {
            let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
            for dt in [-1.0, f32::NAN] {
                assert_eq!(
                    minutes_advanced_by(&mut sim, |sim| frame(sim, true, dt)),
                    0.0
                );
            }
        }

        #[test]
        fn setting_the_hour_moves_the_clock_the_pipboy_shows() {
            let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
            edit(&mut sim, ClockEdit::SetHour(3.5));
            assert_eq!(sim.world_clock().hour(), 3.5);
            assert_eq!(sim.pipboy_snapshot().clock.minute_of_day, 3 * 60 + 30);
        }

        #[test]
        fn an_invalid_hour_is_rejected_and_the_clock_stays() {
            let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
            let before = sim.world_clock().clone();
            edit(&mut sim, ClockEdit::SetHour(f32::NAN));
            edit(&mut sim, ClockEdit::SetHour(30.0));
            assert_eq!(*sim.world_clock(), before);
        }

        #[test]
        fn pausing_stops_the_advance_and_resuming_restarts_it() {
            let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
            let start = sim.world_clock().minute_of_day();
            edit(&mut sim, ClockEdit::SetPaused(true));
            run_seconds(&mut sim, 60.0);
            assert_eq!(sim.world_clock().minute_of_day(), start);
            edit(&mut sim, ClockEdit::SetPaused(false));
            run_seconds(&mut sim, 60.0);
            assert!((sim.world_clock().minute_of_day() - (start + 20.0)).abs() < 1e-6);
        }

        #[test]
        fn a_time_scale_edit_changes_the_rate_and_an_invalid_one_keeps_it() {
            let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
            let start = sim.world_clock().minute_of_day();
            edit(&mut sim, ClockEdit::SetTimeScale(60.0));
            edit(&mut sim, ClockEdit::SetTimeScale(0.0));
            run_seconds(&mut sim, 60.0);
            assert_eq!(sim.world_clock().minute_of_day(), start + 60.0);
        }

        #[test]
        fn the_sim_runs_the_clock_it_was_given() {
            let clock = world_clock::WorldClock::new(world_clock::ClockStart {
                hour: 6.0,
                ..world_clock::ClockStart::fixture()
            })
            .unwrap();
            let sim = SimWorld::new(PlayerMovement::default(), clock);
            assert_eq!(sim.world_clock().hour(), 6.0);
        }
    }
}
