use std::f64::consts::TAU;

use glam::Vec3;

use super::day_time::DayTime;
use super::sky::SkyGameSettings;
use super::unmeasured::{STAR_ANGLE_AT_DAY_ZERO_RAD, STAR_ROTATION_SENSE};

/// The star dome's rotation: a right-handed turn of `angle_rad` about the unit `axis` in engine space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StarRotation {
    pub axis: Vec3,
    pub angle_rad: f32,
}

/// One revolution every `fStarsRotateDays` days, in the sense and from the
/// starting angle set in [`super::unmeasured`].
pub fn star_rotation(time: DayTime, settings: &SkyGameSettings) -> StarRotation {
    let revolutions = time.elapsed_days() / f64::from(settings.stars_rotate_days());
    let turn = STAR_ROTATION_SENSE * revolutions.fract() * TAU;
    StarRotation {
        axis: settings.stars_axis_engine(),
        angle_rad: (f64::from(STAR_ANGLE_AT_DAY_ZERO_RAD) + turn) as f32,
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals parsed or copied, not computed"
)]
mod tests {
    use std::f32::consts::{PI, TAU as TAU_F32};

    use glam::Quat;

    use super::super::test_support::default_climate;
    use super::super::test_support::{at, vanilla_settings};
    use super::super::unmeasured::sun_position;
    use super::*;
    use crate::bethesda_space::axes_to_engine;

    fn rotation_at(days: u32, hour: f32) -> StarRotation {
        star_rotation(at(days, hour), &vanilla_settings())
    }

    fn circular_gap(a: f32, b: f32) -> f32 {
        let gap = (a - b).rem_euclid(TAU_F32);
        gap.min(TAU_F32 - gap)
    }

    fn turn(rotation: StarRotation, direction: Vec3) -> Vec3 {
        Quat::from_axis_angle(rotation.axis, rotation.angle_rad) * direction
    }

    #[test]
    fn the_axis_is_the_bethesda_axis_in_engine_space_as_a_unit_vector() {
        let rotation = rotation_at(0, 0.0);
        let expected = axes_to_engine(-0.09, 0.35, 1.0).normalize();
        assert!(rotation.axis.abs_diff_eq(expected, 1e-6));
        assert!((rotation.axis.length() - 1.0).abs() < 1e-6);
        assert!(rotation.axis.y > 0.9, "mostly up in engine space");
        assert!(rotation.axis.z < 0.0, "Bethesda +Y is engine -Z");
    }

    #[test]
    fn the_angle_is_the_start_angle_at_the_start_of_day_zero() {
        assert_eq!(rotation_at(0, 0.0).angle_rad, STAR_ANGLE_AT_DAY_ZERO_RAD);
    }

    #[test]
    fn one_period_is_one_revolution() {
        let rotation = rotation_at(1, 0.0027 * 24.0);
        assert!(
            circular_gap(rotation.angle_rad, 0.0) < 1e-3,
            "{}",
            rotation.angle_rad
        );
    }

    #[test]
    fn half_a_period_is_half_a_revolution() {
        let rotation = rotation_at(0, 0.5 * 1.0027 * 24.0);
        assert!((rotation.angle_rad.abs() - PI).abs() < 1e-3);
    }

    #[test]
    fn many_days_later_the_angle_is_the_fractional_revolution() {
        let rotation = rotation_at(1000, 0.0);
        let revolutions = 1000.0_f64 / f64::from(1.0027_f32);
        let expected = (STAR_ROTATION_SENSE * revolutions.fract() * std::f64::consts::TAU) as f32;
        assert!((rotation.angle_rad - expected).abs() < 1e-4);
        assert!(rotation.angle_rad.abs() <= TAU_F32);
    }

    #[test]
    fn the_angle_is_continuous_over_midnight() {
        let before = rotation_at(4, 23.9999).angle_rad;
        let after = rotation_at(5, 0.0).angle_rad;
        assert!(circular_gap(before, after) < 1e-3);
    }

    #[test]
    fn the_angle_stays_within_one_turn_for_any_day_count() {
        for days in [0, 1, 365, 100_000, u32::MAX] {
            let angle = rotation_at(days, 13.0).angle_rad;
            assert!(angle.abs() <= TAU_F32, "{days}: {angle}");
        }
    }

    #[test]
    fn a_star_on_the_east_horizon_moves_towards_the_south_like_the_sun() {
        let east = Vec3::X;
        let later = turn(rotation_at(0, 1.0), east);
        assert!(later.z > 0.1, "engine +Z is south: {later:?}");
        assert!(later.y > 0.0, "and it climbs: {later:?}");

        let times = default_climate();
        let settings = vanilla_settings();
        let sun_before = sun_position(at(0, 7.0), &times, &settings).direction();
        let sun_after = sun_position(at(0, 8.0), &times, &settings).direction();
        assert!(sun_after.z > sun_before.z, "the sun heads south too");
    }

    #[test]
    fn a_star_on_the_south_meridian_moves_towards_the_west() {
        let later = turn(rotation_at(0, 1.0), Vec3::Z);
        assert!(later.x < -0.1, "engine -X is west: {later:?}");
    }
}
