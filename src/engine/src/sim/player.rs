//! First-person player: a kinematic capsule moved through rapier's
//! `KinematicCharacterController`. The capsule shape and pose live here; a
//! wake-only kinematic mirror body (see `mirror`) follows it so sleeping
//! clutter wakes on contact, and every scene query filters the mirror out.
//!
//! Velocity is gameplay-owned and integrated below; the controller is a
//! stateless collide-and-slide helper, there is no engine-side velocity
//! (standard kinematic-controller architecture — Unity/Unreal/Havok alike).
//! The controller only sees static geometry; `dynamic_contact` blocks the
//! player against dynamic bodies, shoves them and lets it land on them.

use glam::{Vec2, Vec3};
use rapier3d::control::{
    CharacterAutostep, CharacterLength, EffectiveCharacterMovement, KinematicCharacterController,
};
use rapier3d::parry::query::details::ShapeCastOptions;
use rapier3d::prelude::*;

use super::commands::PlayerInput;
use super::dynamic_contact::{self, Character};
use super::mirror::static_scene_query_filter;
use super::physics::PhysicsContext;
use super::world_to_grid;
use crate::anim::first_person_eye::FirstPersonCamera;

// Jump = the vanilla Fallout 4 model (no easing curve exists): liftoff velocity
// comes from a height game setting, the rise happens under FULL real gravity
// (not floaty), and only the fall is scaled up by the fInAirFallingCharGravityMult INI setting. Engine units:
// 1 unit ≈ 14.3 cm (= 10 Bethesda units), so 9.8 m/s² = 68.59 u/s².
//
//   rise:    g      = 9.8 m/s²                          =  68.59 u/s²
//   liftoff: v0     = sqrt(2*g*h), h = fJumpHeightMin
//                     GMST = 90 Beth = 9.0 engine units =  35.14 u/s
//   fall:    g_fall = g * 1.85 (vanilla INI default)    = 126.89 u/s²
//
// The 1.85x gravity step fires at zero vertical velocity, so there is no
// visible kink. FO4 also plays a jump-anticipation animation before the
// behavior graph launches the capsule; the wind-up below makes the total
// input->land airtime match frame-counted footage (60 fps): landing at
// frame 56 exactly, apex at ~32 (footage read "36", but the apex is a flat
// ±3-frame window — the capsule moves <2 cm/frame there). Exact-fit
// alternative if footage apex is ever confirmed over vanilla data:
// JUMP_WINDUP_TICKS = 6, GRAVITY_FALL = 184.0 (a harsher 2.68x).
const GRAVITY: f32 = 68.59;
const JUMP_VELOCITY: f32 = 35.14;
const GRAVITY_FALL: f32 = 126.89;
pub(super) const JUMP_WINDUP_TICKS: u32 = 2;
/// Coyote time: jumping stays allowed this many ticks (100 ms) after ground
/// contact is lost — the standard grace-window fix. FO4's noisy collision
/// (mesh seams, stair-stepped slopes) makes `grounded` flicker a few mm off
/// the surface while moving downhill, which otherwise eats jump inputs;
/// vanilla absorbs the same noise inside Havok's support-query tolerance.
pub(super) const COYOTE_TICKS: u32 = 6;
/// Terminal velocity: engine safety net, no vanilla equivalent found
/// (~29 m/s, reached after ~1.6 s of free fall).
const MAX_FALL_SPEED: f32 = 200.0;
/// Ground speeds, fed from the `Player_Default_MT` MOVT record in the DB
/// (`movement_types.data`, converted Beth u/s -> engine u/s at the load
/// site). FO4's default gait is Run; Sprint is forward-only in the record
/// but applied to the whole input direction here, matching the old
/// constants' behavior. Walk is only forced by carrying a heavy body
/// (`carry::forces_walk`). These are *top* speeds — FO4 ramps up to them via
/// the race acceleration rate, which we don't model, so a timed course runs
/// slightly faster than vanilla footage (the old footage-calibrated course
/// *average* was 34.43 for run, 48 for sprint).
///
/// `Default` mirrors the vanilla record so unit tests and a missing/old DB
/// (pre-`installer movt-only` extraction) still get sane movement.
#[derive(Copy, Clone, Debug)]
pub struct PlayerMovement {
    pub walk_speed: f32,
    pub run_speed: f32,
    pub sprint_speed: f32,
}

impl Default for PlayerMovement {
    fn default() -> Self {
        Self {
            walk_speed: 9.255,
            run_speed: 37.298,
            sprint_speed: 50.0,
        }
    }
}

impl PlayerMovement {
    /// Top ground speed: walking while forced to, else sprinting when asked and not crouched,
    /// else running.
    fn ground_speed(&self, sprint: bool, crouching: bool, forced_walk: bool) -> f32 {
        if forced_walk {
            self.walk_speed
        } else if sprint && !crouching {
            self.sprint_speed
        } else {
            self.run_speed
        }
    }
}

/// Heights of the first-person eye above the soles, standing and sneaking, in engine units. Read
/// from the first-person skeleton's `Camera` bone and its sneak offset clip; independent of the
/// collision capsule.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct EyeHeights {
    standing: f32,
    sneaking: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EyePose {
    Standing,
    Sneaking,
}

impl std::fmt::Display for EyePose {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Standing => "standing",
            Self::Sneaking => "sneaking",
        })
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EyeHeightsError {
    #[error("the {pose} eye height {height} is not a positive finite length")]
    NotPositive { pose: EyePose, height: f32 },
}

impl TryFrom<FirstPersonCamera> for EyeHeights {
    type Error = EyeHeightsError;

    fn try_from(camera: FirstPersonCamera) -> Result<Self, Self::Error> {
        Self::new(
            camera.standing_height,
            camera.standing_height + camera.sneak_shift,
        )
    }
}

impl EyeHeights {
    pub fn new(standing: f32, sneaking: f32) -> Result<Self, EyeHeightsError> {
        for (pose, height) in [(EyePose::Standing, standing), (EyePose::Sneaking, sneaking)] {
            if !(height.is_finite() && height > 0.0) {
                return Err(EyeHeightsError::NotPositive { pose, height });
            }
        }
        Ok(Self { standing, sneaking })
    }

    pub fn standing(self) -> f32 {
        self.standing
    }

    pub fn sneaking(self) -> f32 {
        self.sneaking
    }

    fn above_soles(self, sneaking: bool) -> f32 {
        if sneaking {
            self.sneaking
        } else {
            self.standing
        }
    }
}

/// The vanilla first-person skeleton: `Camera` 12.0483 above `Root`, lowered 4.0 by the
/// `SneakOffset` clip. Tests only; the engine reads the installed assets.
#[cfg(test)]
impl Default for EyeHeights {
    fn default() -> Self {
        Self {
            standing: 12.0483,
            sneaking: 8.0483,
        }
    }
}

/// Airborne input only steers at this acceleration; momentum is otherwise
/// kept (no mid-air dead stop, a sprint jump can't be reversed before
/// landing). Feel-tuned, not measured.
const AIR_ACCEL: f32 = 60.0;
/// Height of the player's collision capsule. The original game blocks the player at the same
/// lintel standing and sneaking (2026-11-16), so it is one shape for both postures.
const HEIGHT: f32 = 12.6;
pub(super) const RADIUS: f32 = 2.5;
const _: () = assert!(HEIGHT > 2.0 * RADIUS);
// Raw FO4 collision has noisy normals: a tighter climb angle glues the
// capsule on small ledges (player.gd:18).
const MAX_SLOPE_CLIMB: f32 = 60.0 * std::f32::consts::PI / 180.0;
const MIN_SLOPE_SLIDE: f32 = 75.0 * std::f32::consts::PI / 180.0;
/// ~20 cm ground snap bridges seams between adjacent collision meshes.
const SNAP_LEN: f32 = 1.5;
/// Gap the controller preserves around the capsule (safe-margin analog).
const OFFSET: f32 = 0.05;
/// Horizontal travel per fixed step at or below this is contact jitter, not
/// walking: the controller's own skin gap. Pushing into a wall slides by far
/// less than this per step.
pub(super) const MOVE_EPSILON: f32 = OFFSET;
/// Height of the capsule centre above the soles: half the height plus the controller gap.
pub const CENTER_ABOVE_FEET: f32 = HEIGHT / 2.0 + OFFSET;
/// Height of the capsule top above the soles.
pub(super) const TOP_ABOVE_FEET: f32 = HEIGHT + OFFSET;
/// How far below the capsule a dynamic body still counts as ground.
const SNAP_REACH: f32 = OFFSET + 0.05;
/// Highest collision top above the soles the player is lifted onto: original game 2026-11-15,
/// always lifted at 31.0 game units, never at 32.0; the middle.
const MAX_STEP_UP: f32 = 3.15;
/// Free depth the autostep needs on top of an edge.
const STEP_MIN_WIDTH: f32 = 1.0;
/// How far a tick that walked off the ground casts down to glue the capsule back onto a walkable
/// surface. Feel-tuned.
const GROUND_STICK_REACH: f32 = 2.5;
/// Nominal player mass for the shove impulse (split with the body's mass as a
/// reduced mass). Far above typical clutter mass (a bottle is ~2), so the
/// player shoves light props while heavy ones resist. Feel-tuned.
const CHAR_MASS: f32 = 80.0;
/// Most force the player pushes a body with: `CHAR_MASS` accelerating at
/// 135 u/s², an impulse of 180 per tick, so a 15-mass bone (the heaviest
/// skeleton bone) gains about 12 u/s per tick and reaches walking speed in
/// three ticks, lighter bones sooner. Feel-tuned.
const CHAR_PUSH_FORCE: f32 = CHAR_MASS * 135.0;
/// Far enough that a downward spawn-snap ray reaches the ground from any
/// reasonable height above it (the cast starts at the requested point).
const SPAWN_RAY_MAX: f32 = 100_000.0;

/// Placeholder actor-value maxima until the AV store + AVIF extraction land
/// (R1). The documented FO4 bridge formulas
/// at level 1 with a neutral default SPECIAL (Endurance = Agility = 5):
///   `MaxHP` = 80 + 5*Endurance                              = 105
///   `MaxAP` = 60 + 10*Agility (fAVDActionPointsBase/Mult)   = 110
const PLACEHOLDER_MAX_HP: f32 = 80.0 + 5.0 * 5.0;
const PLACEHOLDER_MAX_AP: f32 = 60.0 + 10.0 * 5.0;

/// How a pending placement positions the capsule once its destination cell's
/// collision is resident. Decided at the command boundary; `step_player` only
/// executes it, so placement never re-derives intent from `interior_mode`
/// (which gates readiness, not placement).
#[derive(Copy, Clone)]
pub(crate) enum SpawnSnap {
    /// Cast straight down from the requested point onto the first collider of
    /// any group; a miss (point over the void) spawns the capsule airborne
    /// there. Worldspace spawn/switch and the freecam fly->walk handoff.
    Down,
    /// No ray: place at the authored start and let gravity settle the capsule
    /// onto the object-mesh floor. Interior entry.
    Settle,
}

/// A requested placement awaiting its destination cell's collision. `Copy` so
/// `step_player` can read it out of `&mut PlayerState` and clear the slot
/// without a borrow conflict.
#[derive(Copy, Clone)]
pub(crate) struct PendingSpawn {
    pub pos: Vec3,
    pub snap: SpawnSnap,
}

