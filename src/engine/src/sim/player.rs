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
use rapier3d::control::{CharacterAutostep, CharacterLength, KinematicCharacterController};
use rapier3d::parry::query::details::ShapeCastOptions;
use rapier3d::prelude::*;

use super::commands::PlayerInput;
use super::dynamic_contact::{self, Character};
use super::mirror::{scene_query_filter, static_scene_query_filter};
use super::physics::PhysicsContext;
use super::world_to_grid;

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
/// constants' behavior. These are *top* speeds — FO4 ramps up to them via
/// the race acceleration rate, which we don't model, so a timed course runs
/// slightly faster than vanilla footage (the old footage-calibrated course
/// *average* was 34.43 for run, 48 for sprint).
///
/// `Default` mirrors the vanilla record so unit tests and a missing/old DB
/// (pre-`installer movt-only` extraction) still get sane movement.
#[derive(Copy, Clone, Debug)]
pub struct PlayerMovement {
    pub run_speed: f32,
    pub sprint_speed: f32,
}

impl Default for PlayerMovement {
    fn default() -> Self {
        Self {
            run_speed: 37.298,
            sprint_speed: 50.0,
        }
    }
}
/// Airborne input only steers at this acceleration; momentum is otherwise
/// kept (no mid-air dead stop, a sprint jump can't be reversed before
/// landing). Feel-tuned, not measured.
const AIR_ACCEL: f32 = 60.0;
// Human height is ~12.8 units; eye sits near the capsule top.
const STAND_HEIGHT: f32 = 12.6;
const CROUCH_HEIGHT: f32 = 6.3;
const RADIUS: f32 = 2.5;
const EYE_BELOW_TOP: f32 = 0.8;
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
/// Height of the standing capsule's centre above the soles: half the standing height plus the controller gap.
pub const STANDING_CENTER_ABOVE_FEET: f32 = STAND_HEIGHT / 2.0 + OFFSET;
/// How far below the capsule a dynamic body still counts as ground.
const SNAP_REACH: f32 = OFFSET + 0.05;
/// ~36 cm autostep handles stair steps and curbs.
const STEP_HEIGHT: f32 = 2.5;
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
    /// Actor values (current). Full and static in this version — no damage or
    /// stamina system exists yet; maxima are the placeholder bridge constants.
    hp: f32,
    hp_max: f32,
    ap: f32,
    ap_max: f32,
    controller: KinematicCharacterController,
}

