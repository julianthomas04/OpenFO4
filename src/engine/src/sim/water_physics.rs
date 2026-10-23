//! Fallout 4 rigid-body water physics.
//!
//! The rules are stated in Havok metres. The engine's unit is 1/7 m
//! ([`ENGINE_UNITS_PER_METER`]), so every distance and speed constant is stated
//! in its Havok unit and converted at the point of use; unit-free
//! quantities (damping rates, factors) are applied unchanged.
//!
//! Per fixed tick, every simulating clutter body is run through the
//! pipeline: distance/height gate, "fully above the surface" exit, then buoyancy
//! impulse, gravity factor, exponential velocity damping and the global
//! water-current relaxation.

use std::sync::Arc;

use glam::Vec3;
use rapier3d::prelude::*;

use super::EntityId;
use super::dynamic_body::ENGINE_UNITS_PER_METER;
use super::water_materials::{
    MaterialBuoyancy, WaterBodyProps, buoyancy_multiplier, is_tracked_layer,
};
use crate::utils::{env_flag, env_opt};

/// Havok metres per Fallout 4 game unit.
pub const HAVOK_PER_GAME_UNIT: f32 = 0.014_287_5;

/// `fSubmergedMaxSpeed` (Havok m/s): horizontal speed above which horizontal
/// damping applies.
const SUBMERGED_MAX_SPEED: f32 = 2.0;
/// `fSubmergedLinearDampingH` (1/s).
const SUBMERGED_LINEAR_DAMPING_H: f32 = 2.0;
/// `fSubmergedLinearDampingV` (1/s).
const SUBMERGED_LINEAR_DAMPING_V: f32 = 4.0;
/// `fSubmergedAngularDamping` (1/s).
const SUBMERGED_ANGULAR_DAMPING: f32 = 0.75;
/// `fSubmergedLODDistance` (game units). The gate squares it and multiplies by
/// [`HAVOK_PER_GAME_UNIT`] once (not squared), giving a ~45.9 Havok-unit radius.
const SUBMERGED_LOD_DISTANCE: f32 = 384.0;
/// `fSubmergedMaxWaterDistance` (game units): how far above the surface a body
/// origin may be and still be processed.
const SUBMERGED_MAX_WATER_DISTANCE: f32 = 48.0;
/// `fWaterGravityFactor:Havok`: gravity scale while in water.
const WATER_GRAVITY_FACTOR: f32 = 0.1;
/// Gravity scale restored on leaving the water.
const DRY_GRAVITY_FACTOR: f32 = 1.0;
/// `fFloatingObjectStartBobAtSubmergedPercent:Havok`.
const FLOATING_START_BOB_AT_SUBMERGED_PERCENT: f32 = 0.25;
/// `fFloatingObjectBobbingForceMultiplier:Havok`.
const FLOATING_BOBBING_FORCE_MULTIPLIER: f32 = 0.1;
/// `fFloatingObjectFlattenedPrecisionMod:Havok`.
const FLOATING_FLATTENED_PRECISION_MOD: f32 = 0.01;
/// `fFloatingObjectClampOffsetForceDistanceFromCenter:Havok`.
const FLOATING_CLAMP_OFFSET_FORCE_DISTANCE: f32 = 0.1;
/// Material buoyancy of a body whose materials are unknown (no collision data
/// in its sidecar, or no material table installed): the MATT `BNAM` default.
pub const DEFAULT_MATERIAL_BUOYANCY: f32 = 1.0;
/// Collision layer (clutter) and ragdoll part id assumed for a body without
/// collision data.
const UNKNOWN_BODY_LAYER: u8 = 4;
const UNKNOWN_BODY_PART: u8 = 0;

/// Seconds between trace summaries.
const TRACE_SUMMARY_TICKS: u32 = 60;
const TEST_HEIGHT_ENV_VAR: &str = "OPENFO4_WATER_TEST_HEIGHT";
const TRACE_ENV_VAR: &str = "OPENFO4_WATER_TRACE";

/// A water current in engine axes: linear in engine units per second, angular
/// in radians per second. A zero component leaves that lane of the body alone
/// (lanes where the current is exactly 0 are skipped).
#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub struct WaterCurrent {
    pub linear: Vec3,
    pub angular: Vec3,
}

