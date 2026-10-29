//! How far the sky has turned from solar noon, which times both the directional light and the
//! sun disc. Measured under two climates (sunrise 05-09 / sunset 17-21 and 03-05 / 15-21): solar
//! noon is halfway between the twilight midpoints, the sky turns a quarter circle from the sunrise
//! midpoint to solar noon and another from solar noon to the sunset midpoint, and the other half
//! circle over the night between the sunset midpoint and the next sunrise midpoint.

use std::f32::consts::{FRAC_PI_2, PI};

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
    turn_with_longer_day_legs(hour, times, 0.0)
}

/// The hour of the day whose turn the night mirrors, as the light's `|cos a|` does: the hour
/// itself by day; at night the day hour as far from solar noon as the night hour's turn falls
/// short of a half circle, on the same side of noon.
pub fn mirrored_day_hour(hour: f32, times: &ClimateTimes) -> f32 {
    let turn = turn_from_noon(hour, times);
    let day_turn = if turn.abs() <= FRAC_PI_2 {
        turn
    } else {
        (PI - turn.abs()).copysign(turn)
    };
    let day_leg = (times.sunset_midpoint() - times.sunrise_midpoint()) / 2.0;
    wrap_hours(solar_noon(times) + day_turn / FRAC_PI_2 * day_leg)
}

/// [`turn_from_noon`] with each day quarter `extension` hours longer, at the night's expense: the
/// quarter circles end `extension` hours before the sunrise midpoint and after the sunset one.
pub fn turn_with_longer_day_legs(hour: f32, times: &ClimateTimes, extension: f32) -> f32 {
    let half_day = HOURS_PER_DAY / 2.0;
    let offset = wrap_hours(hour - solar_noon(times) + half_day) - half_day;
    let day_leg = ((times.sunset_midpoint() - times.sunrise_midpoint()) / 2.0 + extension)
        .clamp(f32::EPSILON, half_day);
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
    fn longer_day_legs_move_the_quarter_turns_outwards_and_keep_noon_and_midnight() {
        let times = second_climate();
        for (hour, expected) in [
            (11.0, 0.0),
            (3.9, -FRAC_PI_2),
            (18.1, FRAC_PI_2),
            (7.45, -FRAC_PI_2 / 2.0),
        ] {
            assert_close(
                turn_with_longer_day_legs(hour, &times, 0.1),
                expected,
                &format!("{hour}"),
            );
        }
        assert_close(
            turn_with_longer_day_legs(23.0, &times, 0.1).abs(),
            PI,
            "solar midnight",
        );
    }

    #[test]
    fn the_night_mirrors_the_day_hours_the_light_repeats() {
        let times = default_climate();
        for (hour, expected) in [
            (0.0, 14.0),
            (3.0, 11.0),
            (21.0, 17.0),
            (23.0, 15.0),
            (10.0, 10.0),
            (19.0, 19.0),
        ] {
            assert_close(
                mirrored_day_hour(hour, &times),
                expected,
                &format!("{hour}"),
            );
        }
        let second = second_climate();
        assert_close(
            mirrored_day_hour(23.0, &second),
            11.0,
            "solar midnight mirrors solar noon",
        );
    }

    #[test]
    fn day_legs_cannot_grow_past_half_a_day() {
        let times = second_climate();
        let turn = turn_with_longer_day_legs(23.0, &times, 30.0);
        assert!(turn.is_finite(), "{turn}");
        assert_close(turn.abs(), FRAC_PI_2, "half a day per leg leaves no night");
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
