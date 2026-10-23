//! Tuning that makes a ragdoll come to rest: the displacement-window sleep
//! criterion, the near-rest damping ease-out, and the joint and inertia
//! conditioning that keeps the solver from jittering.

use glam::Vec3;
use rapier3d::prelude::*;

use super::bake::CachedShape;
use super::ragdoll_spec::RagdollSpec;
use crate::utils::warn_once;

/// Seconds rapier waits after the rest window opened the sleep gate.
const SLEEP_DELAY_SECONDS: f32 = 0.1;
/// Kinetic energy per unit mass (u^2/s^2) at which the near-rest damping starts: a body at 1.5 u/s.
const SETTLE_KINETIC_ENERGY: f32 = 0.5 * 1.5 * 1.5;
/// Joint spring of ragdoll bones, limits included; rapier's default is rigid (1e6 Hz).
const JOINT_NATURAL_FREQUENCY: f32 = 30.0;
const JOINT_DAMPING_RATIO: f32 = 1.0;
/// Narrowest half-range of a free joint axis; a zero-width range is a locked axis and stays.
const MIN_LIMIT_HALF_RANGE: f32 = 5.0 * std::f32::consts::PI / 180.0;
/// A bone's smallest principal inertia may not fall below this fraction of its joint neighbour's.
const MIN_NEIGHBOUR_INERTIA_RATIO: f32 = 0.1;
/// Deepest penetration (engine units) the solver resolves per ragdoll bone contact, which caps push-out near 4 u/s.
pub(super) const MAX_CONTACT_CORRECTION_DEPTH: f32 = 0.3;

/// A ragdoll sleeps once every bone point stayed within `envelope` for `window_seconds`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RagdollSettings {
    /// Largest range of any bone point over the window, in engine units.
    pub envelope: f32,
    /// Seconds every bone must stay inside the envelope before the ragdoll may sleep.
    pub window_seconds: f32,
    /// Extra damping coefficient at zero kinetic energy; it fades to zero at `SETTLE_KINETIC_ENERGY`.
    pub settle_damping: f32,
}

impl RagdollSettings {
    pub const ENVELOPE_RANGE: std::ops::RangeInclusive<f32> = 0.005..=0.5;
    pub const WINDOW_SECONDS_RANGE: std::ops::RangeInclusive<f32> = 0.1..=3.0;
    pub const SETTLE_DAMPING_RANGE: std::ops::RangeInclusive<f32> = 0.0..=30.0;

    /// `requested` limited to the ranges the developer menu offers; a non-finite
    /// field keeps its `current` value. Every rejection is reported.
    pub fn validated(requested: Self, current: Self) -> Self {
        Self {
            envelope: validated_field(
                "envelope",
                requested.envelope,
                &Self::ENVELOPE_RANGE,
                current.envelope,
            ),
            window_seconds: validated_field(
                "window_seconds",
                requested.window_seconds,
                &Self::WINDOW_SECONDS_RANGE,
                current.window_seconds,
            ),
            settle_damping: validated_field(
                "settle_damping",
                requested.settle_damping,
                &Self::SETTLE_DAMPING_RANGE,
                current.settle_damping,
            ),
        }
    }
}

impl Default for RagdollSettings {
    /// Jolt's sleep test: 0.03 m/s over 0.5 s is 0.015 m, or 0.1 u at 7 u/m.
    fn default() -> Self {
        Self {
            envelope: 0.1,
            window_seconds: 0.5,
            settle_damping: 8.0,
        }
    }
}

fn validated_field(
    name: &str,
    value: f32,
    range: &std::ops::RangeInclusive<f32>,
    current: f32,
) -> f32 {
    if !value.is_finite() {
        warn_once(&format!("ragdoll_setting_{name}"), || {
            format!("ragdoll: ignored non-finite {name}; keeping {current}")
        });
        return current;
    }
    let clamped = value.clamp(*range.start(), *range.end());
    #[allow(
        clippy::float_cmp,
        reason = "exact: clamp returns its input unchanged when it is inside the range"
    )]
    let was_clamped = clamped != value;
    if was_clamped {
        warn_once(&format!("ragdoll_setting_{name}"), || {
            format!("ragdoll: {name} {value} is outside {range:?}; using {clamped}")
        });
    }
    clamped
}

