//! Settings and helpers every dynamic rigid body shares, whether it is a single
//! piece of Havok-movable clutter (`physics.rs`) or one bone of a ragdoll
//! (`ragdoll.rs`): tuning constants, the body builder, the mass-to-density
//! back-solve, the simulation-radius test and the post-step velocity bounds.

use rapier3d::prelude::*;

use super::bake::CachedShape;
use super::{CellGrid, FIXED_DT};

/// Engine-unit gravity for dynamic bodies: the player's base rise gravity
/// (`player::GRAVITY` = 9.8 m/s² at 1 unit ≈ 14.3 cm), NOT rapier's metric
/// default. The kinematic player integrates its own gravity; this acts only on
/// the rigid bodies.
pub(super) const GRAVITY_DYN: f32 = 68.59;
/// Light damping so pushed/falling clutter settles quickly. The radius gates
/// which bodies are Dynamic (vs frozen Fixed); rapier's per-body island
/// sleeping then lets settled clutter rest, so the world stops re-stepping the
/// whole collision ring every tick once nothing is moving. A push or a fresh
/// activation wakes bodies and resumes stepping. (The radius is the activation
/// mechanism, not sleeping-distance culling.)
pub(super) const DYN_LINEAR_DAMPING: f32 = 0.2;
pub(super) const DYN_ANGULAR_DAMPING: f32 = 0.4;
/// Fallback density for a dynamic body whose sidecar carries no usable Havok
/// mass (shouldn't happen — `dynamic` implies a positive mass) or whose shapes
/// have ~zero volume. Rapier's default; mass then derives from shape volume.
pub(super) const DYN_FALLBACK_DENSITY: f32 = 1.0;
/// Below this total solid volume the density back-solve would run away (or the
/// body is a near-zero-volume open shell), so it falls back to a fixed density.
pub(super) const VOLUME_EPS: f32 = 1.0e-3;
/// Clamp the back-solved density so a small-but-finite volume can't yield a
/// runaway mass and near-singular inertia tensor (the chief source of bodies
/// flinging away). Solid clutter sits comfortably inside this range.
pub(super) const DYN_MIN_DENSITY: f32 = 1.0e-3;
pub(super) const DYN_MAX_DENSITY: f32 = 50.0;
/// Soft-CCD prediction distance for dynamic bodies (engine units). Soft-CCD is
/// the cheap, predictive-constraint variant of continuous collision detection:
/// unlike full `ccd_enabled` it does no shape-cast substeps, so it cannot stall
/// the step. Its lookahead MUST cover a body's per-step motion at terminal fall
/// speed (`GRAVITY_DYN / DYN_LINEAR_DAMPING ≈ 343 u/s` → ~5.7 u/step) — otherwise
/// a fast faller out-runs the predictive window and tunnels through thin terrain
/// in one tick. The ×1.1 is headroom; speeds beyond terminal are only reachable
/// via instability and are caught next tick by `MAX_DYN_LINVEL`. (Full CCD's
/// per-fast-body shape-cast froze the sim; this stays cheap, bounded by the
/// radius-gated awake set.)
pub(super) const DYN_SOFT_CCD_PREDICTION: f32 = GRAVITY_DYN / DYN_LINEAR_DAMPING * FIXED_DT * 1.1;
/// Post-step velocity ceilings: a safety net so a residual instability can't
/// compound into an explosion. The linear ceiling is the soft-CCD window speed
/// (`DYN_SOFT_CCD_PREDICTION / FIXED_DT`): pinning the two together means a body
/// the post-step check admits can never have out-run soft-CCD in the step it
/// just took, closing the tunneling band that a higher ceiling would open.
/// Since `DYN_SOFT_CCD_PREDICTION` carries a ×1.1 margin over terminal-fall
/// motion, this ceiling is exactly 1.1× the terminal fall speed — above it by
/// construction, so it never clamps a legitimate long fall. The static assert
/// guards that margin: gravity, damping, and `FIXED_DT` all cancel algebraically,
/// so it reduces to `1.1 > 1.0` and fails to compile only if that headroom
/// literal in `DYN_SOFT_CCD_PREDICTION` is ever cut to ≤1.0.
pub(super) const MAX_DYN_LINVEL: f32 = DYN_SOFT_CCD_PREDICTION / FIXED_DT;
pub(super) const MAX_DYN_ANGVEL: f32 = 30.0;
const _: () = assert!(MAX_DYN_LINVEL > GRAVITY_DYN / DYN_LINEAR_DAMPING);

