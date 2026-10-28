//! How far the sky has turned from solar noon, which times both the directional light and the
//! sun disc. Measured under two climates (sunrise 05-09 / sunset 17-21 and 03-05 / 15-21): solar
//! noon is halfway between the twilight midpoints, the sky turns a quarter circle from the sunrise
//! midpoint to solar noon and another from solar noon to the sunset midpoint, and the other half
//! circle over the night between the sunset midpoint and the next sunrise midpoint.

use std::f32::consts::FRAC_PI_2;

use super::climate_times::ClimateTimes;
use super::wrap::{HOURS_PER_DAY, wrap_hours};

/// The hour the light and the sun disc stand due south: halfway between the sunrise and sunset
/// midpoints.
pub fn solar_noon(times: &ClimateTimes) -> f32 {
    f32::midpoint(times.sunrise_midpoint(), times.sunset_midpoint())
}

/// The turn from solar noon in radians: negative before it, positive after it, a quarter circle
/// at the twilight midpoints and a half circle at solar midnight, halfway through the night.
pub fn turn_from_noon(hour: f32, times: &ClimateTimes) -> f32 {
    let half_day = HOURS_PER_DAY / 2.0;
    let offset = wrap_hours(hour - solar_noon(times) + half_day) - half_day;
    let day_leg = (times.sunset_midpoint() - times.sunrise_midpoint()) / 2.0;
    let night_leg = half_day - day_leg;
    let turn = if offset.abs() <= day_leg {
        FRAC_PI_2 * offset.abs() / day_leg
    } else {
        FRAC_PI_2 + FRAC_PI_2 * (offset.abs() - day_leg) / night_leg
    };
    turn.copysign(offset)
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{PI, TAU};

    use super::super::test_support::default_climate;
    use super::*;

    fn second_climate() -> ClimateTimes {
        ClimateTimes::new(3.0, 5.0, 15.0, 21.0).unwrap()
    }

    fn assert_close(actual: f32, expected: f32, what: &str) {
        assert!(
            (actual - expected).abs() <= 1e-5,
            "{what}: expected {expected}, got {actual}"
        );
    }

    #[test]
    fn solar_noon_is_halfway_between_the_twilight_midpoints() {
        assert_close(solar_noon(&default_climate()), 13.0, "default");
        assert_close(solar_noon(&second_climate()), 11.0, "second");
    }

    #[test]
    fn the_turn_is_a_quarter_circle_at_the_twilight_midpoints_and_a_half_at_solar_midnight() {
        let times = second_climate();
        for (hour, expected) in [
            (11.0, 0.0),
            (4.0, -FRAC_PI_2),
            (18.0, FRAC_PI_2),
            (7.5, -FRAC_PI_2 / 2.0),
            (20.5, FRAC_PI_2 * 1.5),
            (1.5, -FRAC_PI_2 * 1.5),
        ] {
            assert_close(turn_from_noon(hour, &times), expected, &format!("{hour}"));
        }
        assert_close(turn_from_noon(23.0, &times).abs(), PI, "solar midnight");
    }

    #[test]
    fn under_the_default_climate_the_turn_is_a_steady_24_hour_circle() {
        let times = default_climate();
        for step in 0..48 {
            let hour = step as f32 * 0.5;
            let steady = TAU * (hour - 13.0) / HOURS_PER_DAY;
            let turn = turn_from_noon(hour, &times);
            let gap = (turn - steady).rem_euclid(TAU);
            assert!(gap.min(TAU - gap) < 1e-4, "{hour}: {turn} vs {steady}");
        }
    }

    #[test]
    fn the_turn_never_jumps_except_through_solar_midnight() {
        let times = second_climate();
        for step in 0..2399 {
            let hour = step as f32 * 0.01;
            let gap = (turn_from_noon(hour + 0.01, &times) - turn_from_noon(hour, &times))
                .rem_euclid(TAU);
            assert!(gap.min(TAU - gap) < 0.01, "{hour}");
        }
    }
}