impl WaterCurrent {
    /// From the WATR `NAM0`/`NAM1` values, which are Havok metres per second
    /// already mapped to engine axes.
    pub fn from_havok(linear: Vec3, angular: Vec3) -> Self {
        Self {
            linear: linear * ENGINE_UNITS_PER_METER,
            angular,
        }
    }
}

/// The water at one position: surface height (engine units, Y up) and the
/// global current acting there.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct WaterSurface {
    pub height: f32,
    pub current: Option<WaterCurrent>,
}

/// Water lookup by engine-space position; `None` outside any water volume.
pub type WaterQuery = Box<dyn Fn(Vec3) -> Option<WaterSurface> + Send + Sync>;

/// A global water plane at `height` engine units, as installed by
/// `OPENFO4_WATER_TEST_HEIGHT`.
pub fn infinite_plane(height: f32) -> WaterQuery {
    Box::new(move |_| {
        Some(WaterSurface {
            height,
            current: None,
        })
    })
}

/// Whether `OPENFO4_WATER_TEST_HEIGHT` asks for a global test plane, which
/// then stands in for the water of the world.
pub fn test_plane_requested() -> bool {
    env_opt::<f32>(TEST_HEIGHT_ENV_VAR).is_some()
}

/// The test plane requested through `OPENFO4_WATER_TEST_HEIGHT` (game units).
pub fn test_plane_from_env() -> Option<WaterQuery> {
    let game_units: f32 = env_opt(TEST_HEIGHT_ENV_VAR)?;
    let height = game_units * crate::bethesda_space::UNITS_TO_ENGINE;
    tracing::info!("water: test plane at {game_units} game units ({height} engine units)");
    Some(infinite_plane(height))
}

/// Distance/height gate. `origin` is the body origin.
pub(super) fn passes_gate(origin: Vec3, lod_center: Vec3, water_height: f32) -> bool {
    let to_meters = 1.0 / ENGINE_UNITS_PER_METER;
    let distance_sq_havok = origin.distance_squared(lod_center) * to_meters * to_meters;
    let lod_limit = SUBMERGED_LOD_DISTANCE * SUBMERGED_LOD_DISTANCE * HAVOK_PER_GAME_UNIT;
    let max_height =
        water_height + SUBMERGED_MAX_WATER_DISTANCE * HAVOK_PER_GAME_UNIT * ENGINE_UNITS_PER_METER;
    distance_sq_havok <= lod_limit && origin.y <= max_height
}

/// Share of the way the water has risen from the centre of mass to the top of
/// the body's AABB: 0 at the waterline, 1 when it reaches the top. A degenerate
/// body whose top is at or below its centre counts as fully submerged.
pub(super) fn submerged_fraction(water_height: f32, com_y: f32, aabb_top_y: f32) -> f32 {
    let span = aabb_top_y - com_y;
    if span > 0.0 {
        (water_height - com_y) / span
    } else {
        1.0
    }
}

/// Buoyancy factor after the bobbing reduction: only a positive factor is
/// reduced, and only while the water has not risen past the bob threshold.
pub(super) fn bobbing_buoyancy(buoyancy: f32, submerged_fraction: f32) -> f32 {
    if submerged_fraction <= FLOATING_START_BOB_AT_SUBMERGED_PERCENT && buoyancy > 0.0 {
        buoyancy * FLOATING_BOBBING_FORCE_MULTIPLIER
    } else {
        buoyancy
    }
}

/// Index of the first smallest and first largest component.
fn extreme_axes(extents: Vec3) -> (usize, usize) {
    let mut min_axis = 0;
    let mut max_axis = 0;
    for axis in 1..3 {
        if extents[axis] < extents[min_axis] {
            min_axis = axis;
        }
        if extents[axis] > extents[max_axis] {
            max_axis = axis;
        }
    }
    (min_axis, max_axis)
}

/// Whether the body counts as flattened (plank, tray) for the force offset.
pub(super) fn is_flattened(extents: Vec3) -> bool {
    let (min_axis, max_axis) = extreme_axes(extents);
    extents[max_axis] > extents[min_axis] * (1.0 + FLOATING_FLATTENED_PRECISION_MOD)
}

