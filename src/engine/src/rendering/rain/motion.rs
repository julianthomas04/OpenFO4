//! The numbers the rain's motion rests on: how many drops and splashes fall, where the lattice
//! the drops sit on has moved to, and which way a drop's streak points on screen. The GPU
//! derives every drop from its index; nothing here is per drop.

use glam::{DVec3, Vec2, Vec3};

use super::tuning::{
    DRIFT_AT_FULL_WIND, DROPS_PER_DENSITY, MAX_DROPS, MAX_SPLASHES, SPLASH_SECONDS,
};
use crate::bethesda_space::axes_to_engine;

/// How many drops fall at `intensity` (0 to 1) in weather of the given particle `density`.
pub fn drop_count(intensity: f32, density: f32) -> u32 {
    let wanted = intensity.clamp(0.0, 1.0) * density.max(0.0) * DROPS_PER_DENSITY;
    (wanted.round() as u32).min(MAX_DROPS)
}

/// How many splashes play at `intensity` (0 to 1).
pub fn splash_count(intensity: f32) -> u32 {
    (intensity.clamp(0.0, 1.0) * MAX_SPLASHES as f32).round() as u32
}

/// The horizontal drift of a drop on the engine's x and z axes, in game units per second, in a
/// wind of `speed` (0 to 1) that blows towards `heading`, radians from +X in the Bethesda XY plane.
pub fn wind_drift(speed: f32, heading: f32) -> Vec2 {
    let towards = axes_to_engine(heading.cos(), heading.sin(), 0.0);
    Vec2::new(towards.x, towards.z) * (speed * DRIFT_AT_FULL_WIND)
}

/// The velocity of a drop in engine units per second: straight down, carried along by the wind.
pub fn fall_velocity(fall_speed: f32, wind: Vec2) -> Vec3 {
    Vec3::new(wind.x, -fall_speed, wind.y)
}

/// The axes of a drop's streak: `along` runs up the streak against the drop's velocity, and
/// `across` is perpendicular to it and to the camera's view, so the streak faces the camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreakAxes {
    pub along: Vec3,
    pub across: Vec3,
}

impl StreakAxes {
    pub fn new(velocity: Vec3, forward: Vec3) -> Self {
        let along = (-velocity).try_normalize().unwrap_or(Vec3::Y);
        let across = along
            .cross(forward)
            .try_normalize()
            .unwrap_or_else(|| along.any_orthonormal_vector());
        Self { along, across }
    }
}

/// How far the drops have drifted along their velocity. It builds up step by step, because the
/// wind changes from frame to frame: a position computed from the velocity and the whole time
/// would jump with every change of it.
#[derive(Debug, Default)]
pub struct DropDrift {
    offset: DVec3,
    time: Option<f64>,
}

impl DropDrift {
    /// Moves the drops along `velocity` for the time since the last call, and keeps the total
    /// within one `edge` of the box so it stays exact however long the game has run. The first
    /// call, a time that is not a number and a time before the last one move nothing.
    pub fn advance(&mut self, velocity: Vec3, time: f64, edge: f32) {
        if !time.is_finite() {
            return;
        }
        let elapsed = self
            .time
            .replace(time)
            .map_or(0.0, |last| (time - last).max(0.0));
        self.offset =
            (self.offset + velocity.as_dvec3() * elapsed).rem_euclid(DVec3::splat(f64::from(edge)));
    }

    pub fn offset(&self) -> DVec3 {
        self.offset
    }
}

/// Where the lattice of drop positions lies relative to the camera, as the shader needs it: the
/// `drift` of the lattice less where the camera is within its box cell, both reduced to within
/// one `edge` of the box in double precision. A drop's offset from the camera is its place in
/// the lattice plus this, wrapped into the box.
pub fn lattice_shift(drift: DVec3, eye: Vec3, edge: f32) -> Vec3 {
    let edge = DVec3::splat(f64::from(edge));
    let camera = eye.as_dvec3().rem_euclid(edge);
    (drift.rem_euclid(edge) - camera).as_vec3()
}