/// Prepares a simulated bone's rapier activation: a frozen spawn is eligible to
/// sleep at once, so the first step folds the bodies into one sleeping island.
pub(super) fn prepare_activation(rb: &mut RigidBody, frozen: bool) {
    let activation = rb.activation_mut();
    activation.time_until_sleep = SLEEP_DELAY_SECONDS;
    set_gate(activation, frozen);
    if frozen {
        activation.time_since_can_sleep = activation.time_until_sleep;
    }
}

/// rapier sleeps a body once its speed stays under the thresholds: infinite
/// opens the gate, zero closes it.
fn set_gate(activation: &mut RigidBodyActivation, open: bool) {
    let threshold = if open { f32::INFINITY } else { 0.0 };
    activation.normalized_linear_threshold = threshold;
    activation.angular_threshold = threshold;
}

/// One simulated bone as the rest window sees it.
pub(super) struct RestBone {
    handle: RigidBodyHandle,
    /// Body-local collider box corners.
    probes: Vec<Vec3>,
    linear_damping: f32,
    angular_damping: f32,
}

impl RestBone {
    pub fn new(
        handle: RigidBodyHandle,
        shapes: &[CachedShape],
        linear_damping: f32,
        angular_damping: f32,
    ) -> Self {
        Self {
            handle,
            probes: probe_points(shapes),
            linear_damping,
            angular_damping,
        }
    }
}

fn probe_points(shapes: &[CachedShape]) -> Vec<Vec3> {
    shapes
        .iter()
        .flat_map(|cached| {
            let aabb = cached.shape.compute_local_aabb();
            let (min, max) = (aabb.mins, aabb.maxs);
            (0..8).map(move |corner| {
                let pick =
                    |bit: u32, low: f32, high: f32| if corner & bit == 0 { low } else { high };
                cached.local_pose
                    * Vec3::new(
                        pick(1, min.x, max.x),
                        pick(2, min.y, max.y),
                        pick(4, min.z, max.z),
                    )
            })
        })
        .collect()
}

/// Min and max world position of every probe point since the window started.
type PointBounds = Vec<(Vec3, Vec3)>;

fn seed_bounds(points: &[Vec3]) -> PointBounds {
    points.iter().map(|point| (*point, *point)).collect()
}

/// Rest state of one ragdoll: the sleep window and the near-rest damping.
pub(super) struct RestTracker {
    bones: Vec<RestBone>,
    /// `None` until the first awake step seeds it.
    bounds: Option<PointBounds>,
    /// Seconds every point has stayed inside the envelope.
    still_seconds: f32,
    /// Extent of the widest point over the window as of the last step, in engine units.
    range: f32,
    gate_open: bool,
    /// Largest kinetic energy per unit mass of any bone as of the last step.
    kinetic_energy: f32,
}

impl RestTracker {
    pub fn new(bones: Vec<RestBone>) -> Self {
        Self {
            bones,
            bounds: None,
            still_seconds: 0.0,
            range: 0.0,
            gate_open: false,
            kinetic_energy: 0.0,
        }
    }

    pub fn is_settling(&self) -> bool {
        self.still_seconds > 0.0
    }

    pub fn still_seconds(&self) -> f32 {
        self.still_seconds
    }

    pub fn range(&self) -> f32 {
        self.range
    }

    pub fn gate_open(&self) -> bool {
        self.gate_open
    }

    pub fn kinetic_energy(&self) -> f32 {
        self.kinetic_energy
    }

    /// One awake step: ease the motion out and advance the sleep window.
    pub fn step(&mut self, world: &mut PhysicsWorld, settings: &RagdollSettings, dt: f32) {
        self.apply_settle_damping(world, settings.settle_damping);
        self.update_window(world, settings, dt);
    }

    /// The ragdoll went to sleep: authored damping and a closed gate again.
    pub fn end(&mut self, world: &mut PhysicsWorld) {
        self.close_gate(world);
        self.set_extra_damping(world, 0.0);
        *self = Self::new(std::mem::take(&mut self.bones));
    }

    pub fn close_gate(&mut self, world: &mut PhysicsWorld) {
        self.gate_open = false;
        self.set_gate(world, false);
    }

    fn set_gate(&self, world: &mut PhysicsWorld, open: bool) {
        for bone in &self.bones {
            if let Some(rb) = world.bodies.get_mut(bone.handle) {
                set_gate(rb.activation_mut(), open);
            }
        }
    }

    fn set_extra_damping(&self, world: &mut PhysicsWorld, extra: f32) {
        for bone in &self.bones {
            if let Some(rb) = world.bodies.get_mut(bone.handle) {
                rb.set_linear_damping(bone.linear_damping + extra);
                rb.set_angular_damping(bone.angular_damping + extra);
            }
        }
    }