/// Body-local offset of the buoyancy application point from the centre of
/// mass. Zero unless the body is flattened. The offset is
/// `thinnest * 0.1 * (B != 0 ? longest : 0)` evaluated in Havok metres, so the
/// extents are converted before multiplying.
pub(super) fn buoyancy_offset(extents: Vec3, buoyancy: f32) -> Vec3 {
    if !is_flattened(extents) {
        return Vec3::ZERO;
    }
    let (min_axis, max_axis) = extreme_axes(extents);
    let longest = if buoyancy == 0.0 {
        0.0
    } else {
        extents[max_axis] / ENGINE_UNITS_PER_METER
    };
    let thinnest = extents[min_axis] / ENGINE_UNITS_PER_METER;
    let mut offset = Vec3::ZERO;
    offset[min_axis] =
        thinnest * FLOATING_CLAMP_OFFSET_FORCE_DISTANCE * longest * ENGINE_UNITS_PER_METER;
    offset
}

/// Whether the centre of mass is far enough under the surface to be pushed.
pub(super) fn is_pushed_up(com_y: f32, water_height: f32, thinnest_extent: f32) -> bool {
    com_y < water_height - FLOATING_FLATTENED_PRECISION_MOD * thinnest_extent
}

/// Impulse `-(m * B) * g * dt` (upward for downward gravity).
pub(super) fn buoyancy_impulse(mass: f32, buoyancy: f32, gravity: Vec3, dt: f32) -> Vec3 {
    -(mass * buoyancy) * gravity * dt
}

/// Linear velocity after one damped step: vertical always at the V rate,
/// horizontal at the H rate only above the max speed.
pub(super) fn damped_linear_velocity(velocity: Vec3, dt: f32) -> Vec3 {
    let max_speed = SUBMERGED_MAX_SPEED * ENGINE_UNITS_PER_METER;
    let mut horizontal = Vec3::new(velocity.x, 0.0, velocity.z);
    if horizontal.length() > max_speed {
        horizontal *= (-SUBMERGED_LINEAR_DAMPING_H * dt).exp();
    }
    Vec3::new(
        horizontal.x,
        velocity.y * (-SUBMERGED_LINEAR_DAMPING_V * dt).exp(),
        horizontal.z,
    )
}

pub(super) fn damped_angular_velocity(angular: Vec3, dt: f32) -> Vec3 {
    angular * (-SUBMERGED_ANGULAR_DAMPING * dt).exp()
}

/// `(target - current) * mass * dt` on every lane where `target` is non-zero
/// (so a velocity change of `dt * (target - current)`), zero on the others.
pub(super) fn current_impulse(target: Vec3, current: Vec3, mass: f32, dt: f32) -> Vec3 {
    let lane = |target: f32, current: f32| {
        if target == 0.0 {
            0.0
        } else {
            (target - current) * mass * dt
        }
    };
    Vec3::new(
        lane(target.x, current.x),
        lane(target.y, current.y),
        lane(target.z, current.z),
    )
}

/// Water bookkeeping a body carries: whether the in-water set holds it
/// and its material buoyancy. Owned by the body (a clutter rigid body or a
/// ragdoll bone), so it disappears with it.
#[derive(Default)]
pub(crate) struct WaterBodyState {
    in_water: bool,
    /// The body's own (linear, angular) rapier damping while it is in the
    /// water, where submerged damping is the only damping.
    dry_damping: Option<(f32, f32)>,
    /// Mean MATT `BNAM` over the body's shapes, resolved from its materials on
    /// first use unless set explicitly.
    material_buoyancy: Option<f32>,
}

impl WaterBodyState {
    /// Fixes the mean material buoyancy, overriding the body's materials.
    pub fn set_material_buoyancy(&mut self, buoyancy: f32) {
        self.material_buoyancy = Some(buoyancy);
    }
}

/// One simulating body offered to the water pass. A body without `props` has no
/// collision data in its sidecar and counts as plain clutter.
pub(crate) struct WaterTarget<'a> {
    pub entity: EntityId,
    pub handle: RigidBodyHandle,
    pub props: Option<&'a WaterBodyProps>,
    pub state: &'a mut WaterBodyState,
}

/// Undrained water entries kept: an engine bound; the splat list is otherwise
/// unbounded. It only bounds memory and equals the renderer's splat cap.
const MAX_PENDING_ENTRIES: usize = 512;

/// Water state shared by the clutter bodies and the ragdoll bones: the water
/// source, the material table and the trace counters.
pub(crate) struct WaterPhysics {
    query: Option<WaterQuery>,
    materials: Option<Arc<MaterialBuoyancy>>,
    trace: bool,
    trace_ticks: u32,
    /// Where bodies entered the water since the last drain (engine space), for
    /// the renderer's water displacement splats.
    entries: Vec<Vec3>,
}