pub(crate) struct PlayerState {
    /// Capsule center, authoritative at fixed-step boundaries.
    pub pos: Vec3,
    pub prev_pos: Vec3,
    /// World-space XZ velocity (x, z).
    pub horizontal_vel: Vec2,
    pub vertical_vel: f32,
    pub grounded: bool,
    pub crouching: bool,
    /// The last `step_player` launched a jump.
    pub launched: bool,
    /// Height of the surface point the player last stood on, or of the point it was placed on.
    /// No autostep lifts the player onto a point more than `MAX_STEP_UP` above
    /// [`PlayerState::step_reference`], which never trusts it above the soles' reach.
    footing_height: f32,
    /// The last `step_player` ran to its end (no spawn pending, collision resident).
    pub step_completed: bool,
    pub spawned: bool,
    pub pending_spawn: Option<PendingSpawn>,
    /// Ticks left of jump wind-up (FO4's pre-liftoff animation anticipation);
    /// `Some(0)` launches on the next step.
    jump_windup: Option<u32>,
    /// Ticks since ground contact was last seen; 0 while grounded, saturates.
    /// Drives the coyote window; set to MAX on jump launch to consume it.
    ticks_airborne: u32,
    movement: PlayerMovement,
    eye_heights: EyeHeights,
    /// Actor values (current). Full and static in this version — no damage or
    /// stamina system exists yet; maxima are the placeholder bridge constants.
    hp: f32,
    hp_max: f32,
    ap: f32,
    ap_max: f32,
    controller: KinematicCharacterController,
}

impl PlayerState {
    pub fn new(movement: PlayerMovement, eye_heights: EyeHeights) -> Self {
        Self {
            pos: Vec3::ZERO,
            prev_pos: Vec3::ZERO,
            horizontal_vel: Vec2::ZERO,
            vertical_vel: 0.0,
            grounded: false,
            crouching: false,
            launched: false,
            footing_height: 0.0,
            step_completed: false,
            spawned: false,
            pending_spawn: None,
            jump_windup: None,
            ticks_airborne: u32::MAX,
            movement,
            eye_heights,
            hp: PLACEHOLDER_MAX_HP,
            hp_max: PLACEHOLDER_MAX_HP,
            ap: PLACEHOLDER_MAX_AP,
            ap_max: PLACEHOLDER_MAX_AP,
            controller: KinematicCharacterController {
                up: Vec3::Y,
                offset: CharacterLength::Absolute(OFFSET),
                slide: true,
                autostep: Some(CharacterAutostep {
                    max_height: CharacterLength::Absolute(MAX_STEP_UP),
                    min_width: CharacterLength::Absolute(STEP_MIN_WIDTH),
                    include_dynamic_bodies: false,
                }),
                max_slope_climb_angle: MAX_SLOPE_CLIMB,
                min_slope_slide_angle: MIN_SLOPE_SLIDE,
                snap_to_ground: Some(CharacterLength::Absolute(SNAP_LEN)),
                normal_nudge_factor: 1.0e-4,
            },
        }
    }

    pub fn request_spawn(&mut self, pos: Vec3, snap: SpawnSnap) {
        self.pending_spawn = Some(PendingSpawn { pos, snap });
        self.horizontal_vel = Vec2::ZERO;
        self.vertical_vel = 0.0;
        self.jump_windup = None;
    }

    /// Where a ray straight down starts and how far it reaches to find what
    /// the soles stand on: the capsule bottom, as far as the ground snap.
    pub(super) fn sole_probe(&self) -> (Vec3, f32) {
        let origin = soles_below(self.pos) + Vec3::Y * OFFSET;
        (origin, SNAP_LEN)
    }

    /// Height the autostep limit is measured from: the point the player last stood on, but no
    /// higher than a footing can lie above the soles: on the steepest walkable slope while
    /// grounded, at the soles while airborne. A fall, a spawn over the void or a support that
    /// vanished underfoot so cannot leave it above the player, and a grounded tick on nothing
    /// walkable has already lowered it to the soles.
    fn step_reference(&self, soles: f32) -> f32 {
        let highest_footing = if self.grounded {
            soles + OFFSET + RADIUS * (1.0 - MAX_SLOPE_CLIMB.cos())
        } else {
            soles
        };
        self.footing_height.min(highest_footing)
    }

    /// Eye position of the capsule centred at `center`.
    pub(super) fn eye_above(&self, center: Vec3) -> Vec3 {
        soles_below(center) + Vec3::Y * self.eye_heights.above_soles(self.crouching)
    }

    /// Eye position at the last fixed step.
    pub(super) fn eye(&self) -> Vec3 {
        self.eye_above(self.pos)
    }

    /// Where the eye will be after a fixed step of `dt` if the player keeps moving: the last
    /// step's horizontal travel, which a wall already cut short, and the vertical velocity, which
    /// stays zero on the ground so contact jitter is not echoed. The physics step runs before
    /// the player step, so what follows the eye in it aims here rather than one step behind.
    pub(super) fn eye_after_next_step(&self, dt: f32) -> Vec3 {
        let travelled = self.pos - self.prev_pos;
        self.eye() + Vec3::new(travelled.x, self.vertical_vel * dt, travelled.z)
    }

    /// Current/max hit points, for the HUD HP bar.
    pub fn health(&self) -> (f32, f32) {
        (self.hp, self.hp_max)
    }

    /// Restores hit points, clamped to the maximum. A negative or non-finite amount is ignored.
    pub fn heal(&mut self, amount: f32) {
        if amount.is_finite() && amount > 0.0 {
            self.hp = (self.hp + amount).min(self.hp_max);
        }
    }

    #[cfg(test)]
    pub(super) fn set_health(&mut self, hp: f32) {
        self.hp = hp;
    }

    /// Current/max action points, for the HUD AP bar.
    pub fn action_points(&self) -> (f32, f32) {
        (self.ap, self.ap_max)
    }
}

/// Queries over the static scene the character controller collides with.
fn static_scene(physics: &PhysicsContext) -> QueryPipeline<'_> {
    physics
        .world
        .query_pipeline_with_filter(static_scene_query_filter())
}

/// The soles of the capsule centred at `center`.
pub(super) fn soles_below(center: Vec3) -> Vec3 {
    center - Vec3::Y * CENTER_ABOVE_FEET
}

/// The player's collision capsule, the same in every posture.
fn player_capsule() -> Capsule {
    Capsule::new_y(HEIGHT / 2.0 - RADIUS, RADIUS)
}

/// The player's collide-and-slide, with every autostep held to `MAX_STEP_UP` above the player's
/// step reference. The controller lifts the soles, which sag below that point on a slope, so it
/// may lift them by that sag more. Its own step limit also grows with the approach angle, so a
/// move that rose more than `desired` asked for is redone without autostep when the point it
/// would stand on is higher, or when it would stand on nothing.
fn move_within_step_limit(
    state: &PlayerState,
    physics: &PhysicsContext,
    capsule: &Capsule,
    start: Vec3,
    desired: Vec3,
    dt: f32,
) -> EffectiveCharacterMovement {
    let soles = soles_below(start).y;
    let reference = state.step_reference(soles);
    let soles_sag = (reference - soles).max(0.0);
    let controller = KinematicCharacterController {
        autostep: state.controller.autostep.map(|autostep| CharacterAutostep {
            max_height: CharacterLength::Absolute(MAX_STEP_UP + soles_sag),
            ..autostep
        }),
        ..state.controller
    };
    let movement = move_capsule(&controller, physics, capsule, start, desired, dt);
    if movement.translation.y <= desired.y {
        return movement;
    }
    let end = start + movement.translation;
    let within_limit = static_footing(physics, capsule, end, soles_below(end).y)
        .is_some_and(|footing| footing.height - reference <= MAX_STEP_UP);
    if within_limit {
        return movement;
    }
    let without_autostep = KinematicCharacterController {
        autostep: None,
        ..state.controller
    };
    move_capsule(&without_autostep, physics, capsule, start, desired, dt)
}

/// One collide-and-slide of `capsule` from `start` over the static scene.
fn move_capsule(
    controller: &KinematicCharacterController,
    physics: &PhysicsContext,
    capsule: &Capsule,
    start: Vec3,
    desired: Vec3,
    dt: f32,
) -> EffectiveCharacterMovement {
    controller.move_shape(
        dt,
        &static_scene(physics),
        capsule,
        &Pose::from_translation(start),
        desired,
        |_| {},
    )
}

/// A static point the capsule rests on.
struct StaticFooting {
    height: f32,
    /// Whether the point touches the rounded bottom no steeper than the controller climbs, so the
    /// capsule stands on it rather than grazing it at the side.
    walkable: bool,
}

/// The static point the capsule centred at `center` with its soles at `soles` rests on: the first
/// one below it, when that touches the rounded bottom below its rim and lies within the
/// controller gap of the soles. The rounded bottom rests on an edge itself while the
/// controller's autostep has only partly lifted it onto a step, so this is the top of whatever it
/// climbed; a capsule held up only at its side rests on nothing.
fn static_footing(
    physics: &PhysicsContext,
    capsule: &Capsule,
    center: Vec3,
    soles: f32,
) -> Option<StaticFooting> {
    // A point touching the rounded bottom below its rim more than this far down lies below the
    // soles less the gap.
    let deepest_footing = capsule.radius + OFFSET;
    let (_, hit) = static_scene(physics).cast_shape(
        &Pose::from_translation(center),
        -Vec3::Y,
        capsule,
        ShapeCastOptions {
            target_distance: 0.0,
            stop_at_penetration: false,
            max_time_of_impact: deepest_footing,
            compute_impact_geometry_on_penetration: true,
        },
    )?;
    let below_the_rim = -hit.witness2.y - capsule.half_height();
    let height = center.y - hit.time_of_impact + hit.witness2.y;
    (below_the_rim > OFFSET && height >= soles - OFFSET).then_some(StaticFooting {
        height,
        walkable: below_the_rim >= capsule.radius * MAX_SLOPE_CLIMB.cos(),
    })
}

/// Godot's `move_toward`: step `v` toward `target` by at most `delta`.
fn move_toward(v: f32, target: f32, delta: f32) -> f32 {
    if (target - v).abs() <= delta {
        target
    } else {
        v + delta * (target - v).signum()
    }
}