/// A dynamic body with the engine's shared settings. Soft-CCD (NOT full
/// `ccd_enabled`): a body falling at `GRAVITY_DYN` moves several units per 1/60 s
/// step and a discrete step can miss a thin surface. Soft-CCD adds predictive
/// contacts along the motion to catch that. Full CCD's shape-cast substeps — run
/// per fast body across all clutter — blew the step budget and stalled the sim;
/// this is the cheap predictive variant. The setting survives a Fixed→Dynamic flip.
pub(super) fn body_builder(
    body_type: RigidBodyType,
    linear_damping: f32,
    angular_damping: f32,
) -> RigidBodyBuilder {
    RigidBodyBuilder::new(body_type)
        .soft_ccd_prediction(DYN_SOFT_CCD_PREDICTION)
        .linear_damping(linear_damping)
        .angular_damping(angular_damping)
}

/// Uniform collider density that lands the body's Fallout 4 (Havok) mass: the
/// mass is a relative gameplay "heaviness", used directly (no kg conversion).
/// It is back-solved from the body's true (solid) collider volume — the same
/// quantity rapier multiplies the density by per collider — rather than
/// `additional_mass` (zeroes angular inertia → rotation-locked) or a per-shape
/// split (misplaces the COM): this lands the exact FO4 mass with a correct COM
/// and inertia tensor. The clamp bounds a thin sliver's runaway density
/// (near-singular inertia is the chief "fling away" source). An open-shell
/// trimesh has ~zero solid volume so contributes ~no mass; a purely-trimesh
/// dynamic body thus falls to the fallback density and ends near-massless —
/// rare for movable clutter, which uses primitive/convex collision (known
/// limitation).
pub(super) fn density_for_mass(shapes: &[CachedShape], mass: Option<f32>) -> f32 {
    let volume: f32 = shapes
        .iter()
        .map(|cached| cached.shape.mass_properties(1.0).mass())
        .sum();
    match mass.filter(|mass| mass.is_finite() && *mass > 0.0) {
        Some(mass) if volume > VOLUME_EPS => {
            (mass / volume).clamp(DYN_MIN_DENSITY, DYN_MAX_DENSITY)
        }
        _ => DYN_FALLBACK_DENSITY,
    }
}

/// Whether `grid` lies inside the Chebyshev radius around `center`.
pub(super) fn within_radius(grid: CellGrid, center: CellGrid, radius: i32) -> bool {
    (grid.0 - center.0).abs().max((grid.1 - center.1).abs()) <= radius
}

/// Whether a body's pose and velocities are all finite (no NaN/inf component).
pub(super) fn body_finite(pos: &Pose, rb: &RigidBody) -> bool {
    pos.translation.is_finite()
        && pos.rotation.is_finite()
        && rb.linvel().is_finite()
        && rb.angvel().is_finite()
}

/// Whether a body's state is finite and inside the velocity ceilings. Read-only,
/// so the common case never takes the body mutably: `get_mut` flags the body
/// `modified` every tick and defeats the idle-step gate.
pub(super) fn state_is_bounded(rb: &RigidBody) -> bool {
    body_finite(rb.position(), rb)
        && rb.linvel().length() <= MAX_DYN_LINVEL
        && rb.angvel().length() <= MAX_DYN_ANGVEL
}

/// Rescale `v` to magnitude `max` if it exceeds it, else `None` — so a caller
/// skips touching the body (and flagging it modified) when no clamp is needed.
fn clamp_to_max(v: Vec3, max: f32) -> Option<Vec3> {
    let speed = v.length();
    (speed > max).then(|| v * (max / speed))
}

/// Rescale a body's linear velocity to `max` if it exceeds it (no-op otherwise).
pub(super) fn clamp_linvel(rb: &mut RigidBody, max: f32, wake: bool) {
    if let Some(v) = clamp_to_max(rb.linvel(), max) {
        rb.set_linvel(v, wake);
    }
}