    /// Damping that grows continuously as the fastest bone's kinetic energy per
    /// unit mass `e` falls below `SETTLE_KINETIC_ENERGY`.
    fn apply_settle_damping(&mut self, world: &mut PhysicsWorld, settle_damping: f32) {
        self.kinetic_energy = self
            .bones
            .iter()
            .filter_map(|bone| world.bodies.get(bone.handle))
            .map(kinetic_energy_per_mass)
            .fold(0.0_f32, f32::max);
        let depth = 1.0
            - (self.kinetic_energy / SETTLE_KINETIC_ENERGY)
                .sqrt()
                .min(1.0);
        self.set_extra_damping(world, settle_damping * depth * depth);
    }

    fn world_points(&self, world: &PhysicsWorld) -> Option<Vec<Vec3>> {
        let mut points = Vec::new();
        for bone in &self.bones {
            let Some(rb) = world.bodies.get(bone.handle) else {
                warn_once("ragdoll_rest_bone_missing", || {
                    "ragdoll: a live bone has no rigid body; rest window paused".into()
                });
                return None;
            };
            let pose = *rb.position();
            points.extend(bone.probes.iter().map(|local| pose * *local));
        }
        Some(points)
    }

    /// Every point ranging less than the envelope for `window_seconds` opens the
    /// sleep gate; a point leaving the envelope restarts the window at the
    /// current pose and closes the gate.
    fn update_window(&mut self, world: &mut PhysicsWorld, settings: &RagdollSettings, dt: f32) {
        let Some(points) = self.world_points(world) else {
            return;
        };
        let Some(bounds) = self.bounds.as_mut() else {
            self.bounds = Some(seed_bounds(&points));
            return;
        };
        let mut range = 0.0_f32;
        for ((min, max), point) in bounds.iter_mut().zip(&points) {
            *min = min.min(*point);
            *max = max.max(*point);
            range = range.max((*max - *min).length());
        }
        self.range = range;
        if range > settings.envelope {
            self.bounds = Some(seed_bounds(&points));
            self.still_seconds = 0.0;
            if self.gate_open {
                self.close_gate(world);
            }
            return;
        }
        self.still_seconds += dt;
        if !self.gate_open && self.still_seconds >= settings.window_seconds {
            self.gate_open = true;
            self.set_gate(world, true);
        }
    }
}

/// Translation plus rotation about the principal inertia axes, per unit mass.
fn kinetic_energy_per_mass(body: &RigidBody) -> f32 {
    let mass = body.mass();
    if mass <= 0.0 {
        return 0.0;
    }
    let mprops = &body.mass_properties().local_mprops;
    let spin = mprops.principal_inertia_local_frame.inverse()
        * (body.rotation().inverse() * body.angvel());
    let rotational = 0.5 * mprops.principal_inertia().dot(spin * spin) / mass;
    0.5 * body.linvel().length_squared() + rotational
}

/// Widens a narrow authored limit range around its middle.
pub(super) fn widen_range(range: [f32; 2]) -> [f32; 2] {
    let half = (range[1] - range[0]) * 0.5;
    if half == 0.0 || half >= MIN_LIMIT_HALF_RANGE {
        return range;
    }
    let middle = f32::midpoint(range[0], range[1]);
    [middle - MIN_LIMIT_HALF_RANGE, middle + MIN_LIMIT_HALF_RANGE]
}

pub(super) fn widen_cone(cone: f32) -> f32 {
    if cone == 0.0 {
        cone
    } else {
        cone.max(MIN_LIMIT_HALF_RANGE)
    }
}

pub(super) fn soften_joint(joint: &mut GenericJoint) {
    joint.softness = SpringCoefficients::new(JOINT_NATURAL_FREQUENCY, JOINT_DAMPING_RATIO);
}