impl WaterPhysics {
    pub fn new() -> Self {
        Self {
            query: None,
            materials: None,
            trace: env_flag(TRACE_ENV_VAR, false),
            trace_ticks: 0,
            entries: Vec::new(),
        }
    }

    /// Moves the recorded water entries (engine-space body origins) into `out`.
    pub fn drain_entries(&mut self, out: &mut Vec<Vec3>) {
        out.append(&mut self.entries);
    }

    /// Remembers an entry until the next drain; beyond
    /// [`MAX_PENDING_ENTRIES`] the newest are dropped, so nothing grows while
    /// the renderer does not drain (displacement off, headless).
    fn record_entry(&mut self, origin: Vec3) {
        if self.entries.len() < MAX_PENDING_ENTRIES {
            self.entries.push(origin);
        }
    }

    /// Replaces the water source; bodies already in the water stay tracked.
    /// `None` switches the pass off (tracked bodies keep their state).
    pub fn set_query(&mut self, query: Option<WaterQuery>) {
        self.query = query;
    }

    /// Installs the MATT buoyancy table. A body resolves its material
    /// buoyancy on first use, so install the table before the first cell is
    /// added.
    pub fn set_materials(&mut self, materials: Arc<MaterialBuoyancy>) {
        self.materials = Some(materials);
    }

    /// One fixed tick over the simulating bodies.
    pub fn update<'a>(
        &mut self,
        world: &mut PhysicsWorld,
        targets: impl Iterator<Item = WaterTarget<'a>>,
        lod_center: Vec3,
        dt: f32,
    ) {
        if self.query.is_none() {
            return;
        }
        let mut submerged = 0usize;
        for target in targets {
            if self.update_body(world, target, lod_center, dt) {
                submerged += 1;
            }
        }
        if self.trace {
            self.trace_ticks += 1;
            if self.trace_ticks >= TRACE_SUMMARY_TICKS {
                self.trace_ticks = 0;
                tracing::info!("water: submerged_this_tick={submerged}");
            }
        }
    }

    /// Mean material buoyancy of the body, resolving and caching it on first use.
    fn material_buoyancy(&self, target: &mut WaterTarget<'_>) -> f32 {
        let resolve = || match (target.props, &self.materials) {
            (Some(props), Some(table)) => table.mean(&props.materials),
            _ => DEFAULT_MATERIAL_BUOYANCY,
        };
        *target.state.material_buoyancy.get_or_insert_with(resolve)
    }

    /// Returns whether the body was submerged and processed this tick.
    fn update_body(
        &mut self,
        world: &mut PhysicsWorld,
        mut target: WaterTarget<'_>,
        lod_center: Vec3,
        dt: f32,
    ) -> bool {
        let (layer, part) = target
            .props
            .map_or((UNKNOWN_BODY_LAYER, UNKNOWN_BODY_PART), |props| {
                (props.layer, props.part)
            });
        if !is_tracked_layer(layer) {
            return false;
        }
        let handle = target.handle;
        let Some(rb) = world.bodies.get(handle).filter(|rb| rb.is_dynamic()) else {
            return false;
        };
        let origin = rb.translation();
        let surface = self.query.as_ref().and_then(|query| query(origin));
        let Some(surface) = surface else {
            self.leave(world, &mut target, "left the water volume");
            return false;
        };
        if !passes_gate(origin, lod_center, surface.height) {
            return false;
        }
        let Some((aabb_min_y, aabb_max_y)) = world_vertical_bounds(world, rb) else {
            return false;
        };
        if aabb_min_y > surface.height {
            self.leave(world, &mut target, "above the surface");
            return false;
        }
        let material = self.material_buoyancy(&mut target);
        let buoyancy = material * buoyancy_multiplier(layer, part, material);
        let local_extents = local_extents(world, rb);
        let gravity = world.gravity;
        let Some(rb) = world.bodies.get_mut(handle) else {
            return false;
        };
        if rb.is_sleeping() {
            rb.wake_up(true);
        }
        apply_buoyancy(
            rb,
            &BuoyancyInput {
                water_height: surface.height,
                aabb_top_y: aabb_max_y,
                local_extents,
                buoyancy,
                gravity,
            },
            dt,
        );
        if rb.gravity_scale() != WATER_GRAVITY_FACTOR {
            rb.set_gravity_scale(WATER_GRAVITY_FACTOR, false);
        }
        suspend_rapier_damping(rb, target.state);
        apply_damping(rb, dt);
        if let Some(current) = surface.current {
            apply_current(rb, current, dt);
        }
        if !std::mem::replace(&mut target.state.in_water, true) {
            self.record_entry(origin);
            if self.trace {
                tracing::info!(
                    "water: enter entity={:#x} x={:.2} z={:.2} y={:.2} waterH={:.2} layer={layer} part={part} B={buoyancy} mass={:.2}",
                    target.entity.0,
                    origin.x,
                    origin.z,
                    origin.y,
                    surface.height,
                    rb.mass()
                );
            }
        }
        true
    }

    fn leave(&self, world: &mut PhysicsWorld, target: &mut WaterTarget<'_>, reason: &str) {
        if !std::mem::take(&mut target.state.in_water) {
            return;
        }
        if let Some(rb) = world.bodies.get_mut(target.handle) {
            rb.set_gravity_scale(DRY_GRAVITY_FACTOR, false);
            restore_rapier_damping(rb, target.state);
        }
        if self.trace {
            tracing::info!("water: exit entity={:#x} ({reason})", target.entity.0);
        }
    }
}