/// Bound a dynamic body's post-step state so a single instability can't
/// compound into an explosion or poison the render readback, and return the
/// pose to record for interpolation. A non-finite pose/velocity is recovered
/// to the last good pose and the body is put to sleep (and logged, so a
/// persistent source surfaces instead of being silently masked every frame);
/// an over-speed body is clamped. `wake_up = false` throughout: this must not
/// keep a settling body awake. Called only on the rare body that fails
/// [`state_is_bounded`], so the common path never reaches here.
pub(super) fn sanitize_body(rb: &mut RigidBody, last_good: Pose) -> Pose {
    let pos = *rb.position();
    if !body_finite(&pos, rb) {
        tracing::warn!("physics: recovered a non-finite dynamic body to its last good pose");
        rb.set_linvel(Vec3::ZERO, false);
        rb.set_angvel(Vec3::ZERO, false);
        rb.set_position(last_good, false);
        rb.sleep();
        return last_good;
    }
    clamp_linvel(rb, MAX_DYN_LINVEL, false);
    if let Some(ang) = clamp_to_max(rb.angvel(), MAX_DYN_ANGVEL) {
        rb.set_angvel(ang, false);
    }
    pos
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use super::*;

    fn cuboid(half: f32) -> CachedShape {
        CachedShape {
            shape: SharedShape::cuboid(half, half, half),
            local_pose: Pose::IDENTITY,
            joint: None,
            material: 0,
        }
    }

    /// A cuboid of half extent 0.5 has a solid volume of exactly 1.
    fn unit_cube() -> Vec<CachedShape> {
        vec![cuboid(0.5)]
    }

    #[test]
    fn density_makes_the_body_weigh_the_requested_mass() {
        assert!((density_for_mass(&unit_cube(), Some(2.5)) - 2.5).abs() < 1e-5);
        let two_cubes = [cuboid(0.5), cuboid(0.5)];
        assert!(
            (density_for_mass(&two_cubes, Some(2.0)) - 1.0).abs() < 1e-5,
            "volume of all colliders counts"
        );
    }

    #[test]
    fn density_is_clamped_to_a_sane_range() {
        assert_eq!(density_for_mass(&unit_cube(), Some(1.0e6)), DYN_MAX_DENSITY);
        assert_eq!(
            density_for_mass(&unit_cube(), Some(1.0e-9)),
            DYN_MIN_DENSITY
        );
    }

    #[test]
    fn density_falls_back_without_a_usable_mass() {
        for mass in [
            None,
            Some(0.0),
            Some(-3.0),
            Some(f32::NAN),
            Some(f32::INFINITY),
        ] {
            assert_eq!(
                density_for_mass(&unit_cube(), mass),
                DYN_FALLBACK_DENSITY,
                "{mass:?}"
            );
        }
    }

    #[test]
    fn density_falls_back_for_shapes_without_volume() {
        assert_eq!(density_for_mass(&[], Some(5.0)), DYN_FALLBACK_DENSITY);
        let sliver = [cuboid(0.01)];
        assert_eq!(
            density_for_mass(&sliver, Some(5.0)),
            DYN_FALLBACK_DENSITY,
            "volume 8e-6 is below the epsilon"
        );
    }

    #[test]
    fn the_radius_is_a_chebyshev_distance_in_cells() {
        let center = (10, -4);
        assert!(
            within_radius((12, -2), center, 2),
            "diagonal corner is inside"
        );
        assert!(within_radius(center, center, 0));
        assert!(!within_radius((13, -4), center, 2));
        assert!(!within_radius((10, -7), center, 2));
        assert!(!within_radius((11, -4), center, 0));
    }

    fn dynamic_body() -> RigidBody {
        RigidBodyBuilder::dynamic().build()
    }

    #[test]
    fn finite_slow_bodies_are_bounded() {
        let mut body = dynamic_body();
        body.set_linvel(Vec3::new(10.0, 0.0, 0.0), false);
        assert!(state_is_bounded(&body));
    }

    #[test]
    fn bodies_over_the_speed_ceilings_or_non_finite_are_not_bounded() {
        let mut fast = dynamic_body();
        fast.set_linvel(Vec3::new(MAX_DYN_LINVEL * 1.01, 0.0, 0.0), false);
        assert!(!state_is_bounded(&fast));
        let mut spinning = dynamic_body();
        spinning.set_angvel(Vec3::new(0.0, MAX_DYN_ANGVEL * 1.01, 0.0), false);
        assert!(!state_is_bounded(&spinning));
        let mut broken = dynamic_body();
        broken.set_linvel(Vec3::new(f32::NAN, 0.0, 0.0), false);
        assert!(!state_is_bounded(&broken));
    }

    #[test]
    fn sanitising_clamps_speeds_and_keeps_the_pose() {
        let mut body = dynamic_body();
        body.set_position(Pose::from_translation(Vec3::new(1.0, 2.0, 3.0)), false);
        body.set_linvel(Vec3::new(MAX_DYN_LINVEL * 4.0, 0.0, 0.0), false);
        body.set_angvel(Vec3::new(0.0, MAX_DYN_ANGVEL * 3.0, 0.0), false);
        let pose = sanitize_body(&mut body, Pose::IDENTITY);
        assert_eq!(pose.translation, Vec3::new(1.0, 2.0, 3.0));
        assert!((body.linvel().length() - MAX_DYN_LINVEL).abs() < 1e-2);
        assert!((body.angvel().length() - MAX_DYN_ANGVEL).abs() < 1e-3);
    }

    #[test]
    fn sanitising_a_non_finite_body_restores_the_last_good_pose_and_puts_it_to_sleep() {
        let mut body = dynamic_body();
        body.set_position(Pose::from_translation(Vec3::new(f32::NAN, 0.0, 0.0)), false);
        body.set_linvel(Vec3::new(5.0, 0.0, 0.0), false);
        let good = Pose::from_translation(Vec3::new(7.0, 8.0, 9.0));
        let pose = sanitize_body(&mut body, good);
        assert_eq!(pose.translation, good.translation);
        assert_eq!(body.translation(), good.translation);
        assert_eq!(body.linvel(), Vec3::ZERO);
        assert!(body.is_sleeping());
    }
}