/// Raises the principal inertia of light bones next to heavy ones with an
/// isotropic, massless addition.
pub(super) fn apply_inertia_floor(
    world: &mut PhysicsWorld,
    bodies: &[RigidBodyHandle],
    spec: &RagdollSpec,
) {
    let mut smallest: Vec<Option<f32>> = bodies
        .iter()
        .zip(&spec.bodies)
        .map(|(handle, body)| {
            if body.anchor {
                return None;
            }
            let rb = world.bodies.get_mut(*handle)?;
            rb.recompute_mass_properties_from_colliders(&world.colliders);
            Some(
                rb.mass_properties()
                    .local_mprops
                    .principal_inertia()
                    .min_element(),
            )
        })
        .collect();
    let authored = smallest.clone();
    let mut changed = true;
    for _ in 0..spec.bodies.len() {
        changed = false;
        for constraint in &spec.constraints {
            for (light, heavy) in [
                (constraint.parent, constraint.child),
                (constraint.child, constraint.parent),
            ] {
                let (Some(light_inertia), Some(heavy_inertia)) = (smallest[light], smallest[heavy])
                else {
                    continue;
                };
                let floor = heavy_inertia * MIN_NEIGHBOUR_INERTIA_RATIO;
                if light_inertia < floor {
                    smallest[light] = Some(floor);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    debug_assert!(!changed, "inertia floor did not settle");
    for ((handle, before), after) in bodies.iter().zip(&authored).zip(&smallest) {
        let (Some(before), Some(after)) = (before, after) else {
            continue;
        };
        if let Some(rb) = world.bodies.get_mut(*handle).filter(|_| after > before) {
            let com = rb.local_center_of_mass();
            rb.set_additional_mass_properties(
                MassProperties::new(com, 0.0, Vec3::splat(after - before)),
                false,
            );
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::sim::ragdoll_spec::parse_ragdoll_spec;

    const DT: f32 = 1.0 / 60.0;
    const AUTHORED_DAMPING: f32 = 0.3;

    fn ball_shapes() -> Vec<CachedShape> {
        vec![CachedShape {
            shape: SharedShape::ball(0.5),
            local_pose: Pose::IDENTITY,
            joint: None,
            material: 0,
        }]
    }

    /// One free-floating ball (no gravity) tracked by a `RestTracker`.
    struct Fixture {
        world: PhysicsWorld,
        body: RigidBodyHandle,
        tracker: RestTracker,
    }

    impl Fixture {
        fn new() -> Self {
            let mut world = PhysicsWorld::new();
            world.gravity = Vec3::ZERO;
            world.integration_parameters.dt = DT;
            let body = world.insert_body(
                RigidBodyBuilder::dynamic()
                    .linear_damping(AUTHORED_DAMPING)
                    .angular_damping(AUTHORED_DAMPING),
            );
            world.insert_collider(ColliderBuilder::ball(0.5), Some(body));
            let bone = RestBone::new(body, &ball_shapes(), AUTHORED_DAMPING, AUTHORED_DAMPING);
            Self {
                world,
                body,
                tracker: RestTracker::new(vec![bone]),
            }
        }

        fn tick(&mut self, settings: &RagdollSettings) {
            self.world.step();
            self.tracker.step(&mut self.world, settings, DT);
        }

        fn body(&self) -> &RigidBody {
            &self.world.bodies[self.body]
        }
    }

    #[test]
    fn validated_clamps_each_field_to_its_range() {
        let requested = RagdollSettings {
            envelope: 99.0,
            window_seconds: 0.0,
            settle_damping: -5.0,
        };
        let validated = RagdollSettings::validated(requested, RagdollSettings::default());
        assert_eq!(validated.envelope, *RagdollSettings::ENVELOPE_RANGE.end());
        assert_eq!(
            validated.window_seconds,
            *RagdollSettings::WINDOW_SECONDS_RANGE.start()
        );
        assert_eq!(
            validated.settle_damping,
            *RagdollSettings::SETTLE_DAMPING_RANGE.start()
        );
    }

    #[test]
    fn validated_keeps_in_range_values_untouched() {
        let requested = RagdollSettings {
            envelope: 0.25,
            window_seconds: 1.5,
            settle_damping: 12.0,
        };
        assert_eq!(
            RagdollSettings::validated(requested, RagdollSettings::default()),
            requested
        );
    }

    #[test]
    fn validated_falls_back_to_the_current_value_for_non_finite_input() {
        let current = RagdollSettings {
            envelope: 0.2,
            window_seconds: 1.0,
            settle_damping: 4.0,
        };
        let requested = RagdollSettings {
            envelope: f32::NAN,
            window_seconds: f32::INFINITY,
            settle_damping: f32::NEG_INFINITY,
        };
        assert_eq!(RagdollSettings::validated(requested, current), current);
    }

    #[test]
    fn widen_range_widens_a_narrow_range_around_its_middle() {
        let [low, high] = widen_range([0.1, 0.12]);
        assert!((f32::midpoint(low, high) - 0.11).abs() < 1e-6);
        assert!((high - low - 2.0 * MIN_LIMIT_HALF_RANGE).abs() < 1e-6);
    }

    #[test]
    fn widen_range_leaves_locked_and_wide_ranges_alone() {
        assert_eq!(
            widen_range([0.3, 0.3]),
            [0.3, 0.3],
            "zero width is a locked axis"
        );
        assert_eq!(widen_range([-1.0, 1.0]), [-1.0, 1.0]);
    }

    #[test]
    fn widen_cone_raises_small_cones_but_keeps_a_locked_cone() {
        assert_eq!(widen_cone(0.0), 0.0);
        assert_eq!(widen_cone(0.01), MIN_LIMIT_HALF_RANGE);
        assert_eq!(widen_cone(1.0), 1.0);
    }

    #[test]
    fn frozen_activation_is_eligible_to_sleep_at_once() {
        let mut rb = RigidBodyBuilder::dynamic().build();
        prepare_activation(&mut rb, true);
        assert_eq!(rb.activation().normalized_linear_threshold, f32::INFINITY);
        assert_eq!(
            rb.activation().time_since_can_sleep,
            rb.activation().time_until_sleep
        );
    }

    #[test]
    fn unfrozen_activation_keeps_the_sleep_gate_closed() {
        let mut rb = RigidBodyBuilder::dynamic().build();
        prepare_activation(&mut rb, false);
        assert_eq!(rb.activation().normalized_linear_threshold, 0.0);
        assert_eq!(rb.activation().time_since_can_sleep, 0.0);
    }

    #[test]
    fn a_motionless_ragdoll_opens_the_sleep_gate_once_the_window_has_passed() {
        let settings = RagdollSettings::default();
        let mut fixture = Fixture::new();
        for _ in 0..25 {
            fixture.tick(&settings);
        }
        assert!(
            !fixture.tracker.gate_open(),
            "window of 0.5 s not yet elapsed"
        );
        assert!(fixture.tracker.is_settling());
        for _ in 0..10 {
            fixture.tick(&settings);
        }
        assert!(fixture.tracker.gate_open());
        assert_eq!(fixture.body().activation().angular_threshold, f32::INFINITY);
    }

    #[test]
    fn a_ragdoll_moving_beyond_the_envelope_never_opens_the_gate() {
        let settings = RagdollSettings::default();
        let mut fixture = Fixture::new();
        // 5 u/s covers 0.083 u per tick, far past the 0.1 u envelope over any 0.5 s window.
        fixture.world.bodies[fixture.body].set_linvel(Vec3::new(5.0, 0.0, 0.0), true);
        for _ in 0..120 {
            fixture.tick(&settings);
        }
        assert!(!fixture.tracker.gate_open());
        assert!(fixture.tracker.still_seconds() < settings.window_seconds);
    }

    #[test]
    fn leaving_the_envelope_restarts_the_window_and_closes_the_gate() {
        let settings = RagdollSettings::default();
        let mut fixture = Fixture::new();
        for _ in 0..40 {
            fixture.tick(&settings);
        }
        assert!(fixture.tracker.gate_open());
        fixture.world.bodies[fixture.body].set_translation(Vec3::new(1.0, 0.0, 0.0), true);
        fixture.tick(&settings);
        assert!(!fixture.tracker.gate_open());
        assert_eq!(fixture.tracker.still_seconds(), 0.0);
        assert_eq!(fixture.body().activation().angular_threshold, 0.0);
    }

    #[test]
    fn the_window_length_comes_from_the_settings() {
        let settings = RagdollSettings {
            window_seconds: *RagdollSettings::WINDOW_SECONDS_RANGE.end(),
            ..RagdollSettings::default()
        };
        let mut fixture = Fixture::new();
        for _ in 0..60 {
            fixture.tick(&settings);
        }
        assert!(
            !fixture.tracker.gate_open(),
            "a 3 s window is still open after 1 s"
        );
    }

    #[test]
    fn settle_damping_is_full_at_rest() {
        let settings = RagdollSettings::default();
        let mut fixture = Fixture::new();
        fixture.tick(&settings);
        let expected = AUTHORED_DAMPING + settings.settle_damping;
        assert!((fixture.body().linear_damping() - expected).abs() < 1e-4);
        assert!((fixture.body().angular_damping() - expected).abs() < 1e-4);
    }

    #[test]
    fn settle_damping_is_a_quarter_at_a_quarter_of_the_energy_threshold() {
        // e = E/4 gives d = 1 - sqrt(1/4) = 0.5, so the extra damping is 8 * 0.5^2 = 2.
        let speed = (2.0 * SETTLE_KINETIC_ENERGY / 4.0).sqrt();
        let settings = RagdollSettings::default();
        let mut fixture = Fixture::new();
        fixture.world.bodies[fixture.body].set_linvel(Vec3::new(speed, 0.0, 0.0), true);
        fixture.tracker.step(&mut fixture.world, &settings, DT);
        assert!((fixture.body().linear_damping() - (AUTHORED_DAMPING + 2.0)).abs() < 1e-4);
    }

    #[test]
    fn settle_damping_vanishes_at_the_energy_threshold() {
        let speed = (2.0 * SETTLE_KINETIC_ENERGY).sqrt() * 1.1;
        let mut fixture = Fixture::new();
        fixture.world.bodies[fixture.body].set_linvel(Vec3::new(speed, 0.0, 0.0), true);
        fixture
            .tracker
            .step(&mut fixture.world, &RagdollSettings::default(), DT);
        assert!((fixture.body().linear_damping() - AUTHORED_DAMPING).abs() < 1e-6);
    }

    #[test]
    fn ending_the_tracker_restores_authored_damping_and_closes_the_gate() {
        let settings = RagdollSettings::default();
        let mut fixture = Fixture::new();
        for _ in 0..40 {
            fixture.tick(&settings);
        }
        fixture.tracker.end(&mut fixture.world);
        assert!(!fixture.tracker.gate_open());
        assert!(!fixture.tracker.is_settling());
        assert_eq!(fixture.body().linear_damping(), AUTHORED_DAMPING);
        assert_eq!(fixture.body().activation().normalized_linear_threshold, 0.0);
    }

    fn inertia_floor_world() -> (PhysicsWorld, Vec<RigidBodyHandle>, RagdollSpec) {
        let body = |node: &str| {
            json!({
                "node": node, "mass": 1.0, "anchor": false, "damping": [0.0, 0.0],
                "rest": { "t": [0, 0, 0], "r": [0, 0, 0, 1] },
                "shapes": [{ "type": "sphere", "radius": 0.5 }],
            })
        };
        let joint = |child: usize, parent: usize| {
            json!({
                "type": "limited_hinge", "child": child, "parent": parent,
                "frame_child": { "t": [0, 0, 0], "r": [0, 0, 0, 1] },
                "frame_parent": { "t": [0, 0, 0], "r": [0, 0, 0, 1] },
                "hinge": [-1.0, 1.0], "friction": 0.0,
            })
        };
        let sidecar = json!({
            "bodies": [body("Heavy"), body("Light"), body("Stranger")],
            "constraints": [joint(1, 0)],
        });
        let spec = parse_ragdoll_spec(&serde_json::to_vec(&sidecar).unwrap()).unwrap();
        let mut world = PhysicsWorld::new();
        let radii_and_densities = [(2.0_f32, 10.0_f32), (0.1, 1.0), (0.1, 1.0)];
        let handles = radii_and_densities
            .iter()
            .map(|&(radius, density)| {
                let handle = world.insert_body(RigidBodyBuilder::dynamic());
                world.insert_collider(ColliderBuilder::ball(radius).density(density), Some(handle));
                handle
            })
            .collect();
        (world, handles, spec)
    }

    fn smallest_inertia(world: &PhysicsWorld, handle: RigidBodyHandle) -> f32 {
        world.bodies[handle]
            .mass_properties()
            .local_mprops
            .principal_inertia()
            .min_element()
    }

    #[test]
    fn inertia_floor_lifts_a_light_bone_next_to_a_heavy_one() {
        let (mut world, handles, spec) = inertia_floor_world();
        let heavy_before = smallest_inertia(&world, handles[0]);
        apply_inertia_floor(&mut world, &handles, &spec);
        world.step();
        let light_after = smallest_inertia(&world, handles[1]);
        let floor = heavy_before * MIN_NEIGHBOUR_INERTIA_RATIO;
        assert!(
            light_after >= floor * 0.999,
            "light {light_after} below floor {floor}"
        );
        assert_eq!(
            smallest_inertia(&world, handles[0]),
            heavy_before,
            "the heavy bone is untouched"
        );
    }

    #[test]
    fn inertia_floor_ignores_bones_without_a_joint_to_a_heavy_neighbour() {
        let (mut world, handles, spec) = inertia_floor_world();
        let stranger_before = smallest_inertia(&world, handles[2]);
        apply_inertia_floor(&mut world, &handles, &spec);
        world.step();
        assert_eq!(smallest_inertia(&world, handles[2]), stranger_before);
    }
}