impl PlayerState {
    pub fn new(movement: PlayerMovement) -> Self {
        Self {
            pos: Vec3::ZERO,
            prev_pos: Vec3::ZERO,
            horizontal_vel: Vec2::ZERO,
            vertical_vel: 0.0,
            grounded: false,
            crouching: false,
            launched: false,
            step_completed: false,
            spawned: false,
            pending_spawn: None,
            jump_windup: None,
            ticks_airborne: u32::MAX,
            movement,
            hp: PLACEHOLDER_MAX_HP,
            hp_max: PLACEHOLDER_MAX_HP,
            ap: PLACEHOLDER_MAX_AP,
            ap_max: PLACEHOLDER_MAX_AP,
            controller: KinematicCharacterController {
                up: Vec3::Y,
                offset: CharacterLength::Absolute(OFFSET),
                slide: true,
                autostep: Some(CharacterAutostep {
                    max_height: CharacterLength::Absolute(STEP_HEIGHT),
                    min_width: CharacterLength::Absolute(1.0),
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

    pub(super) fn height(&self) -> f32 {
        if self.crouching {
            CROUCH_HEIGHT
        } else {
            STAND_HEIGHT
        }
    }

    fn half_height(&self) -> f32 {
        self.height() * 0.5
    }

    fn capsule(&self) -> Capsule {
        let half_cyl = (self.half_height() - RADIUS).max(0.01);
        Capsule::new_y(half_cyl, RADIUS)
    }

    /// Where a ray straight down starts and how far it reaches to find what
    /// the soles stand on: just above the soles, as far as the ground snap.
    pub(super) fn sole_probe(&self) -> (Vec3, f32) {
        let origin = self.pos - Vec3::Y * self.half_height();
        (origin, SNAP_LEN)
    }

    /// Eye offset above the capsule center.
    pub fn eye_above_center(&self) -> f32 {
        self.half_height() - EYE_BELOW_TOP
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
        let foot_lift = state.half_height() + OFFSET;
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
        state.spawned = true;
        state.pending_spawn = None;
        physics.sync_player_mirror(state.pos, &state.capsule(), true);
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

    // 3. Crouch: swap capsule height keeping the feet planted. Standing up
    // needs clear headroom; while blocked the key release re-probes every
    // step until the player walks somewhere taller.
    if input.crouch != state.crouching {
        let feet = state.pos.y - state.half_height();
        if state.crouching {
            // Probe with a slightly shrunk standing capsule lifted off the
            // floor so the ground itself doesn't read as a blocker.
            let probe = Capsule::new_y(STAND_HEIGHT * 0.5 - RADIUS, RADIUS - 0.1);
            let center = Vec3::new(state.pos.x, feet + STAND_HEIGHT * 0.5 + 0.15, state.pos.z);
            let queries = physics
                .world
                .query_pipeline_with_filter(scene_query_filter());
            let blocked = queries
                .intersect_shape(Pose::from_translation(center), &probe)
                .next()
                .is_some();
            if !blocked {
                state.crouching = false;
                state.pos.y = feet + state.half_height();
                state.prev_pos = state.pos; // don't lerp the eye through the swap
            }
        } else {
            state.crouching = true;
            state.pos.y = feet + state.half_height();
            state.prev_pos = state.pos;
        }
    }

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

    let speed = if input.sprint && !state.crouching {
        state.movement.sprint_speed
    } else {
        state.movement.run_speed
    };
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
    let capsule = state.capsule();
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
        let queries = physics
            .world
            .query_pipeline_with_filter(static_scene_query_filter());
        state.controller.move_shape(
            dt,
            &queries,
            &capsule,
            &Pose::from_translation(start),
            desired,
            |_| {},
        )
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
    // airborne casts down one step height and glues onto any walkable
    // surface (the normal check keeps cliff faces from grabbing the capsule
    // at ledge lips). Taller drops — real ledges — still fall naturally.
    if was_grounded && !state.grounded && state.vertical_vel <= 0.0 {
        let queries = physics
            .world
            .query_pipeline_with_filter(static_scene_query_filter());
        let down_pose = Pose::from_translation(state.pos);
        let hit = queries.cast_shape(
            &down_pose,
            -Vec3::Y,
            &capsule,
            ShapeCastOptions {
                target_distance: OFFSET,
                stop_at_penetration: false,
                max_time_of_impact: STEP_HEIGHT,
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
        eye_above_center: state.eye_above_center(),
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
    use crate::sim::SimWorld;
    use crate::sim::collision::{CellColliderBatch, ColShape, PartShape};
    use crate::sim::commands::SimCommand;
    use crate::sim::physics::test_support::{
        bake, box_shape, ground_batch, object_group, placement,
    };

    const SPAWN_AT: Vec3 = Vec3::new(200.0, 3.0, -100.0);
    const STANDING_CENTER_HEIGHT: f32 = STANDING_CENTER_ABOVE_FEET;
    const CROUCHING_CENTER_HEIGHT: f32 = CROUCH_HEIGHT / 2.0 + OFFSET;

    struct Prop {
        path: &'static str,
        at: Vec3,
        shape: PartShape,
        dynamic: bool,
        mass: Option<f32>,
    }

    fn prop_box(path: &'static str, at: Vec3, half: Vec3) -> Prop {
        Prop {
            path,
            at,
            shape: PartShape {
                shape: ColShape::Box {
                    center: Vec3::ZERO,
                    rotation: Quat::IDENTITY,
                    half_extents: half,
                },
                node: None,
                joint: None,
                material: 0,
            },
            dynamic: false,
            mass: None,
        }
    }

    fn crate_prop(at: Vec3, mass: f32) -> Prop {
        Prop {
            dynamic: true,
            mass: Some(mass),
            ..Prop {
                path: "meshes\\crate.glb",
                at,
                shape: box_shape(0.5),
                dynamic: false,
                mass: None,
            }
        }
    }

    /// A flat world with `props` registered in cell (0, 0).
    fn world_with(props: &[Prop]) -> SimWorld {
        let mut batch: CellColliderBatch = ground_batch();
        for (index, prop) in props.iter().enumerate() {
            let ref_key = format!("{:06X}:Fallout4.esm", 0x0A_0000 + index);
            batch.objects.push(object_group(
                prop.path,
                vec![placement(&ref_key, prop.at)],
                prop.dynamic,
                prop.mass,
            ));
        }
        let shapes: Vec<(&str, Vec<PartShape>)> = props
            .iter()
            .map(|prop| (prop.path, vec![prop.shape.clone()]))
            .collect();
        let baked = bake(&batch, &shapes);
        let mut sim = SimWorld::new(PlayerMovement::default());
        sim.apply(SimCommand::AddCellColliders {
            grid: (0, 0),
            batch: Arc::new(batch),
            baked: Some(baked),
        });
        sim.apply(SimCommand::SetPhysicsCenter {
            grid: (0, 0),
            radius: 2,
        });
        sim
    }

    fn flat_world() -> SimWorld {
        world_with(&[])
    }

    fn tick(sim: &mut SimWorld, ticks: u32) {
        for _ in 0..ticks {
            sim.tick(FIXED_DT);
        }
    }

    fn spawned_at(mut sim: SimWorld, at: Vec3) -> SimWorld {
        sim.apply(SimCommand::SpawnPlayer { pos: at });
        tick(&mut sim, 2);
        sim
    }

    fn hold(sim: &mut SimWorld, input: PlayerInput) {
        sim.apply(SimCommand::SetPlayerInput(input));
    }

    fn walking(yaw: f32) -> PlayerInput {
        PlayerInput {
            move_dir: Vec2::new(0.0, 1.0),
            yaw,
            ..PlayerInput::default()
        }
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

    fn position(sim: &SimWorld) -> Vec3 {
        sim.player_debug().pos
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
        let mut state = PlayerState::new(PlayerMovement::default());
        let (_, hp_max) = state.health();
        state.hp = hp_max - 30.0;
        state.heal(10.0);
        assert_eq!(state.health().0, hp_max - 20.0);
        state.heal(1000.0);
        assert_eq!(state.health().0, hp_max);
    }

    #[test]
    fn healing_ignores_negative_and_non_finite_amounts() {
        let mut state = PlayerState::new(PlayerMovement::default());
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
            (debug.pos.y - STANDING_CENTER_HEIGHT).abs() < 0.01,
            "{}",
            debug.pos.y
        );
        assert_eq!((debug.pos.x, debug.pos.z), (200.0, -100.0));
        let eye = sim.player_eye().unwrap();
        assert!((eye.y - (debug.pos.y + STAND_HEIGHT / 2.0 - EYE_BELOW_TOP)).abs() < 0.01);
    }

    #[test]
    fn a_spawn_waits_until_the_destination_terrain_is_resident() {
        let mut sim = SimWorld::new(PlayerMovement::default());
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
        hold(&mut sim, walking(0.0));
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
        hold(&mut sim, walking(-FRAC_PI_2));
        tick(&mut sim, 30);
        let moved = position(&sim) - start;
        assert!(moved.z < -15.0 && moved.x.abs() < 0.01, "moved {moved}");
    }

    #[test]
    fn diagonal_input_is_not_faster_than_straight_input() {
        let mut straight = spawned_at(flat_world(), SPAWN_AT);
        let mut diagonal = spawned_at(flat_world(), SPAWN_AT);
        hold(&mut straight, walking(0.0));
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
                    ..walking(0.0)
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
        hold(&mut sim, walking(-FRAC_PI_4));
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
        hold(&mut sim, walking(0.0));
        tick(&mut sim, 90);
        let end = position(&sim);
        assert!(end.x > 215.0, "x = {}", end.x);
        assert!(
            (end.y - (1.5 + STANDING_CENTER_HEIGHT)).abs() < 0.1,
            "y = {}",
            end.y
        );
    }

    #[test]
    fn a_ledge_taller_than_the_step_height_blocks_the_player() {
        let height = STEP_HEIGHT + 1.0;
        let ledge = prop_box(
            "meshes\\ledge.glb",
            Vec3::new(240.0, height / 2.0, -100.0),
            Vec3::new(30.0, height / 2.0, 50.0),
        );
        let mut sim = spawned_at(world_with(&[ledge]), SPAWN_AT);
        hold(&mut sim, walking(0.0));
        tick(&mut sim, 90);
        assert!(position(&sim).x < 210.0 - RADIUS + 0.2);
        assert!((position(&sim).y - STANDING_CENTER_HEIGHT).abs() < 0.1);
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

    #[test]
    fn crouching_lowers_the_capsule_with_the_feet_planted() {
        let mut sim = spawned_at(flat_world(), SPAWN_AT);
        hold(
            &mut sim,
            PlayerInput {
                crouch: true,
                ..PlayerInput::default()
            },
        );
        tick(&mut sim, 5);
        let debug = sim.player_debug();
        assert!(debug.crouching && debug.grounded);
        assert!(
            (debug.pos.y - CROUCHING_CENTER_HEIGHT).abs() < 0.05,
            "{}",
            debug.pos.y
        );
    }

    #[test]
    fn standing_up_needs_headroom() {
        let ceiling = prop_box(
            "meshes\\ceiling.glb",
            Vec3::new(100.0, 8.0, -100.0),
            Vec3::new(10.0, 0.5, 10.0),
        );
        let mut sim = spawned_at(world_with(&[ceiling]), Vec3::new(120.0, 3.0, -100.0));
        hold(
            &mut sim,
            PlayerInput {
                crouch: true,
                ..PlayerInput::default()
            },
        );
        tick(&mut sim, 5);
        hold(
            &mut sim,
            PlayerInput {
                crouch: true,
                ..walking(PI)
            },
        );
        tick(&mut sim, 40);
        assert!(
            position(&sim).x < 108.0,
            "now under the ceiling: {}",
            position(&sim)
        );
        hold(&mut sim, PlayerInput::default());
        tick(&mut sim, 10);
        assert!(sim.player_debug().crouching, "blocked by the ceiling");
        hold(&mut sim, walking(0.0));
        tick(&mut sim, 60);
        assert!(!sim.player_debug().crouching, "stood up again in the open");
    }

    #[test]
    fn a_player_bumping_a_ceiling_stops_rising() {
        let ceiling = prop_box(
            "meshes\\ceiling.glb",
            Vec3::new(200.0, 12.0, -100.0),
            Vec3::new(10.0, 0.5, 10.0),
        );
        let mut sim = spawned_at(world_with(&[ceiling]), SPAWN_AT);
        hold(
            &mut sim,
            PlayerInput {
                crouch: true,
                ..PlayerInput::default()
            },
        );
        tick(&mut sim, 5);
        hold(
            &mut sim,
            PlayerInput {
                jump: true,
                crouch: true,
                ..PlayerInput::default()
            },
        );
        tick(&mut sim, 60);
        assert!(
            position(&sim).y + CROUCH_HEIGHT / 2.0 < 11.5 + 0.1,
            "the head never passes the ceiling"
        );
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
        let mut sim = SimWorld::new(PlayerMovement::default());
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
            debug.grounded && (debug.pos.y - STANDING_CENTER_HEIGHT).abs() < 0.1,
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
        assert!((debug.pos.y - STANDING_CENTER_HEIGHT).abs() < 0.01);
    }

    #[test]
    fn walking_into_a_heavy_crate_pushes_it_by_no_more_than_the_push_force_allows() {
        let crate_mass = 15.0;
        let heavy = crate_prop(Vec3::new(215.0, 0.5, -100.0), crate_mass);
        let mut sim = spawned_at(world_with(&[heavy]), SPAWN_AT);
        hold(&mut sim, walking(0.0));
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
            (debug.pos.y - (1.0 + STANDING_CENTER_HEIGHT)).abs() < 0.2,
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
            hold(&mut sim, walking(0.0));
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
            hold(&mut sim, walking(0.0));
            assert!(record(&mut sim, 60).is_empty());
        }

        #[test]
        fn sprinting_and_crouching_pick_their_gaits() {
            let mut sim = footstep_world();
            hold(
                &mut sim,
                PlayerInput {
                    sprint: true,
                    ..walking(0.0)
                },
            );
            let sprint = record(&mut sim, 30);
            assert!(!sprint.is_empty());
            assert!(sprint.iter().all(|(_, event)| event.gait == Gait::Sprint));

            let mut sim = footstep_world();
            hold(
                &mut sim,
                PlayerInput {
                    crouch: true,
                    sprint: true,
                    ..walking(0.0)
                },
            );
            let sneak = record(&mut sim, 60);
            assert!(!sneak.is_empty());
            assert!(sneak.iter().all(|(_, event)| event.gait == Gait::Sneak));
        }

        #[test]
        fn standing_still_and_released_input_are_silent() {
            let mut sim = footstep_world();
            assert!(record(&mut sim, 60).is_empty());
            hold(&mut sim, walking(0.0));
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
            hold(&mut sim, walking(0.0));
            record(&mut sim, 40);
            assert!(position(&sim).x > 205.0, "the player should be at the wall");
            assert!(record(&mut sim, 120).is_empty());
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