/// Zeroes rapier's damping for a body in the water, remembering the original
/// values; submerged damping replaces it.
fn suspend_rapier_damping(rb: &mut RigidBody, state: &mut WaterBodyState) {
    if state.dry_damping.is_none() {
        state.dry_damping = Some((rb.linear_damping(), rb.angular_damping()));
        rb.set_linear_damping(0.0);
        rb.set_angular_damping(0.0);
    }
}

fn restore_rapier_damping(rb: &mut RigidBody, state: &mut WaterBodyState) {
    if let Some((linear, angular)) = state.dry_damping.take() {
        rb.set_linear_damping(linear);
        rb.set_angular_damping(angular);
    }
}

/// Lowest and highest world Y over the body's colliders.
fn world_vertical_bounds(world: &PhysicsWorld, rb: &RigidBody) -> Option<(f32, f32)> {
    rb.colliders()
        .iter()
        .filter_map(|handle| world.colliders.get(*handle))
        .map(|collider| {
            let aabb = collider.compute_aabb();
            (aabb.mins.y, aabb.maxs.y)
        })
        .reduce(|(low, high), (min_y, max_y)| (low.min(min_y), high.max(max_y)))
}

/// Extents of the body's collider AABB in its own frame.
fn local_extents(world: &PhysicsWorld, rb: &RigidBody) -> Vec3 {
    let bounds = rb
        .colliders()
        .iter()
        .filter_map(|handle| world.colliders.get(*handle))
        .map(|collider| {
            let local = collider
                .position_wrt_parent()
                .copied()
                .unwrap_or(Pose::IDENTITY);
            collider.shape().compute_aabb(&local)
        })
        .reduce(|a, b| a.merged(&b));
    bounds.map_or(Vec3::ZERO, |aabb| aabb.maxs - aabb.mins)
}

struct BuoyancyInput {
    water_height: f32,
    aabb_top_y: f32,
    local_extents: Vec3,
    buoyancy: f32,
    gravity: Vec3,
}

fn apply_buoyancy(rb: &mut RigidBody, input: &BuoyancyInput, dt: f32) {
    let com = rb.center_of_mass();
    let fraction = submerged_fraction(input.water_height, com.y, input.aabb_top_y);
    let buoyancy = bobbing_buoyancy(input.buoyancy, fraction);
    let (thinnest_axis, _) = extreme_axes(input.local_extents);
    if !is_pushed_up(
        com.y,
        input.water_height,
        input.local_extents[thinnest_axis],
    ) {
        return;
    }
    let impulse = buoyancy_impulse(rb.mass(), buoyancy, input.gravity, dt);
    let offset = buoyancy_offset(input.local_extents, buoyancy);
    if offset == Vec3::ZERO {
        rb.apply_impulse(impulse, true);
    } else {
        let point = com + *rb.rotation() * offset;
        rb.apply_impulse_at_point(impulse, point, true);
    }
}

fn apply_damping(rb: &mut RigidBody, dt: f32) {
    let linear = rb.linvel();
    let damped = damped_linear_velocity(linear, dt);
    if damped != linear {
        rb.set_linvel(damped, false);
    }
    let angular = rb.angvel();
    let damped = damped_angular_velocity(angular, dt);
    if damped != angular {
        rb.set_angvel(damped, false);
    }
}