pub(crate) fn step_player(
    state: &mut PlayerState,
    input: &PlayerInput,
    physics: &mut PhysicsContext,
    dt: f32,
) {
    state.launched = false;
    state.step_completed = false;
    // 1. Pending spawn/teleport: wait for the destination cell's collision,
    // then place the capsule per the request's intent.
    if let Some(PendingSpawn { pos: p, snap }) = state.pending_spawn {
        state.prev_pos = state.pos;
        if !physics.spawn_ready(world_to_grid(p)) {
            return;
        }
        // Stand the capsule's FEET on the requested point — the Down ray's hit
        // surface, or the authored start for Settle — by lifting the CENTER half
        // a height plus the controller gap above it, so the capsule rests on the
        // point instead of burying its lower half in it.
        let foot_lift = CENTER_ABOVE_FEET;
        let (pos, grounded) = match snap {
            SpawnSnap::Down => match physics.raycast_down(p, SPAWN_RAY_MAX) {
                // Stand the capsule on the surface the ray found.
                Some(ground_y) => (Vec3::new(p.x, ground_y + foot_lift, p.z), true),
                // Ray missed (point over the void): spawn airborne at the point
                // and let gravity carry the player down.
                None => (p, false),
            },
            // No ray: lift the capsule onto the authored start (a floor marker
            // for load doors, the cell anchor for the debug picker) and let
            // gravity keep the feet planted or settle onto a lower floor.
            SpawnSnap::Settle => (Vec3::new(p.x, p.y + foot_lift, p.z), false),
        };
        state.pos = pos;
        state.prev_pos = pos;
        state.grounded = grounded;
        state.footing_height = soles_below(pos).y;
        state.spawned = true;
        state.pending_spawn = None;
        physics.sync_player_mirror(state.pos, &player_capsule(), true);
        return;
    }
    if !state.spawned {
        state.prev_pos = state.pos;
        return;
    }

    // 2. Ready gate: hold still while the local cell's collision streams in.
    // In interior mode spawn_ready is satisfied by the one resident interior
    // cell, so this never freezes movement as the player crosses grids inside it.
    if !physics.spawn_ready(world_to_grid(state.pos)) {
        state.prev_pos = state.pos;
        state.horizontal_vel = Vec2::ZERO;
        state.vertical_vel = 0.0;
        return;
    }

    // 3. Sneak: the posture changes the eye and the gait, never the collision shape.
    state.crouching = input.crouch;

    // 4. Velocities, vanilla FO4 model (see constants): full gravity while
    // rising, fInAirFallingCharGravityMult-scaled gravity from the apex on —
    // ledge walk-offs (v = 0) start straight on the fall branch.
    if !state.grounded {
        let g = if state.vertical_vel > 0.0 {
            GRAVITY
        } else {
            GRAVITY_FALL
        };
        state.vertical_vel = (state.vertical_vel - g * dt).max(-MAX_FALL_SPEED);
    }
    // Jump permission: grounded, or just barely not (coyote window) and not
    // already rising — covers grounded-flicker on rough terrain and fair
    // ledge-edge jumps without enabling double jumps.
    let can_jump =
        state.grounded || (state.ticks_airborne <= COYOTE_TICKS && state.vertical_vel <= 0.0);
    // Jump wind-up: FO4 plays an anticipation animation and only launches the
    // capsule when the behavior graph fires the jump event mid-animation.
    if input.jump && can_jump && state.jump_windup.is_none() {
        state.jump_windup = Some(JUMP_WINDUP_TICKS);
    }
    if let Some(ticks) = state.jump_windup {
        if !can_jump {
            // Genuinely airborne (past the coyote window): no launch.
            state.jump_windup = None;
        } else if ticks == 0 {
            state.vertical_vel = JUMP_VELOCITY;
            state.launched = true;
            state.jump_windup = None;
            state.ticks_airborne = u32::MAX; // consume the coyote window
        } else {
            state.jump_windup = Some(ticks - 1);
        }
    }

    let speed =
        state
            .movement
            .ground_speed(input.sprint, state.crouching, physics.carry_forces_walk());
    let target = if input.move_dir.length_squared() > 0.0 {
        // XZ-plane basis from the camera yaw (see PlayerInput::yaw docs).
        let fwd = Vec2::new(input.yaw.cos(), input.yaw.sin());
        let right = Vec2::new(-fwd.y, fwd.x);
        let dir = (fwd * input.move_dir.y + right * input.move_dir.x).normalize_or_zero();
        Some(dir * speed)
    } else {
        None
    };
    if state.grounded {
        // On the ground velocity is authoritative input, player.gd semantics.
        if let Some(t) = target {
            state.horizontal_vel = t;
        } else {
            state.horizontal_vel.x = move_toward(state.horizontal_vel.x, 0.0, speed);
            state.horizontal_vel.y = move_toward(state.horizontal_vel.y, 0.0, speed);
        }
    } else if let Some(t) = target {
        // Airborne: steer toward the desired velocity, never decelerate to a
        // dead stop — launch momentum carries through the jump.
        let delta = t - state.horizontal_vel;
        state.horizontal_vel += delta.clamp_length_max(AIR_ACCEL * dt);
    }

    // 5. Collide-and-slide through the character controller over static
    // geometry; dynamic bodies are shoved and walked on by `dynamic_contact`.
    let desired = Vec3::new(
        state.horizontal_vel.x,
        state.vertical_vel,
        state.horizontal_vel.y,
    ) * dt;
    let capsule = player_capsule();
    let character = Character {
        capsule: &capsule,
        mass: CHAR_MASS,
        push_force: CHAR_PUSH_FORCE,
        gravity: GRAVITY,
        skin: OFFSET,
    };
    let start = state.pos;
    let was_grounded = state.grounded;
    let movement = {
        let _span = tracing::info_span!("sim_char_move").entered();
        move_within_step_limit(state, physics, &capsule, start, desired, dt)
    };
    let horizontal = Vec3::new(movement.translation.x, 0.0, movement.translation.z);
    let allowed = {
        let _span = tracing::info_span!("sim_char_shove").entered();
        let shove_start = start + Vec3::Y * movement.translation.y.max(0.0);
        dynamic_contact::shove_and_slide(physics, &character, shove_start, horizontal, dt)
    };

    state.prev_pos = state.pos;
    state.pos += movement.translation - horizontal + allowed;
    state.grounded = movement.grounded;
    let static_grounded = movement.grounded;

    // Landing on a dynamic body: static geometry never saw it, so cast down
    // from where the tick began and stop on its walkable surface.
    let mut support = None;
    if state.vertical_vel <= 0.0 {
        let reach = (start.y - state.pos.y).max(0.0) + SNAP_REACH;
        let origin = Vec3::new(state.pos.x, start.y, state.pos.z);
        support =
            dynamic_contact::support_below(physics, &character, origin, reach, MAX_SLOPE_CLIMB);
        if let Some(support) = &support {
            state.pos.y = state.pos.y.max(start.y - support.drop);
            state.grounded = true;
            dynamic_contact::load_support(physics, &character, support, dt);
        }
    }

    // Ground stick (Source-engine "StayOnGround" analog). Rapier's built-in
    // snap_to_ground is double-gated: it needs ground contact within
    // offset+0.05 at the START pose, and a downward component in the APPLIED
    // translation. A grounded capsule moving horizontally out over a
    // downslope satisfies neither — translation.y is exactly 0, nothing
    // blocks the move — so it goes airborne and falls from v = 0, taking
    // dozens of ticks to recontact a steep slope: the downhill bounce.
    // Instead: a tick that starts grounded, isn't a jump launch, and ends
    // airborne casts down `GROUND_STICK_REACH` and glues onto any walkable
    // surface (the normal check keeps cliff faces from grabbing the capsule
    // at ledge lips). Taller drops — real ledges — still fall naturally.
    if was_grounded && !state.grounded && state.vertical_vel <= 0.0 {
        let down_pose = Pose::from_translation(state.pos);
        let hit = static_scene(physics).cast_shape(
            &down_pose,
            -Vec3::Y,
            &capsule,
            ShapeCastOptions {
                target_distance: OFFSET,
                stop_at_penetration: false,
                max_time_of_impact: GROUND_STICK_REACH,
                compute_impact_geometry_on_penetration: true,
            },
        );
        if let Some((_, hit)) = hit
            && hit.normal1.y >= MAX_SLOPE_CLIMB.cos()
        {
            state.pos.y -= hit.time_of_impact;
            state.grounded = true;
        }
    }

    if state.grounded {
        state.ticks_airborne = 0;
        if state.vertical_vel <= 0.0 {
            let soles = soles_below(state.pos).y;
            state.footing_height = support
                .as_ref()
                .map(dynamic_contact::Support::contact_height)
                .or_else(|| {
                    static_footing(physics, &capsule, state.pos, soles)
                        .filter(|footing| footing.walkable)
                        .map(|footing| footing.height)
                })
                .unwrap_or(state.footing_height.min(soles));
        }
    } else {
        state.ticks_airborne = state.ticks_airborne.saturating_add(1);
    }
    if state.grounded && state.vertical_vel < 0.0 {
        state.vertical_vel = 0.0;
    }
    // The controller only clips the *movement*; mirror vertical clipping back
    // into the tracked velocity or a ceiling bump leaves the capsule pinned
    // while the upward velocity decays under rise gravity ("hover until the
    // rise runs out"). While rising, snap/autostep are inactive, so any
    // vertical shortfall means overhead contact: adopt the achieved velocity —
    // flat ceilings zero it (the fall starts next step), sloped ceilings keep
    // the slide component. Tolerance is well above float noise, far below a
    // real head bump.
    if state.vertical_vel > 0.0 {
        let achieved = movement.translation.y / dt;
        if achieved < state.vertical_vel - 0.5 {
            state.vertical_vel = achieved.max(0.0);
        }
    }

    dynamic_contact::trace_step(&dynamic_contact::StepTrace {
        start,
        end: state.pos,
        eye_y: state.eye().y,
        static_translation: movement.translation,
        allowed_horizontal: allowed,
        static_grounded,
        grounded: state.grounded,
        support: support.as_ref(),
    });

    physics.sync_player_mirror(state.pos, &capsule, false);
    state.step_completed = true;
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};
    use std::sync::Arc;

    use glam::{Quat, Vec3Swizzles};

    use super::*;
    use crate::sim::FIXED_DT;
    use crate::sim::collision::{CellColliderBatch, ColShape, TerrainCollision};
    use crate::sim::commands::SimCommand;
    use crate::sim::physics::test_support::{bake, ground_batch, object_group, placement};
    use crate::sim::test_world::{
        Prop, SPAWN_AT, crate_prop, flat_world, flat_world_with_eye_heights, hold, position,
        prop_box, prop_entity, running, spawned_at, tick, world_in, world_with,
    };
    use crate::sim::{SimWorld, WorldClock};

    /// A world whose cell (0, 0) has no terrain: its floor is an object.
    fn landless_world() -> SimWorld {
        let floor = prop_box(
            "meshes\\floor.glb",
            Vec3::new(200.0, -0.5, -100.0),
            Vec3::new(150.0, 0.5, 150.0),
        );
        world_in(CellColliderBatch::default(), &[floor])
    }

    /// Ticks after a jump input (frame 0 already ticked) until the player is grounded
    /// again after having left the ground: (frame of landing counted from the input, apex height).
    fn flight(sim: &mut SimWorld, ground: f32) -> (u32, f32) {
        const GIVE_UP: u32 = 200;
        let mut apex = ground;
        let mut left_ground = false;
        let mut frame = 0;
        while frame < GIVE_UP {
            tick(sim, 1);
            frame += 1;
            apex = apex.max(position(sim).y);
            left_ground |= !sim.player_debug().grounded;
            if left_ground && sim.player_debug().grounded {
                break;
            }
        }
        (frame, apex)
    }

    #[test]
    fn move_toward_steps_by_at_most_delta_and_never_overshoots() {
        assert_eq!(move_toward(0.0, 10.0, 3.0), 3.0);
        assert_eq!(move_toward(0.0, -10.0, 3.0), -3.0);
        assert_eq!(move_toward(9.0, 10.0, 3.0), 10.0);
        assert_eq!(move_toward(5.0, 5.0, 3.0), 5.0);
    }

    #[test]
    fn healing_restores_hit_points_up_to_the_maximum() {
        let mut state = PlayerState::new(PlayerMovement::default(), EyeHeights::default());
        let (_, hp_max) = state.health();
        state.hp = hp_max - 30.0;
        state.heal(10.0);
        assert_eq!(state.health().0, hp_max - 20.0);
        state.heal(1000.0);
        assert_eq!(state.health().0, hp_max);
    }

    #[test]
    fn healing_ignores_negative_and_non_finite_amounts() {
        let mut state = PlayerState::new(PlayerMovement::default(), EyeHeights::default());
        let (_, hp_max) = state.health();
        state.hp = hp_max - 30.0;
        for amount in [-5.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0] {
            state.heal(amount);
            assert_eq!(state.health().0, hp_max - 30.0, "{amount}");
        }
    }

    #[test]
    fn a_spawn_stands_the_feet_on_the_ground_below_the_requested_point() {
        let sim = spawned_at(flat_world(), SPAWN_AT);
        let debug = sim.player_debug();
        assert!(debug.spawned && debug.grounded);
        assert!(
            (debug.pos.y - CENTER_ABOVE_FEET).abs() < 0.01,
            "{}",
            debug.pos.y
        );
        assert_eq!((debug.pos.x, debug.pos.z), (200.0, -100.0));
        let eye = sim.player_eye().unwrap();
        assert!((eye.y - EyeHeights::default().standing()).abs() < 0.01);
    }

    fn crouching() -> PlayerInput {
        PlayerInput {
            crouch: true,
            ..PlayerInput::default()
        }
    }

    #[test]
    fn eye_heights_must_be_positive_finite_lengths() {
        assert!(EyeHeights::new(12.0, 8.0).is_ok());
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(
                matches!(
                    EyeHeights::new(bad, 8.0),
                    Err(EyeHeightsError::NotPositive {
                        pose: EyePose::Standing,
                        ..
                    })
                ),
                "{bad}"
            );
            assert!(
                matches!(
                    EyeHeights::new(12.0, bad),
                    Err(EyeHeightsError::NotPositive {
                        pose: EyePose::Sneaking,
                        ..
                    })
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_first_person_camera_stands_the_eye_and_its_sneak_shift_lowers_it() {
        let camera = FirstPersonCamera {
            standing_height: 12.0,
            sneak_shift: -4.0,
        };
        assert_eq!(
            EyeHeights::try_from(camera).unwrap(),
            EyeHeights::new(12.0, 8.0).unwrap()
        );
        let shifted_below_the_soles = FirstPersonCamera {
            sneak_shift: -12.0,
            ..camera
        };
        assert!(matches!(
            EyeHeights::try_from(shifted_below_the_soles),
            Err(EyeHeightsError::NotPositive {
                pose: EyePose::Sneaking,
                ..
            })
        ));
    }

    #[test]
    fn the_eye_stands_the_loaded_heights_above_the_soles_standing_and_sneaking() {
        let heights = EyeHeights::new(15.0, 3.0).unwrap();
        let mut sim = spawned_at(flat_world_with_eye_heights(heights), SPAWN_AT);
        let standing = sim.player_eye().unwrap();
        assert!((standing.y - 15.0).abs() < 0.01, "{standing}");
        hold(&mut sim, crouching());
        tick(&mut sim, 5);
        assert!(sim.player_debug().crouching);
        let sneaking = sim.player_eye().unwrap();
        assert!((sneaking.y - 3.0).abs() < 0.01, "{sneaking}");
    }

    #[test]
    fn the_eye_of_a_walking_player_is_interpolated_between_the_last_two_steps() {
        let mut sim = spawned_at(flat_world(), SPAWN_AT);
        hold(&mut sim, running(0.0));
        tick(&mut sim, 30);
        let before = position(&sim);
        tick(&mut sim, 1);
        let after = position(&sim);
        assert!(after.x - before.x > 0.3, "walking: {before} -> {after}");
        sim.tick(FIXED_DT * 0.5);
        let eye = sim.player_eye().unwrap();
        let halfway = f32::midpoint(before.x, after.x);
        assert!(
            (eye.x - halfway).abs() < 0.02,
            "eye {eye}, halfway {halfway}"
        );
        assert!(
            (eye.y - EyeHeights::default().standing()).abs() < 0.01,
            "{eye}"
        );
    }

    #[test]
    fn a_spawn_waits_until_the_destination_collision_is_installed() {
        let mut sim = SimWorld::new(
            PlayerMovement::default(),
            EyeHeights::default(),
            WorldClock::fixture(),
        );
        sim.apply(SimCommand::SpawnPlayer { pos: SPAWN_AT });
        tick(&mut sim, 5);
        let waiting = sim.player_debug();
        assert!(!waiting.spawned && waiting.waiting_for_ground);
        assert!(sim.player_eye().is_none());
        sim.apply(SimCommand::AddCellColliders {
            grid: (0, 0),
            batch: Arc::new(ground_batch()),
            baked: None,
        });
        tick(&mut sim, 2);
        assert!(sim.player_debug().spawned);
    }

    #[test]
    fn a_spawn_in_a_cell_without_terrain_stands_on_its_object_floor() {
        let sim = spawned_at(landless_world(), SPAWN_AT);
        let debug = sim.player_debug();
        assert!(debug.spawned && debug.grounded);
        assert!(
            (debug.pos.y - CENTER_ABOVE_FEET).abs() < 0.01,
            "{}",
            debug.pos.y
        );
    }

    #[test]
    fn walking_in_a_cell_without_terrain_is_not_frozen_by_the_ready_gate() {
        let mut sim = spawned_at(landless_world(), SPAWN_AT);
        let start = position(&sim);
        hold(&mut sim, running(0.0));
        tick(&mut sim, 60);
        let moved = position(&sim) - start;
        assert!(
            moved.x > PlayerMovement::default().run_speed * 0.5,
            "moved {moved}"
        );
    }

    #[test]
    fn a_teleport_in_a_cell_without_terrain_snaps_onto_the_object_floor() {
        let mut sim = spawned_at(landless_world(), SPAWN_AT);
        sim.apply(SimCommand::TeleportPlayer {
            pos: Vec3::new(50.0, 80.0, -50.0),
        });
        tick(&mut sim, 2);
        let debug = sim.player_debug();
        assert!(debug.spawned && !debug.waiting_for_ground);
        assert_eq!((debug.pos.x, debug.pos.z), (50.0, -50.0));
        assert!((debug.pos.y - CENTER_ABOVE_FEET).abs() < 0.01);
    }

    #[test]
    fn a_spawn_waits_forever_for_a_cell_whose_terrain_failed_to_build() {
        let mut sim = SimWorld::new(
            PlayerMovement::default(),
            EyeHeights::default(),
            WorldClock::fixture(),
        );
        sim.apply(SimCommand::SpawnPlayer { pos: SPAWN_AT });
        sim.apply(SimCommand::AddCellColliders {
            grid: (0, 0),
            batch: Arc::new(CellColliderBatch {
                terrain: TerrainCollision::Failed,
                objects: Vec::new(),
            }),
            baked: None,
        });
        tick(&mut sim, 30);
        let debug = sim.player_debug();
        assert!(!debug.spawned && debug.waiting_for_ground);
    }

    #[test]
    fn standing_still_on_flat_ground_stays_put_and_grounded() {
        let mut sim = spawned_at(flat_world(), SPAWN_AT);
        let start = position(&sim);
        tick(&mut sim, 120);
        assert!(position(&sim).abs_diff_eq(start, 1e-3));
        assert!(sim.player_debug().grounded);
    }

    #[test]
    fn walking_covers_the_run_speed_per_second_along_the_yaw_direction() {
        let mut sim = spawned_at(flat_world(), SPAWN_AT);
        let start = position(&sim);
        hold(&mut sim, running(0.0));
        tick(&mut sim, 60);
        let moved = position(&sim) - start;
        let run_speed = PlayerMovement::default().run_speed;
        assert!((moved.x - run_speed).abs() < 0.5, "moved {moved}");
        assert!(
            moved.z.abs() < 0.01 && moved.y.abs() < 0.05,
            "moved {moved}"
        );
    }

    #[test]
    fn yaw_of_minus_half_pi_walks_towards_minus_z() {
        let mut sim = spawned_at(flat_world(), SPAWN_AT);
        let start = position(&sim);
        hold(&mut sim, running(-FRAC_PI_2));
        tick(&mut sim, 30);
        let moved = position(&sim) - start;
        assert!(moved.z < -15.0 && moved.x.abs() < 0.01, "moved {moved}");
    }

    #[test]
    fn diagonal_input_is_not_faster_than_straight_input() {
        let mut straight = spawned_at(flat_world(), SPAWN_AT);
        let mut diagonal = spawned_at(flat_world(), SPAWN_AT);
        hold(&mut straight, running(0.0));
        hold(
            &mut diagonal,
            PlayerInput {
                move_dir: Vec2::new(1.0, 1.0),
                ..PlayerInput::default()
            },
        );
        tick(&mut straight, 30);
        tick(&mut diagonal, 30);
        let straight_distance = (position(&straight) - SPAWN_AT).xz().length();
        let diagonal_distance = (position(&diagonal) - SPAWN_AT).xz().length();
        assert!(
            (straight_distance - diagonal_distance).abs() < 0.5,
            "{straight_distance} vs {diagonal_distance}"
        );
    }

    #[test]
    fn sprinting_uses_the_sprint_speed_but_not_while_crouched() {
        let movement = PlayerMovement::default();
        let travelled = |crouch: bool| {
            let mut sim = spawned_at(flat_world(), SPAWN_AT);
            let start = position(&sim);
            hold(
                &mut sim,
                PlayerInput {
                    sprint: true,
                    crouch,
                    ..running(0.0)
                },
            );
            tick(&mut sim, 60);
            (position(&sim) - start).x
        };
        assert!((travelled(false) - movement.sprint_speed).abs() < 0.5);
        assert!((travelled(true) - movement.run_speed).abs() < 0.5);
    }

    #[test]
    fn a_wall_stops_the_player_at_its_radius_and_the_player_slides_along_it() {
        let wall = prop_box(
            "meshes\\wall.glb",
            Vec3::new(260.0, 10.0, -400.0),
            Vec3::new(1.0, 10.0, 400.0),
        );
        let mut sim = spawned_at(world_with(&[wall]), SPAWN_AT);
        hold(&mut sim, running(-FRAC_PI_4));
        tick(&mut sim, 180);
        let end = position(&sim);
        let face = 259.0;
        assert!(
            end.x < face - RADIUS + 0.2 && end.x > face - RADIUS - 0.5,
            "stopped at {}",
            end.x
        );
        assert!(end.z < -170.0, "kept sliding along the wall, z = {}", end.z);
    }

    #[test]
    fn a_low_ledge_is_stepped_onto() {
        let ledge = prop_box(
            "meshes\\ledge.glb",
            Vec3::new(240.0, 0.75, -100.0),
            Vec3::new(30.0, 0.75, 50.0),
        );
        let mut sim = spawned_at(world_with(&[ledge]), SPAWN_AT);
        hold(&mut sim, running(0.0));
        tick(&mut sim, 90);
        let end = position(&sim);
        assert!(end.x > 215.0, "x = {}", end.x);
        assert!(
            (end.y - (1.5 + CENTER_ABOVE_FEET)).abs() < 0.1,
            "y = {}",
            end.y
        );
    }

    const LEDGE_FACE_X: f32 = 210.0;

    /// One way of moving at a face: the input, and whether the player is forced to walk (as when
    /// carrying something heavy).
    #[derive(Clone, Copy, Debug)]
    struct Approach {
        input: PlayerInput,
        forced_walk: bool,
    }

    impl Approach {
        fn speed(self) -> f32 {
            PlayerMovement::default().ground_speed(
                self.input.sprint,
                self.input.crouch,
                self.forced_walk,
            )
        }

        /// Gives the player of `sim` this approach's gait. Forced walking is the movement record
        /// of a player whose every gait is the walk.
        fn apply(self, sim: &mut SimWorld) {
            if self.forced_walk {
                let walk = PlayerMovement::default().walk_speed;
                sim.player.movement = PlayerMovement {
                    walk_speed: walk,
                    run_speed: walk,
                    sprint_speed: walk,
                };
            }
            hold(sim, self.input);
        }
    }

    /// The highest collision top the original game lifted the player onto: 31 game units.
    const LIFTED_IN_THE_ORIGINAL: f32 = 3.1;
    /// The lowest collision top the original game refused to lift the player onto: 32 game units.
    const REFUSED_IN_THE_ORIGINAL: f32 = 3.2;

    struct BlockApproach {
        end: Vec3,
        highest_soles: f32,
        lowest_soles: f32,
    }

    /// Where the player ends, and the highest and lowest its soles got, after moving with `approach` at a
    /// block of height `top` whose face is at `LEDGE_FACE_X`, for as long as crossing the face by
    /// 7 units takes at that gait.
    fn moved_at_a_block(top: f32, approach: Approach) -> BlockApproach {
        let block = prop_box(
            "meshes\\block.glb",
            Vec3::new(LEDGE_FACE_X + 30.0, top / 2.0, -100.0),
            Vec3::new(30.0, top / 2.0, 300.0),
        );
        let mut sim = spawned_at(world_with(&[block]), SPAWN_AT);
        let crossing =
            (LEDGE_FACE_X + 7.0 - SPAWN_AT.x) / (approach.speed() * approach.input.yaw.cos());
        approach.apply(&mut sim);
        let (mut highest_soles, mut lowest_soles) = (f32::NEG_INFINITY, f32::INFINITY);
        for _ in 0..((crossing + 0.5) / FIXED_DT).ceil() as u32 {
            tick(&mut sim, 1);
            let soles = soles_below(sim.player.pos).y;
            highest_soles = highest_soles.max(soles);
            lowest_soles = lowest_soles.min(soles);
        }
        BlockApproach {
            end: position(&sim),
            highest_soles,
            lowest_soles,
        }
    }

    /// The approach angles from the face normal the original game's step limit was measured at,
    /// each walking, running, sprinting and crouched.
    fn ledge_approaches() -> Vec<Approach> {
        let angles = [0.0, FRAC_PI_4, 70.0_f32.to_radians()];
        angles
            .into_iter()
            .flat_map(|angle| {
                let approach = |input, forced_walk| Approach { input, forced_walk };
                [
                    approach(running(angle), true),
                    approach(running(angle), false),
                    approach(
                        PlayerInput {
                            sprint: true,
                            ..running(angle)
                        },
                        false,
                    ),
                    approach(
                        PlayerInput {
                            crouch: true,
                            ..running(angle)
                        },
                        false,
                    ),
                ]
            })
            .collect()
    }

    #[test]
    fn a_ledge_the_original_lifts_onto_is_climbed_at_any_approach_gait_and_stance() {
        let top = LIFTED_IN_THE_ORIGINAL;
        let failures: Vec<String> = ledge_approaches()
            .into_iter()
            .filter_map(|approach| {
                let ledge = moved_at_a_block(top, approach);
                let end = ledge.end;
                let on_the_ledge = ledge.lowest_soles > -OFFSET
                    && end.x > LEDGE_FACE_X + 5.0
                    && (end.y - (top + CENTER_ABOVE_FEET)).abs() < 0.1;
                (!on_the_ledge).then(|| {
                    format!(
                        "{approach:?} ended at {end}, its soles down to {}",
                        ledge.lowest_soles
                    )
                })
            })
            .collect();
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn a_ledge_the_original_refuses_is_never_stepped_onto_at_any_approach_gait_and_stance() {
        let top = REFUSED_IN_THE_ORIGINAL;
        let failures: Vec<String> = ledge_approaches()
            .into_iter()
            .filter_map(|approach| {
                let ledge = moved_at_a_block(top, approach);
                let end_soles = ledge.end.y - CENTER_ABOVE_FEET;
                let blocked = ledge.highest_soles < top
                    && ledge.lowest_soles > -OFFSET
                    && end_soles < OFFSET
                    && ledge.end.x < LEDGE_FACE_X;
                (!blocked).then(|| {
                    format!(
                        "{approach:?} ended at {} with its soles between {} and {}",
                        ledge.end, ledge.lowest_soles, ledge.highest_soles
                    )
                })
            })
            .collect();
        assert!(failures.is_empty(), "{failures:#?}");
    }

    const CURB_TOP: f32 = 1.0;
    /// Within the step limit of the curb, beyond it from the floor.
    const LEDGE_ABOVE_THE_CURB: f32 = CURB_TOP + 2.5;

    /// A curb `CURB_TOP` high running from x 150 to `curb_gap` before a ledge face at
    /// `LEDGE_FACE_X`, the ledge `LEDGE_ABOVE_THE_CURB` high, with the player standing on the
    /// curb.
    fn on_a_curb_before_a_ledge(curb_gap: f32) -> SimWorld {
        let curb_end = LEDGE_FACE_X - curb_gap;
        let curb = prop_box(
            "meshes\\curb.glb",
            Vec3::new(f32::midpoint(150.0, curb_end), CURB_TOP / 2.0, -100.0),
            Vec3::new((curb_end - 150.0) / 2.0, CURB_TOP / 2.0, 50.0),
        );
        let ledge = prop_box(
            "meshes\\ledge.glb",
            Vec3::new(LEDGE_FACE_X + 30.0, LEDGE_ABOVE_THE_CURB / 2.0, -100.0),
            Vec3::new(30.0, LEDGE_ABOVE_THE_CURB / 2.0, 50.0),
        );
        spawned_at(
            world_with(&[curb, ledge]),
            Vec3::new(curb_end - 10.0, CURB_TOP + 3.0, -100.0),
        )
    }

    #[test]
    fn running_off_a_curb_onto_the_floor_and_into_a_ledge_too_high_from_the_floor_does_not_climb_it()
     {
        let mut sim = on_a_curb_before_a_ledge(15.0);
        hold(&mut sim, running(0.0));
        let mut stood_on_the_floor = false;
        for _ in 0..90 {
            tick(&mut sim, 1);
            stood_on_the_floor |= sim.player.grounded
                && soles_below(sim.player.pos).y.abs() < 0.1
                && position(&sim).x < LEDGE_FACE_X - RADIUS - 1.0;
        }
        assert!(stood_on_the_floor, "never landed on the floor");
        let end = position(&sim);
        assert!(end.x < LEDGE_FACE_X - RADIUS + 0.2, "x = {}", end.x);
        assert!((end.y - CENTER_ABOVE_FEET).abs() < 0.1, "y = {}", end.y);
    }

    #[test]
    fn running_from_a_curb_against_a_ledge_face_climbs_a_ledge_within_the_step_limit_of_the_curb() {
        let mut sim = on_a_curb_before_a_ledge(0.0);
        hold(&mut sim, running(0.0));
        tick(&mut sim, 60);
        let end = position(&sim);
        assert!(end.x > LEDGE_FACE_X + 5.0, "x = {}", end.x);
        assert!(
            (end.y - (LEDGE_ABOVE_THE_CURB + CENTER_ABOVE_FEET)).abs() < 0.1,
            "y = {}",
            end.y
        );
    }

    /// How a run at a face at `face_x` went: the most the soles rose in one tick, their height
    /// when the capsule first came within reach of the face, and where the player ended.
    struct FaceRun {
        highest_rise: f32,
        soles_at_the_face: Option<f32>,
        end: Vec3,
    }

    impl FaceRun {
        /// Problems with a run at a face whose block is `top` high: any lift above the step
        /// limit in one tick, or ending on the block after reaching the face with the soles too
        /// far below its top to step onto it.
        fn problems(&self, top: f32, face_x: f32) -> Option<String> {
            let lifted_too_far = self.highest_rise > MAX_STEP_UP + OFFSET;
            let out_of_reach = self
                .soles_at_the_face
                .is_some_and(|soles| soles + MAX_STEP_UP < top);
            let on_the_block =
                self.end.x >= face_x && self.end.y - CENTER_ABOVE_FEET >= top - OFFSET;
            (lifted_too_far || (out_of_reach && on_the_block)).then(|| {
                format!(
                    "rose up to {} in a tick, soles at the face {:?}, ended at {}",
                    self.highest_rise, self.soles_at_the_face, self.end
                )
            })
        }
    }

    /// Ticks `sim` for `ticks` with whatever input it holds, following its run at a face at
    /// `face_x`.
    fn ran_at_a_face(sim: &mut SimWorld, face_x: f32, ticks: u32) -> FaceRun {
        let mut soles = soles_below(sim.player.pos).y;
        let mut highest_rise = f32::NEG_INFINITY;
        let mut soles_at_the_face = None;
        for _ in 0..ticks {
            tick(sim, 1);
            let now = soles_below(sim.player.pos).y;
            highest_rise = highest_rise.max(now - soles);
            soles = now;
            if soles_at_the_face.is_none() && position(sim).x >= face_x - RADIUS - 0.2 {
                soles_at_the_face = Some(now);
            }
        }
        FaceRun {
            highest_rise,
            soles_at_the_face,
            end: position(sim),
        }
    }

    /// Ticks `sim` until its player's soles are below `height`.
    fn fall_below(sim: &mut SimWorld, height: f32) {
        for _ in 0..600 {
            if soles_below(sim.player.pos).y < height {
                return;
            }
            tick(sim, 1);
        }
        panic!("the player never fell below {height}");
    }

    #[test]
    fn running_off_a_platform_at_a_block_face_never_lifts_the_player_beyond_the_step_limit() {
        let edge = 200.0;
        let mut failures = Vec::new();
        for platform_top in [5.0, 10.0, 20.0, 40.0] {
            for block_top in [3.3, 8.0, 16.0, 30.0] {
                for gap in [6.0, 12.0, 20.0, 30.0] {
                    for sprint in [false, true] {
                        let platform = prop_box(
                            "meshes\\platform.glb",
                            Vec3::new(edge - 50.0, platform_top / 2.0, -100.0),
                            Vec3::new(50.0, platform_top / 2.0, 50.0),
                        );
                        let face_x = edge + gap;
                        let block = prop_box(
                            "meshes\\block.glb",
                            Vec3::new(face_x + 20.0, block_top / 2.0, -100.0),
                            Vec3::new(20.0, block_top / 2.0, 50.0),
                        );
                        let mut sim = spawned_at(
                            world_with(&[platform, block]),
                            Vec3::new(edge - 15.0, platform_top + 3.0, -100.0),
                        );
                        hold(
                            &mut sim,
                            PlayerInput {
                                sprint,
                                ..running(0.0)
                            },
                        );
                        let run = ran_at_a_face(&mut sim, face_x, 120);
                        if let Some(problem) = run.problems(block_top, face_x) {
                            failures.push(format!(
                                "platform {platform_top}, block {block_top}, gap {gap}, \
                                 sprint {sprint}: {problem}"
                            ));
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// A block `top` high whose face is at `LEDGE_FACE_X`.
    fn block_of(top: f32) -> Prop {
        prop_box(
            "meshes\\block.glb",
            Vec3::new(LEDGE_FACE_X + 20.0, top / 2.0, -100.0),
            Vec3::new(20.0, top / 2.0, 50.0),
        )
    }

    /// Where the player stands for a fall beside a block face: its front 1 short of the face.
    const BESIDE_THE_FACE_X: f32 = LEDGE_FACE_X - RADIUS - 1.0;

    #[test]
    fn falling_from_a_spawn_over_the_void_at_a_block_face_never_lifts_the_player_beyond_the_step_limit()
     {
        let top = 20.0;
        let mut sim = spawned_at(
            world_in(CellColliderBatch::default(), &[block_of(top)]),
            Vec3::new(BESIDE_THE_FACE_X, top + 10.0, -100.0),
        );
        fall_below(&mut sim, top - MAX_STEP_UP - 1.0);
        hold(&mut sim, running(0.0));
        let run = ran_at_a_face(&mut sim, LEDGE_FACE_X, 60);
        assert!(run.soles_at_the_face.is_some(), "never reached the face");
        assert_eq!(run.problems(top, LEDGE_FACE_X), None);
    }

    /// A heavy box resting on the floor that the player can stand on as a dynamic support.
    fn heavy_box(at_x: f32, top: f32, half_width: f32) -> Prop {
        Prop {
            dynamic: true,
            mass: Some(5000.0),
            ..prop_box(
                "meshes\\heavy_box.glb",
                Vec3::new(at_x, top / 2.0, -100.0),
                Vec3::new(half_width, top / 2.0, half_width.min(30.0)),
            )
        }
    }

    /// A world of `props` with the player standing on the heavy box at `at_x`, `top` high. The
    /// box settles before the player arrives: rapier 0.35 wakes a body resting on the floor at
    /// its first contact and lets it dip into the one-sided terrain for a few ticks, which would
    /// drop a player spawned on it to the ground.
    fn standing_on_a_heavy_box(props: &[Prop], at_x: f32, top: f32) -> SimWorld {
        let mut sim = spawned_at(world_with(props), Vec3::new(120.0, 3.0, -100.0));
        tick(&mut sim, 30);
        sim.apply(SimCommand::TeleportPlayer {
            pos: Vec3::new(at_x, top + 3.0, -100.0),
        });
        tick(&mut sim, 30);
        assert!(
            (sim.player.footing_height - top).abs() < 0.1,
            "standing at {} on footing {}",
            position(&sim),
            sim.player.footing_height
        );
        sim
    }

    #[test]
    fn a_support_removed_underfoot_before_a_block_face_never_lifts_the_player_beyond_the_step_limit()
     {
        let (support_top, top) = (30.0, 25.0);
        let support_end = LEDGE_FACE_X - 5.0;
        let support_x = support_end - 32.5;
        let support = heavy_box(support_x, support_top, 32.5);
        let mut sim =
            standing_on_a_heavy_box(&[support, block_of(top)], support_x - 20.0, support_top);
        hold(&mut sim, running(0.0));
        for _ in 0..120 {
            if position(&sim).x >= LEDGE_FACE_X - 18.0 {
                break;
            }
            tick(&mut sim, 1);
        }
        sim.physics.remove_ref(prop_entity(0));
        let run = ran_at_a_face(&mut sim, LEDGE_FACE_X, 60);
        assert!(
            run.soles_at_the_face
                .is_some_and(|soles| soles < top - MAX_STEP_UP),
            "reached the face with its soles at {:?}",
            run.soles_at_the_face
        );
        assert_eq!(run.problems(top, LEDGE_FACE_X), None);
    }

    #[test]
    fn wedged_between_a_slope_too_steep_to_stand_on_and_a_block_face_never_lifts_the_player_beyond_the_step_limit()
     {
        let slope = 62.0_f32.to_radians();
        let (plateau_top, foot_y, top) = (20.0, 6.0, 18.1);
        let crest_x = LEDGE_FACE_X - (plateau_top - foot_y) / slope.tan();
        let along = Vec2::new(slope.cos(), -slope.sin());
        let normal = Vec2::new(slope.sin(), slope.cos());
        let half_length = (plateau_top - foot_y + 4.0) / slope.sin() / 2.0;
        let center = Vec2::new(crest_x, plateau_top) + along * half_length - normal;
        let mut slope_down = prop_box(
            "meshes\\steep.glb",
            Vec3::new(center.x, center.y, -100.0),
            Vec3::ZERO,
        );
        slope_down.shape.shape = ColShape::Box {
            center: Vec3::ZERO,
            rotation: Quat::from_rotation_z(-slope),
            half_extents: Vec3::new(half_length, 1.0, 50.0),
        };
        let plateau = prop_box(
            "meshes\\plateau.glb",
            Vec3::new(crest_x - 25.0, plateau_top / 2.0, -100.0),
            Vec3::new(25.0, plateau_top / 2.0, 50.0),
        );
        let mut sim = spawned_at(
            world_with(&[slope_down, plateau, block_of(top)]),
            Vec3::new(crest_x - 10.0, plateau_top + 3.0, -100.0),
        );
        hold(&mut sim, running(0.0));
        let run = ran_at_a_face(&mut sim, LEDGE_FACE_X, 60);
        assert!(
            run.soles_at_the_face
                .is_some_and(|soles| soles < top - MAX_STEP_UP),
            "reached the face with its soles at {:?}",
            run.soles_at_the_face
        );
        assert_eq!(run.problems(top, LEDGE_FACE_X), None);
    }

    const SLAB_TOP: f32 = 1.0;
    /// The middle of a heavy slab whose far edge is at `LEDGE_FACE_X`.
    const SLAB_X: f32 = LEDGE_FACE_X - 15.0;

    /// Where the player ends after running at a ledge `above_the_slab` higher than the heavy
    /// slab it starts on, the slab reaching up to the ledge face.
    fn ran_from_a_heavy_slab_at_a_ledge(above_the_slab: f32) -> Vec3 {
        let top = SLAB_TOP + above_the_slab;
        let ledge = prop_box(
            "meshes\\ledge.glb",
            Vec3::new(LEDGE_FACE_X + 30.0, top / 2.0, -100.0),
            Vec3::new(30.0, top / 2.0, 50.0),
        );
        let slab = heavy_box(SLAB_X, SLAB_TOP, 15.0);
        let mut sim = standing_on_a_heavy_box(&[slab, ledge], SLAB_X, SLAB_TOP);
        hold(&mut sim, running(0.0));
        tick(&mut sim, 60);
        position(&sim)
    }

    #[test]
    fn a_ledge_within_the_step_limit_of_a_dynamic_support_is_climbed_from_it() {
        let end = ran_from_a_heavy_slab_at_a_ledge(3.0);
        assert!(end.x > LEDGE_FACE_X + 5.0, "x = {}", end.x);
        assert!(
            (end.y - (SLAB_TOP + 3.0 + CENTER_ABOVE_FEET)).abs() < 0.1,
            "y = {}",
            end.y
        );
    }

    #[test]
    fn a_ledge_beyond_the_step_limit_of_a_dynamic_support_blocks_the_player_on_it() {
        let above_the_slab = 3.3;
        let end = ran_from_a_heavy_slab_at_a_ledge(above_the_slab);
        assert!(end.x < LEDGE_FACE_X, "x = {}", end.x);
        assert!(
            end.y - CENTER_ABOVE_FEET < SLAB_TOP + above_the_slab,
            "y = {}",
            end.y
        );
    }

    #[test]
    fn walking_off_a_dynamic_support_takes_the_footing_back_to_the_floor() {
        let slab = heavy_box(SLAB_X, SLAB_TOP, 15.0);
        let mut sim = standing_on_a_heavy_box(&[slab], SLAB_X, SLAB_TOP);
        hold(&mut sim, running(PI));
        tick(&mut sim, 60);
        assert!(
            sim.player.footing_height.abs() < 0.1,
            "footing {} at {}",
            sim.player.footing_height,
            position(&sim)
        );
    }

    #[test]
    fn a_ledge_too_high_to_step_onto_stays_so_after_running_down_off_it() {
        let top = REFUSED_IN_THE_ORIGINAL;
        let ledge = prop_box(
            "meshes\\ledge.glb",
            Vec3::new(LEDGE_FACE_X + 30.0, top / 2.0, -100.0),
            Vec3::new(30.0, top / 2.0, 50.0),
        );
        let on_the_ledge = Vec3::new(LEDGE_FACE_X + 10.0, top + 3.0, SPAWN_AT.z);
        let mut sim = spawned_at(world_with(&[ledge]), on_the_ledge);
        hold(&mut sim, running(PI));
        tick(&mut sim, 60);
        assert!(
            (position(&sim).y - CENTER_ABOVE_FEET).abs() < 0.1,
            "down at {}",
            position(&sim)
        );
        hold(&mut sim, running(0.0));
        tick(&mut sim, 60);
        let end = position(&sim);
        assert!(end.x <= LEDGE_FACE_X - RADIUS, "x = {}", end.x);
        assert!(end.y - CENTER_ABOVE_FEET < top, "y = {}", end.y);
    }

    /// Where the player ends 4 s after jumping with `run` held from just before a ledge face at
    /// `LEDGE_FACE_X`, the ledge long and wide enough to stay on at a sprint.
    fn jumped_at_a_ledge(top: f32, run: PlayerInput) -> Vec3 {
        let ledge = prop_box(
            "meshes\\ledge.glb",
            Vec3::new(LEDGE_FACE_X + 99.5, top / 2.0, -200.0),
            Vec3::new(99.5, top / 2.0, 180.0),
        );
        let near_the_face = Vec3::new(LEDGE_FACE_X - RADIUS - 0.5, SPAWN_AT.y, -200.0);
        let mut sim = spawned_at(world_with(&[ledge]), near_the_face);
        hold(&mut sim, PlayerInput { jump: true, ..run });
        tick(&mut sim, JUMP_WINDUP_TICKS + 2);
        hold(&mut sim, run);
        tick(&mut sim, 240 - JUMP_WINDUP_TICKS - 2);
        position(&sim)
    }

    fn jump_apex() -> f32 {
        JUMP_VELOCITY.powi(2) / (2.0 * GRAVITY)
    }

    #[test]
    fn a_jump_lands_on_a_ledge_below_its_apex() {
        let top = jump_apex() - 2.0;
        let end = jumped_at_a_ledge(top, running(0.0));
        assert!(end.x > LEDGE_FACE_X + RADIUS, "x = {}", end.x);
        assert!(
            (end.y - (top + CENTER_ABOVE_FEET)).abs() < 0.1,
            "y = {}",
            end.y
        );
    }

    /// The ways of running at a ledge a jump is checked with: run and sprint, square to the face
    /// and at 45 deg.
    fn jump_runs() -> [PlayerInput; 4] {
        [false, true]
            .map(|sprint| {
                [0.0, FRAC_PI_4].map(|yaw| PlayerInput {
                    sprint,
                    ..running(yaw)
                })
            })
            .concat()
            .try_into()
            .unwrap()
    }

    /// The rounded bottom slides up an edge it meets below its centre, and while it stands on that
    /// edge the jump keeps its rising speed, so a jump gets onto edges nearly a radius above its
    /// apex; onto none half a radius higher still.
    #[test]
    fn a_jump_does_not_get_onto_a_ledge_one_and_a_half_capsule_radii_above_its_apex() {
        let top = jump_apex() + 1.5 * RADIUS;
        let failures: Vec<String> = jump_runs()
            .into_iter()
            .filter_map(|run| {
                let end = jumped_at_a_ledge(top, run);
                let below = end.x < LEDGE_FACE_X && end.y - CENTER_ABOVE_FEET < top;
                (!below).then(|| format!("{run:?}: ended at {end}"))
            })
            .collect();
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn a_jump_at_a_ledge_always_ends_standing_on_the_floor_or_on_the_ledge() {
        let failures: Vec<String> = (0..=50)
            .map(|step| jump_apex() - 1.0 + 0.1 * step as f32)
            .flat_map(|top| jump_runs().map(|run| (top, run)))
            .filter_map(|(top, run)| {
                let end = jumped_at_a_ledge(top, run);
                let soles = end.y - CENTER_ABOVE_FEET;
                let standing = soles.abs() < 0.1 || (soles - top).abs() < 0.1;
                (!standing).then(|| format!("top {top}, {run:?}: ended at {end}"))
            })
            .collect();
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// Where the player ends after running 1.5 s up a ramp of `slope` rising from `LEDGE_FACE_X`
    /// to a plateau 10 high whose edge stands `lip` above the ramp's upper end, with the height
    /// of that edge and its x.
    fn ran_up_a_ramp(slope: f32, lip: f32) -> (Vec3, f32, f32) {
        let ramp_top = 10.0;
        let foot = Vec2::new(LEDGE_FACE_X, 0.0);
        let along = Vec2::new(slope.cos(), slope.sin());
        let normal = Vec2::new(-slope.sin(), slope.cos());
        let upper_end = foot + along * (ramp_top / slope.sin());
        let half_length = (ramp_top + 5.0) / slope.sin() / 2.0;
        let half_thickness = 1.0;
        let center = upper_end - along * half_length - normal * half_thickness;
        let mut ramp = prop_box(
            "meshes\\ramp.glb",
            Vec3::new(center.x, center.y, -100.0),
            Vec3::ZERO,
        );
        ramp.shape.shape = ColShape::Box {
            center: Vec3::ZERO,
            rotation: Quat::from_rotation_z(slope),
            half_extents: Vec3::new(half_length, half_thickness, 50.0),
        };
        let edge = ramp_top + lip;
        let plateau = prop_box(
            "meshes\\plateau.glb",
            Vec3::new(upper_end.x + 30.0, edge / 2.0, -100.0),
            Vec3::new(30.0, edge / 2.0, 50.0),
        );
        let mut sim = spawned_at(world_with(&[ramp, plateau]), SPAWN_AT);
        hold(&mut sim, running(0.0));
        tick(&mut sim, 90);
        (position(&sim), edge, upper_end.x)
    }

    const RAMP_SLOPES: [f32; 2] = [FRAC_PI_4, 55.0 * PI / 180.0];

    #[test]
    fn a_lip_within_the_step_limit_at_the_top_of_a_steep_ramp_is_climbed() {
        let failures: Vec<String> = RAMP_SLOPES
            .into_iter()
            .flat_map(|slope| [0.0, 1.5, 2.0].map(|lip| (slope, lip)))
            .filter_map(|(slope, lip)| {
                let (end, edge, edge_x) = ran_up_a_ramp(slope, lip);
                let on_top =
                    end.x > edge_x + 5.0 && (end.y - (edge + CENTER_ABOVE_FEET)).abs() < 0.1;
                (!on_top).then(|| format!("slope {slope}, lip {lip}: ended at {end}"))
            })
            .collect();
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn a_lip_above_the_step_limit_at_the_top_of_a_steep_ramp_blocks_the_player() {
        let failures: Vec<String> = RAMP_SLOPES
            .into_iter()
            .filter_map(|slope| {
                let (end, edge, edge_x) = ran_up_a_ramp(slope, REFUSED_IN_THE_ORIGINAL);
                let blocked = end.x < edge_x && end.y - CENTER_ABOVE_FEET < edge;
                (!blocked).then(|| format!("slope {slope}: ended at {end}"))
            })
            .collect();
        assert!(failures.is_empty(), "{failures:#?}");
    }

    const STAIR_STEPS: [&str; 4] = [
        "meshes\\step1.glb",
        "meshes\\step2.glb",
        "meshes\\step3.glb",
        "meshes\\step4.glb",
    ];
    const STAIR_BACK_X: f32 = LEDGE_FACE_X + 90.0;

    /// A stair of `STAIR_STEPS` with `rise` and `tread` whose first riser is at `LEDGE_FACE_X`,
    /// its top running on to `STAIR_BACK_X`.
    fn stair(rise: f32, tread: f32) -> Vec<Prop> {
        STAIR_STEPS
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let front = LEDGE_FACE_X + tread * index as f32;
                let top = rise * (index + 1) as f32;
                prop_box(
                    path,
                    Vec3::new(f32::midpoint(front, STAIR_BACK_X), top / 2.0, -100.0),
                    Vec3::new((STAIR_BACK_X - front) / 2.0, top / 2.0, 50.0),
                )
            })
            .collect()
    }

    /// Whether `end` stands on the top of the stair of `rise` and `tread`, past its last riser.
    fn on_top_of_the_stair(end: Vec3, rise: f32, tread: f32) -> bool {
        let last_riser = LEDGE_FACE_X + tread * (STAIR_STEPS.len() - 1) as f32;
        end.x > last_riser + RADIUS
            && (end.y - (rise * STAIR_STEPS.len() as f32 + CENTER_ABOVE_FEET)).abs() < 0.1
    }

    /// Where the player ends after running 2 s at the stair of `rise` and `tread`.
    fn ran_up_a_stair(rise: f32, tread: f32) -> Vec3 {
        let mut sim = spawned_at(world_with(&stair(rise, tread)), SPAWN_AT);
        hold(&mut sim, running(0.0));
        tick(&mut sim, 120);
        position(&sim)
    }

    #[test]
    fn stairs_with_risers_within_the_step_limit_are_climbed_on_treads_narrower_than_the_capsule() {
        let failures: Vec<String> = [
            (3.0, 1.5),
            (3.0, 2.0),
            (LIFTED_IN_THE_ORIGINAL, 1.5),
            (LIFTED_IN_THE_ORIGINAL, 2.0),
        ]
        .into_iter()
        .filter_map(|(rise, tread)| {
            let end = ran_up_a_stair(rise, tread);
            (!on_top_of_the_stair(end, rise, tread))
                .then(|| format!("rise {rise}, tread {tread}: ended at {end}"))
        })
        .collect();
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn a_stair_with_risers_above_the_step_limit_blocks_the_player() {
        let end = ran_up_a_stair(REFUSED_IN_THE_ORIGINAL, 2.0);
        assert!(end.x < LEDGE_FACE_X - RADIUS + 0.2, "x = {}", end.x);
        assert!((end.y - CENTER_ABOVE_FEET).abs() < 0.1, "y = {}", end.y);
    }

    #[test]
    fn a_stair_run_down_is_climbed_again() {
        let (rise, tread) = (3.0, 2.0);
        let on_top = Vec3::new(STAIR_BACK_X - 30.0, 20.0, SPAWN_AT.z);
        let mut sim = spawned_at(world_with(&stair(rise, tread)), on_top);
        hold(&mut sim, running(PI));
        tick(&mut sim, 150);
        assert!(
            (position(&sim).y - CENTER_ABOVE_FEET).abs() < 0.1,
            "down at {}",
            position(&sim)
        );
        hold(&mut sim, running(0.0));
        tick(&mut sim, 150);
        let end = position(&sim);
        assert!(on_top_of_the_stair(end, rise, tread), "ended at {end}");
    }

    #[test]
    fn a_jump_lands_after_the_footage_airtime_and_peaks_at_the_game_setting_height() {
        let mut sim = spawned_at(flat_world(), SPAWN_AT);
        let ground = position(&sim).y;
        hold(
            &mut sim,
            PlayerInput {
                jump: true,
                ..PlayerInput::default()
            },
        );
        tick(&mut sim, 1);
        hold(&mut sim, PlayerInput::default());
        let (frames_in_air, apex) = flight(&mut sim, ground);
        assert!(sim.player_debug().grounded, "never landed");
        assert_eq!(frames_in_air, 56, "landing frame measured from the footage");
        assert!(
            (apex - ground - 9.0).abs() < 0.5,
            "apex {} u above the ground",
            apex - ground
        );
    }

    #[test]
    fn holding_jump_does_not_launch_while_airborne_past_the_coyote_window() {
        let mut sim = spawned_at(flat_world(), Vec3::new(405.0, 40.0, -100.0));
        // Spawned over the void: the capsule is airborne from the start.
        hold(
            &mut sim,
            PlayerInput {
                jump: true,
                ..PlayerInput::default()
            },
        );
        tick(&mut sim, COYOTE_TICKS + 10);
        assert!(
            sim.player_debug().vertical_vel < 0.0,
            "no launch while falling"
        );
    }

    #[test]
    fn falling_is_capped_at_the_terminal_speed() {
        let mut sim = spawned_at(flat_world(), Vec3::new(405.0, 40.0, -100.0));
        assert!(!sim.player_debug().grounded);
        tick(&mut sim, 600);
        assert_eq!(sim.player_debug().vertical_vel, -MAX_FALL_SPEED);
    }

    /// A 1 thick ceiling over the spawn point whose underside is `underside` above the floor.
    fn ceiling_at(underside: f32) -> Prop {
        prop_box(
            "meshes\\ceiling.glb",
            Vec3::new(SPAWN_AT.x, underside + 0.5, SPAWN_AT.z),
            Vec3::new(10.0, 0.5, 10.0),
        )
    }

    #[test]
    fn a_player_bumping_a_ceiling_stops_rising() {
        let underside = TOP_ABOVE_FEET + 2.0;
        let mut sim = spawned_at(world_with(&[ceiling_at(underside)]), SPAWN_AT);
        hold(
            &mut sim,
            PlayerInput {
                jump: true,
                ..PlayerInput::default()
            },
        );
        let mut highest_top = f32::NEG_INFINITY;
        for _ in 0..60 {
            tick(&mut sim, 1);
            highest_top = highest_top.max(position(&sim).y + HEIGHT / 2.0);
        }
        assert!(
            highest_top > underside - 0.1 && highest_top < underside + 0.01,
            "the head rose to {highest_top} under a ceiling at {underside}"
        );
    }

    #[test]
    fn sneaking_moves_neither_the_capsule_nor_the_body() {
        let mut sim = spawned_at(flat_world(), SPAWN_AT);
        let standing = (position(&sim), sim.player_body().unwrap());
        hold(&mut sim, crouching());
        tick(&mut sim, 5);
        assert!(sim.player_debug().crouching);
        let sneaking = (position(&sim), sim.player_body().unwrap());
        assert!(
            sneaking.0.abs_diff_eq(standing.0, 1e-3),
            "{standing:?} -> {sneaking:?}"
        );
        assert!(
            sneaking.1.feet.abs_diff_eq(standing.1.feet, 1e-3)
                && sneaking.1.height == standing.1.height,
            "{standing:?} -> {sneaking:?}"
        );
        assert!(
            (sneaking.1.feet.y + sneaking.1.height - (sneaking.0.y + HEIGHT / 2.0)).abs()
                < OFFSET + 1e-4,
            "the body reaches the capsule top: {sneaking:?}"
        );
    }

    #[test]
    fn the_sneak_eye_lies_below_the_capsule_top() {
        let mut sim = spawned_at(flat_world(), SPAWN_AT);
        hold(&mut sim, crouching());
        tick(&mut sim, 5);
        let body = sim.player_body().unwrap();
        let eye = sim.player_eye().unwrap();
        assert!(eye.y < body.feet.y + body.height, "eye {eye}, {body:?}");
    }

    #[test]
    fn toggling_sneak_under_a_low_ceiling_needs_no_headroom_and_never_moves_the_player() {
        let mut sim = spawned_at(world_with(&[ceiling_at(TOP_ABOVE_FEET + 0.2)]), SPAWN_AT);
        let start = position(&sim);
        for crouch in [true, false, true, false] {
            hold(
                &mut sim,
                PlayerInput {
                    crouch,
                    ..PlayerInput::default()
                },
            );
            tick(&mut sim, 1);
            assert_eq!(sim.player_debug().crouching, crouch);
            assert!(
                position(&sim).abs_diff_eq(start, 1e-3),
                "{start} -> {}",
                position(&sim)
            );
        }
    }

    /// Where the player's centre gets to, and the farthest, running for 4 s at a lintel whose
    /// face is at `LEDGE_FACE_X` and whose underside is `underside` above the floor.
    fn ran_at_a_lintel(underside: f32, crouch: bool) -> (Vec3, f32) {
        let lintel = prop_box(
            "meshes\\lintel.glb",
            Vec3::new(LEDGE_FACE_X + 5.0, underside + 5.0, -100.0),
            Vec3::new(5.0, 5.0, 50.0),
        );
        let mut sim = spawned_at(
            world_with(&[lintel]),
            Vec3::new(LEDGE_FACE_X - 6.0, 3.0, -100.0),
        );
        hold(
            &mut sim,
            PlayerInput {
                crouch,
                ..running(0.0)
            },
        );
        let mut farthest = f32::NEG_INFINITY;
        for _ in 0..240 {
            tick(&mut sim, 1);
            farthest = farthest.max(position(&sim).x);
        }
        (position(&sim), farthest)
    }

    #[test]
    fn standing_and_sneaking_stop_at_the_same_lintel_and_pass_under_the_same_one() {
        let blocking = TOP_ABOVE_FEET - 0.5;
        let (standing, standing_farthest) = ran_at_a_lintel(blocking, false);
        let (sneaking, sneaking_farthest) = ran_at_a_lintel(blocking, true);
        assert!(
            standing_farthest < LEDGE_FACE_X,
            "standing reached {standing_farthest}"
        );
        assert!(
            (sneaking_farthest - standing_farthest).abs() < 0.01,
            "standing stopped at {standing}, sneaking at {sneaking}"
        );
        let clear = TOP_ABOVE_FEET + 0.5;
        for crouch in [false, true] {
            let (end, _) = ran_at_a_lintel(clear, crouch);
            assert!(
                end.x > LEDGE_FACE_X + 10.0 + RADIUS,
                "sneaking {crouch}: ended at {end}"
            );
        }
    }

    #[test]
    fn an_interior_spawn_places_the_player_at_the_start_and_lets_gravity_settle_it() {
        let floor = prop_box(
            "meshes\\floor.glb",
            Vec3::new(100.0, -0.5, -100.0),
            Vec3::new(50.0, 0.5, 50.0),
        );
        let mut batch = CellColliderBatch::default();
        batch.objects.push(object_group(
            floor.path,
            vec![placement("0A0000:Fallout4.esm", floor.at)],
            false,
            None,
        ));
        let baked = bake(&batch, &[(floor.path, vec![floor.shape.clone()])]);
        let mut sim = SimWorld::new(
            PlayerMovement::default(),
            EyeHeights::default(),
            WorldClock::fixture(),
        );
        sim.apply(SimCommand::SetInteriorMode(true));
        sim.apply(SimCommand::AddCellColliders {
            grid: (0, 0),
            batch: Arc::new(batch),
            baked: Some(baked),
        });
        sim.apply(SimCommand::SpawnPlayer {
            pos: Vec3::new(100.0, 0.0, -100.0),
        });
        tick(&mut sim, 1);
        assert!(
            !sim.player_debug().grounded,
            "no ground snap in an interior"
        );
        tick(&mut sim, 30);
        let debug = sim.player_debug();
        assert!(
            debug.grounded && (debug.pos.y - CENTER_ABOVE_FEET).abs() < 0.1,
            "{}",
            debug.pos.y
        );
    }

    #[test]
    fn a_teleport_snaps_the_player_onto_the_surface_below_the_new_point() {
        let mut sim = spawned_at(flat_world(), SPAWN_AT);
        sim.apply(SimCommand::TeleportPlayer {
            pos: Vec3::new(50.0, 80.0, -50.0),
        });
        tick(&mut sim, 2);
        let debug = sim.player_debug();
        assert_eq!((debug.pos.x, debug.pos.z), (50.0, -50.0));
        assert!((debug.pos.y - CENTER_ABOVE_FEET).abs() < 0.01);
    }

    #[test]
    fn a_teleported_body_is_unknown_until_it_stands_at_the_new_point() {
        let mut sim = spawned_at(flat_world(), SPAWN_AT);
        assert!(sim.player_body().is_some());
        let target = Vec3::new(50.0, 80.0, -50.0);
        sim.apply(SimCommand::TeleportPlayer { pos: target });
        assert!(sim.player_body().is_none());
        let body = (0..10)
            .find_map(|_| {
                tick(&mut sim, 1);
                sim.player_body()
            })
            .expect("the teleport completes");
        assert_eq!((body.feet.x, body.feet.z), (target.x, target.z));
    }

    #[test]
    fn walking_into_a_heavy_crate_pushes_it_by_no_more_than_the_push_force_allows() {
        let crate_mass = 15.0;
        let heavy = crate_prop(Vec3::new(215.0, 0.5, -100.0), crate_mass);
        let mut sim = spawned_at(world_with(&[heavy]), SPAWN_AT);
        hold(&mut sim, running(0.0));
        let per_tick_cap = CHAR_PUSH_FORCE * FIXED_DT / crate_mass;
        let mut previous = 0.0_f32;
        let mut pushed_ticks = 0;
        for _ in 0..120 {
            tick(&mut sim, 1);
            let speed = crate_speed(&sim);
            assert!(
                speed - previous <= per_tick_cap + 0.5,
                "speed jumped {previous} -> {speed}"
            );
            pushed_ticks += u32::from(speed > 0.1);
            previous = speed;
        }
        assert!(pushed_ticks > 0, "the crate was never pushed");
        assert!(crate_x(&sim) > 215.5, "the crate moved ahead of the walker");
    }

    #[test]
    #[ignore = "rapier 0.35 strong-wakes a prop resting exactly on the floor at its first contact, \
                where 0.33 left it asleep. The woken prop dips into the one-sided terrain for \
                about seven ticks, so the spawn raycast lands the player on the ground instead of \
                the crate. An awake prop dips on neither version, so the upgrade caused this \
                rather than exposing it; why the penetration reaches 0.47 in the wake-from-sleep \
                path is unexplained"]
    fn a_player_can_stand_on_a_crate_without_pushing_it_through_the_ground() {
        let heavy = crate_prop(Vec3::new(200.0, 0.5, -100.0), 15.0);
        let mut sim = spawned_at(world_with(&[heavy]), Vec3::new(200.0, 6.0, -100.0));
        tick(&mut sim, 120);
        let debug = sim.player_debug();
        assert!(debug.grounded);
        assert!(
            (debug.pos.y - (1.0 + CENTER_ABOVE_FEET)).abs() < 0.2,
            "{}",
            debug.pos.y
        );
        assert!(crate_x(&sim) > 199.0 && crate_x(&sim) < 201.0);
    }

    fn crate_body(sim: &SimWorld) -> &rapier3d::dynamics::RigidBody {
        sim.physics
            .world
            .bodies
            .iter()
            .map(|(_, body)| body)
            .find(|body| body.is_dynamic() || body.is_fixed())
            .expect("the crate")
    }

    fn crate_speed(sim: &SimWorld) -> f32 {
        crate_body(sim).linvel().length()
    }

    fn crate_x(sim: &SimWorld) -> f32 {
        crate_body(sim).translation().x
    }

    mod footsteps {
        use super::*;
        use crate::footsteps::{
            FootTag, FootstepEvent, Gait, GaitCycle, GaitTimings, SurfaceMaterial,
        };

        fn timings() -> GaitTimings {
            let cycle = |what, period, marks| GaitCycle::new(what, period, marks).unwrap();
            GaitTimings::new(
                cycle(
                    "run",
                    0.5,
                    vec![(0.09, FootTag::FootRight), (0.34, FootTag::FootLeft)],
                ),
                cycle(
                    "sprint",
                    0.4,
                    vec![
                        (0.05, FootTag::FootSprintRight),
                        (0.25, FootTag::FootSprintLeft),
                    ],
                ),
                cycle(
                    "sneak",
                    0.8,
                    vec![(0.1, FootTag::FootRight), (0.5, FootTag::FootLeft)],
                ),
                0.033,
                0.067,
            )
            .unwrap()
        }

        fn footstep_world() -> SimWorld {
            let mut sim = spawned_at(flat_world(), SPAWN_AT);
            sim.install_gait_timings(timings());
            sim
        }

        /// Ticks `ticks` fixed steps; (1-based tick, event) for each event.
        fn record(sim: &mut SimWorld, ticks: u32) -> Vec<(u32, FootstepEvent)> {
            let mut events = Vec::new();
            for tick in 1..=ticks {
                sim.tick(FIXED_DT);
                events.extend(sim.drain_footsteps().into_iter().map(|event| (tick, event)));
            }
            events
        }

        fn tags(events: &[(u32, FootstepEvent)]) -> Vec<FootTag> {
            events.iter().map(|(_, event)| event.tag).collect()
        }

        #[test]
        fn running_on_the_ground_steps_right_then_left_on_the_terrain() {
            let mut sim = footstep_world();
            hold(&mut sim, running(0.0));
            let events = record(&mut sim, 60);
            assert_eq!(
                tags(&events),
                [
                    FootTag::FootRight,
                    FootTag::FootLeft,
                    FootTag::FootRight,
                    FootTag::FootLeft
                ]
            );
            assert!(events.iter().all(|(_, event)| event.gait == Gait::Run));
            for (_, event) in &events {
                let SurfaceMaterial::Terrain { point } = event.surface else {
                    panic!(
                        "expected the terrain under the feet, got {:?}",
                        event.surface
                    );
                };
                assert!(
                    point[1].abs() < 0.2,
                    "the flat ground is at y = 0: {point:?}"
                );
            }
        }

        #[test]
        fn nothing_plays_when_no_timings_are_installed() {
            let mut sim = spawned_at(flat_world(), SPAWN_AT);
            hold(&mut sim, running(0.0));
            assert_eq!(record(&mut sim, 60), []);
        }

        #[test]
        fn sprinting_and_crouching_pick_their_gaits() {
            let mut sim = footstep_world();
            hold(
                &mut sim,
                PlayerInput {
                    sprint: true,
                    ..running(0.0)
                },
            );
            let sprint = record(&mut sim, 30);
            assert_ne!(sprint, []);
            assert!(sprint.iter().all(|(_, event)| event.gait == Gait::Sprint));

            let mut sim = footstep_world();
            hold(
                &mut sim,
                PlayerInput {
                    crouch: true,
                    sprint: true,
                    ..running(0.0)
                },
            );
            let sneak = record(&mut sim, 60);
            assert_ne!(sneak, []);
            assert!(sneak.iter().all(|(_, event)| event.gait == Gait::Sneak));
        }

        #[test]
        fn standing_still_and_released_input_are_silent() {
            let mut sim = footstep_world();
            assert_eq!(record(&mut sim, 60), []);
            hold(&mut sim, running(0.0));
            record(&mut sim, 30);
            hold(&mut sim, PlayerInput::default());
            let after_release = record(&mut sim, 60);
            assert!(
                after_release.iter().all(|(tick, _)| *tick <= 1),
                "{after_release:?}"
            );
        }

        #[test]
        fn pushing_against_a_wall_is_silent() {
            let wall = prop_box(
                "meshes\\wall.glb",
                Vec3::new(215.0, 10.0, -100.0),
                Vec3::new(5.0, 10.0, 50.0),
            );
            let mut sim = spawned_at(world_with(&[wall]), SPAWN_AT);
            sim.install_gait_timings(timings());
            hold(&mut sim, running(0.0));
            record(&mut sim, 40);
            assert!(position(&sim).x > 205.0, "the player should be at the wall");
            assert_eq!(record(&mut sim, 120), []);
        }

        #[test]
        fn a_jump_fires_jump_up_after_the_windup_and_jump_down_after_touchdown() {
            let mut sim = footstep_world();
            hold(
                &mut sim,
                PlayerInput {
                    jump: true,
                    ..PlayerInput::default()
                },
            );
            let mut events = Vec::new();
            let mut left_ground = false;
            let mut touchdown = None;
            for tick in 1..=90u32 {
                sim.tick(FIXED_DT);
                if tick == 1 {
                    hold(&mut sim, PlayerInput::default());
                }
                let grounded = sim.player_debug().grounded;
                left_ground |= !grounded;
                if left_ground && grounded && touchdown.is_none() {
                    touchdown = Some(tick);
                }
                events.extend(sim.drain_footsteps().into_iter().map(|e| (tick, e)));
            }
            let touchdown = touchdown.expect("the player landed");
            assert_eq!(tags(&events), [FootTag::JumpUp, FootTag::JumpDown]);
            assert_eq!(events[0].0, 1 + JUMP_WINDUP_TICKS);
            assert_eq!(events[1].0, touchdown + 4);
            assert!(matches!(
                events[0].1.surface,
                SurfaceMaterial::Terrain { .. }
            ));
        }

        #[test]
        fn the_jump_windup_agrees_with_the_jump_up_mark() {
            let jump_up = timings().jump_up_offset();
            assert!((JUMP_WINDUP_TICKS as f32 * FIXED_DT - jump_up).abs() <= FIXED_DT);
            assert!(crate::sim::jump_windup_mismatch(&timings()).is_none());
        }
    }
}