/// How far into its life the splashes are: the share of the current cycle passed, and the count
/// of cycles started, which makes each splash land somewhere new.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplashPhase {
    pub cycle: u32,
    pub fraction: f32,
}

impl SplashPhase {
    pub fn at(time: f64) -> Self {
        let cycles = time / SPLASH_SECONDS;
        Self {
            cycle: cycles.floor().rem_euclid(f64::from(u32::MAX) + 1.0) as u32,
            fraction: cycles.rem_euclid(1.0) as f32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    #[test]
    fn the_drops_drift_with_the_wind_on_the_engine_axes() {
        let east = wind_drift(1.0, 0.0);
        assert!((east - Vec2::new(DRIFT_AT_FULL_WIND, 0.0)).length() < 1e-3);

        // Bethesda's +Y is the engine's -Z.
        let north = wind_drift(0.5, FRAC_PI_2);
        assert!((north - Vec2::new(0.0, -0.5 * DRIFT_AT_FULL_WIND)).length() < 1e-3);
    }

    #[test]
    fn still_air_lets_the_drops_fall_straight() {
        assert_eq!(wind_drift(0.0, 1.0), Vec2::ZERO);
    }

    const EDGE: f32 = 120.0;
    const EYE: Vec3 = Vec3::new(1234.5, 300.0, -987.25);

    /// The position of a lattice point `place` (0 to 1 across the box) relative to the camera, the
    /// way the shader reads the lattice: wrapped into the box around the camera.
    fn offset_from_camera(place: Vec3, shift: Vec3) -> Vec3 {
        let wrapped = place * EDGE + shift;
        wrapped - EDGE * (wrapped / EDGE + 0.5).floor()
    }

    fn assert_close(actual: Vec3, expected: Vec3, tolerance: f32, what: &str) {
        assert!(
            actual.abs_diff_eq(expected, tolerance),
            "{what}: {actual} is not {expected}"
        );
    }

    #[test]
    fn no_rain_has_no_drops_and_no_splashes() {
        assert_eq!(drop_count(0.0, 7.0), 0);
        assert_eq!(drop_count(1.0, 0.0), 0);
        assert_eq!(splash_count(0.0), 0);
    }

    #[test]
    fn the_drops_grow_with_the_intensity_and_the_density_of_the_weather() {
        let one = drop_count(1.0, 1.0);
        assert_eq!(one, DROPS_PER_DENSITY.round() as u32);
        assert!(drop_count(1.0, 2.0).abs_diff(2 * one) <= 1);
        assert!(drop_count(0.5, 2.0).abs_diff(one) <= 1);
    }

    #[test]
    fn the_drops_stop_at_the_budget_of_the_pass() {
        assert_eq!(drop_count(1.0, 1.0e6), MAX_DROPS);
        assert_eq!(
            drop_count(5.0, 2.0),
            drop_count(1.0, 2.0),
            "intensity stops at one"
        );
    }

    #[test]
    fn the_splashes_grow_with_the_intensity_up_to_their_budget() {
        assert_eq!(splash_count(1.0), MAX_SPLASHES);
        assert!(splash_count(0.25).abs_diff(MAX_SPLASHES / 4) <= 1);
        assert_eq!(splash_count(3.0), MAX_SPLASHES);
    }

    #[test]
    fn counts_of_values_that_are_no_numbers_or_negative_are_zero() {
        for bad in [f32::NAN, -1.0, f32::NEG_INFINITY] {
            assert_eq!(drop_count(bad, 7.0), 0, "intensity {bad}");
            assert_eq!(splash_count(bad), 0, "intensity {bad}");
        }
        assert_eq!(drop_count(1.0, f32::NAN), 0);
        assert_eq!(drop_count(1.0, -4.0), 0);
    }

    #[test]
    fn a_drop_falls_down_and_the_wind_carries_it_sideways() {
        assert_eq!(
            fall_velocity(75.0, Vec2::new(3.0, -2.0)),
            Vec3::new(3.0, -75.0, -2.0)
        );
    }

    #[test]
    fn a_streak_runs_up_against_the_velocity_of_its_drop() {
        let axes = StreakAxes::new(fall_velocity(75.0, Vec2::ZERO), Vec3::NEG_Z);
        assert_close(axes.along, Vec3::Y, 1e-6, "calm air");
        let slanted = StreakAxes::new(fall_velocity(75.0, Vec2::new(25.0, 0.0)), Vec3::NEG_Z);
        assert!(slanted.along.x < 0.0, "drops blown to +x lean back to -x");
        assert!((slanted.along.x / slanted.along.y + 25.0 / 75.0).abs() < 1e-6);
    }

    #[test]
    fn a_streak_faces_the_camera_whatever_the_view_direction() {
        let velocity = fall_velocity(75.0, Vec2::new(10.0, 4.0));
        for forward in [
            Vec3::NEG_Z,
            Vec3::X,
            Vec3::new(0.3, -0.2, 0.9).normalize(),
            Vec3::new(-0.6, 0.7, -0.1).normalize(),
        ] {
            let axes = StreakAxes::new(velocity, forward);
            assert!((axes.along.length() - 1.0).abs() < 1e-5);
            assert!((axes.across.length() - 1.0).abs() < 1e-5);
            assert!(axes.across.dot(axes.along).abs() < 1e-5, "across {forward}");
            assert!(axes.across.dot(forward).abs() < 1e-5, "facing {forward}");
        }
    }

    #[test]
    fn a_streak_seen_end_on_still_has_an_across_axis() {
        let velocity = fall_velocity(75.0, Vec2::ZERO);
        for forward in [Vec3::Y, Vec3::NEG_Y] {
            let axes = StreakAxes::new(velocity, forward);
            assert!((axes.across.length() - 1.0).abs() < 1e-5);
            assert!(axes.across.dot(axes.along).abs() < 1e-5);
        }
    }

    #[test]
    fn a_still_drop_points_up() {
        let axes = StreakAxes::new(Vec3::ZERO, Vec3::NEG_Z);
        assert_eq!(axes.along, Vec3::Y);
        assert!((axes.across.length() - 1.0).abs() < 1e-5);
    }

    const STEP: f64 = 1.0 / 60.0;

    fn drifted(velocity: Vec3, seconds: f64) -> DropDrift {
        let mut drift = DropDrift::default();
        let steps = (seconds / STEP).round() as u32;
        for step in 0..=steps {
            drift.advance(velocity, f64::from(step) * STEP, EDGE);
        }
        drift
    }

    #[test]
    fn the_shift_stays_within_one_box_edge_however_far_the_camera_and_the_clock_have_gone() {
        let velocity = fall_velocity(75.0, Vec2::new(10.0, -4.0));
        for (seconds, eye) in [
            (0.0, Vec3::ZERO),
            (12.5, EYE),
            (600.0, Vec3::new(-52_000.0, 3000.0, 49_000.0)),
        ] {
            let shift = lattice_shift(drifted(velocity, seconds).offset(), eye, EDGE);
            assert!(
                shift.abs().max_element() <= EDGE,
                "after {seconds} s, eye {eye}: {shift}"
            );
        }
    }

    #[test]
    fn a_drop_stays_where_it_is_in_the_world_while_the_camera_moves() {
        let drift = drifted(fall_velocity(75.0, Vec2::new(10.0, -4.0)), 3.0);
        let place = Vec3::new(0.2, 0.7, 0.45);
        let before = offset_from_camera(place, lattice_shift(drift.offset(), EYE, EDGE));
        for step in [
            Vec3::new(5.0, 0.0, 0.0),
            Vec3::new(-17.5, 2.0, 30.25),
            Vec3::new(0.0, -3.0, 0.5),
        ] {
            let after = offset_from_camera(place, lattice_shift(drift.offset(), EYE + step, EDGE));
            let expected = before - step;
            let wrapped_expected = expected - EDGE * (expected / EDGE + 0.5).floor();
            assert_close(
                after,
                wrapped_expected,
                1e-3,
                &format!("camera moved {step}"),
            );
        }
    }

    #[test]
    fn a_drop_moves_along_its_velocity_as_time_passes() {
        let velocity = fall_velocity(75.0, Vec2::new(10.0, -4.0));
        let place = Vec3::new(0.2, 0.7, 0.45);
        let mut drift = DropDrift::default();
        drift.advance(velocity, 5.0, EDGE);
        let before = offset_from_camera(place, lattice_shift(drift.offset(), EYE, EDGE));
        drift.advance(velocity, 5.1, EDGE);
        let after = offset_from_camera(place, lattice_shift(drift.offset(), EYE, EDGE));
        assert_close(after - before, velocity * 0.1, 1e-3, "a tenth of a second");
    }

    #[test]
    fn a_change_of_the_wind_moves_the_drops_by_one_step_and_not_by_the_whole_run() {
        let place = Vec3::new(0.2, 0.7, 0.45);
        let mut drift = drifted(fall_velocity(75.0, Vec2::ZERO), 600.0);
        let before = offset_from_camera(place, lattice_shift(drift.offset(), EYE, EDGE));
        let gust = fall_velocity(75.0, Vec2::new(40.0, 0.0));
        drift.advance(gust, 600.0 + STEP, EDGE);
        let after = offset_from_camera(place, lattice_shift(drift.offset(), EYE, EDGE));
        assert_close(
            after - before,
            gust * STEP as f32,
            1e-3,
            "one step in the gust",
        );
    }

    #[test]
    fn the_rain_keeps_its_steps_after_a_very_long_run() {
        let velocity = fall_velocity(75.0, Vec2::new(10.0, -4.0));
        let seconds = 3.0 * 3600.0;
        let drift = drifted(velocity, seconds);
        let exact = (velocity.as_dvec3() * seconds).rem_euclid(DVec3::splat(f64::from(EDGE)));
        let error = (drift.offset() - exact).abs().max_element();
        let wrapped_error = error.min(f64::from(EDGE) - error);
        assert!(
            wrapped_error < 1e-6,
            "off by {wrapped_error} after {seconds} s"
        );
    }

    #[test]
    fn the_drift_ignores_the_first_call_a_time_going_back_and_a_time_that_is_no_number() {
        let velocity = fall_velocity(75.0, Vec2::new(10.0, -4.0));
        let mut drift = DropDrift::default();
        drift.advance(velocity, 100.0, EDGE);
        assert_eq!(
            drift.offset(),
            DVec3::ZERO,
            "the first call has no step yet"
        );
        drift.advance(velocity, 99.0, EDGE);
        assert_eq!(drift.offset(), DVec3::ZERO, "time went back");
        drift.advance(velocity, f64::NAN, EDGE);
        drift.advance(velocity, f64::INFINITY, EDGE);
        assert_eq!(drift.offset(), DVec3::ZERO, "no usable time");
        drift.advance(velocity, 99.5, EDGE);
        assert_ne!(
            drift.offset(),
            DVec3::ZERO,
            "time runs on from the earlier call"
        );
    }

    #[test]
    fn a_splash_plays_through_its_cycle_and_starts_another() {
        let start = SplashPhase::at(0.0);
        assert_eq!((start.cycle, start.fraction), (0, 0.0));
        let middle = SplashPhase::at(SPLASH_SECONDS * 0.5);
        assert_eq!((middle.cycle, middle.fraction), (0, 0.5));
        let next = SplashPhase::at(SPLASH_SECONDS);
        assert_eq!((next.cycle, next.fraction), (1, 0.0));
        let later = SplashPhase::at(SPLASH_SECONDS * 41.25);
        assert_eq!(later.cycle, 41);
        assert!((later.fraction - 0.25).abs() < 1e-6);
    }

    #[test]
    fn the_splash_phase_stays_exact_after_a_very_long_run() {
        let phase = SplashPhase::at(SPLASH_SECONDS * 4.0e9 + SPLASH_SECONDS * 0.75);
        assert_eq!(phase.cycle, 4_000_000_000);
        assert!((phase.fraction - 0.75).abs() < 1e-6);
    }

    #[test]
    fn the_splash_cycle_wraps_instead_of_failing_past_the_count_that_fits() {
        let phase = SplashPhase::at(SPLASH_SECONDS * (f64::from(u32::MAX) + 3.0));
        assert_eq!(phase.cycle, 2);
    }
}