fn apply_current(rb: &mut RigidBody, current: WaterCurrent, dt: f32) {
    let mass = rb.mass();
    let linear = current_impulse(current.linear, rb.linvel(), mass, dt);
    if linear != Vec3::ZERO {
        rb.apply_impulse(linear, false);
    }
    let angular = current_impulse(current.angular, rb.angvel(), mass, dt);
    if angular != Vec3::ZERO {
        rb.apply_torque_impulse(angular, false);
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact in f32 or compared with an epsilon"
)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;
    const GRAVITY: Vec3 = Vec3::new(0.0, -68.59, 0.0);

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1.0e-4 * b.abs().max(1.0)
    }

    #[test]
    fn undrained_entries_are_bounded_and_drain_in_arrival_order() {
        let mut water = WaterPhysics::new();
        for index in 0..MAX_PENDING_ENTRIES + 10 {
            water.record_entry(Vec3::new(index as f32, 0.0, 0.0));
        }
        let mut drained = Vec::new();
        water.drain_entries(&mut drained);
        assert_eq!(drained.len(), MAX_PENDING_ENTRIES);
        assert_eq!(drained[0].x, 0.0, "the oldest entries survive");
        water.record_entry(Vec3::ONE);
        water.drain_entries(&mut drained);
        assert_eq!(
            drained.len(),
            MAX_PENDING_ENTRIES + 1,
            "draining frees room"
        );
    }

    #[test]
    fn the_lod_radius_squares_the_distance_but_scales_once() {
        // sqrt(384^2 * 0.0142875) = 45.9 Havok = 321.3 engine units.
        let radius = (SUBMERGED_LOD_DISTANCE * SUBMERGED_LOD_DISTANCE * HAVOK_PER_GAME_UNIT).sqrt()
            * ENGINE_UNITS_PER_METER;
        assert!(close(radius, 321.3), "{radius}");
        let inside = Vec3::new(320.0, 0.0, 0.0);
        let outside = Vec3::new(323.0, 0.0, 0.0);
        assert!(passes_gate(inside, Vec3::ZERO, 0.0));
        assert!(!passes_gate(outside, Vec3::ZERO, 0.0));
    }

    #[test]
    fn the_gate_rejects_origins_more_than_48_game_units_over_the_surface() {
        // 48 gu * 0.0142875 * 7 = 4.8006 engine units.
        assert!(passes_gate(Vec3::new(0.0, 4.8, 0.0), Vec3::ZERO, 0.0));
        assert!(!passes_gate(Vec3::new(0.0, 4.81, 0.0), Vec3::ZERO, 0.0));
        assert!(passes_gate(Vec3::new(0.0, -50.0, 0.0), Vec3::ZERO, 0.0));
    }

    #[test]
    fn the_submerged_fraction_is_zero_at_the_waterline_and_one_at_the_top() {
        assert_eq!(submerged_fraction(2.0, 2.0, 3.0), 0.0);
        assert_eq!(submerged_fraction(3.0, 2.0, 3.0), 1.0);
        assert_eq!(submerged_fraction(2.25, 2.0, 3.0), 0.25);
        assert_eq!(submerged_fraction(5.0, 3.0, 3.0), 1.0, "degenerate AABB");
    }

    #[test]
    fn bobbing_reduces_only_a_positive_factor_at_or_below_the_threshold() {
        assert!(close(bobbing_buoyancy(1.0, 0.25), 0.1));
        assert!(close(bobbing_buoyancy(1.0, -0.3), 0.1));
        assert_eq!(bobbing_buoyancy(1.0, 0.26), 1.0);
        assert_eq!(bobbing_buoyancy(-0.3, 0.0), -0.3);
        assert_eq!(bobbing_buoyancy(0.0, 0.0), 0.0);
    }

    #[test]
    fn a_body_is_flattened_when_its_longest_extent_beats_the_thinnest_by_over_one_percent() {
        assert!(!is_flattened(Vec3::new(1.0, 1.0, 1.0)));
        assert!(!is_flattened(Vec3::new(1.0, 1.01, 1.0)));
        assert!(is_flattened(Vec3::new(1.0, 1.0101, 1.0)));
    }

    #[test]
    fn the_flattened_offset_is_thinnest_times_longest_in_havok_metres() {
        // Extents in engine units: thin 0.7 (0.1 m), long 14 (2 m).
        let offset = buoyancy_offset(Vec3::new(14.0, 0.7, 7.0), 1.0);
        // 0.1 m * 0.1 * 2 m = 0.02 m^2 -> 0.14 engine units, along the thin axis.
        assert_eq!(offset.x, 0.0);
        assert_eq!(offset.z, 0.0);
        assert!(close(offset.y, 0.14), "{}", offset.y);
        assert_eq!(buoyancy_offset(Vec3::new(14.0, 0.7, 7.0), 0.0), Vec3::ZERO);
        assert_eq!(buoyancy_offset(Vec3::splat(3.0), 1.0), Vec3::ZERO);
    }

    #[test]
    fn the_push_needs_the_centre_of_mass_a_hundredth_of_the_thinnest_extent_below_the_surface() {
        assert!(is_pushed_up(0.89, 1.0, 10.0 * 1.0));
        assert!(!is_pushed_up(0.91, 1.0, 10.0 * 1.0));
        assert!(!is_pushed_up(1.0, 1.0, 1.0));
    }

    #[test]
    fn the_buoyancy_impulse_is_mass_times_factor_times_gravity_times_dt_upwards() {
        let impulse = buoyancy_impulse(2.0, 0.5, GRAVITY, DT);
        assert_eq!(impulse.x, 0.0);
        assert!(close(impulse.y, 2.0 * 0.5 * 68.59 * DT));
        assert!(buoyancy_impulse(2.0, -0.3, GRAVITY, DT).y < 0.0);
    }

    #[test]
    fn vertical_damping_is_always_applied_horizontal_only_above_two_havok_metres_per_second() {
        let slow = damped_linear_velocity(Vec3::new(13.0, 10.0, 0.0), DT);
        assert_eq!(slow.x, 13.0, "13 u/s < 14 u/s limit");
        assert!(close(slow.y, 10.0 * (-4.0 * DT).exp()));
        let fast = damped_linear_velocity(Vec3::new(15.0, 0.0, 0.0), DT);
        assert!(close(fast.x, 15.0 * (-2.0 * DT).exp()));
        let diagonal = damped_linear_velocity(Vec3::new(10.0, 0.0, 10.0), DT);
        assert!(diagonal.x < 10.0, "length 14.14 > 14");
    }

    #[test]
    fn angular_damping_scales_every_axis() {
        let damped = damped_angular_velocity(Vec3::new(1.0, -2.0, 3.0), DT);
        let factor = (-0.75 * DT).exp();
        assert!(
            close(damped.x, factor)
                && close(damped.y, -2.0 * factor)
                && close(damped.z, 3.0 * factor)
        );
    }

    #[test]
    fn the_current_relaxes_velocity_towards_it_only_on_its_non_zero_lanes() {
        let impulse = current_impulse(Vec3::new(7.0, 0.0, -7.0), Vec3::new(1.0, 5.0, 1.0), 2.0, DT);
        assert!(close(impulse.x, 6.0 * 2.0 * DT));
        assert_eq!(impulse.y, 0.0);
        assert!(close(impulse.z, -8.0 * 2.0 * DT));
    }

    #[test]
    fn rapier_damping_is_off_in_the_water_and_restored_on_leaving() {
        let mut body = RigidBodyBuilder::dynamic()
            .linear_damping(0.2)
            .angular_damping(0.4)
            .build();
        let mut state = WaterBodyState::default();
        suspend_rapier_damping(&mut body, &mut state);
        assert_eq!((body.linear_damping(), body.angular_damping()), (0.0, 0.0));
        suspend_rapier_damping(&mut body, &mut state);
        restore_rapier_damping(&mut body, &mut state);
        assert_eq!((body.linear_damping(), body.angular_damping()), (0.2, 0.4));
        restore_rapier_damping(&mut body, &mut state);
        assert_eq!((body.linear_damping(), body.angular_damping()), (0.2, 0.4));
    }

    #[test]
    fn havok_currents_are_scaled_to_engine_units() {
        let current = WaterCurrent::from_havok(Vec3::new(1.0, 0.0, 0.0), Vec3::Y);
        assert_eq!(current.linear, Vec3::new(7.0, 0.0, 0.0));
        assert_eq!(current.angular, Vec3::Y);
    }
}
