//! Where the sun disc stands, as the probe found it in the sky under two climates (`DefaultClimate`
//! at 08:00 and 12:00; sunrise 03-05 / sunset 15-21 every hour from 05:00 to 18:00). In Bethesda
//! axes (X east, Y north, Z up) the disc runs in straight lines around the viewer at the constant
//! north-south offset `fSunYExtreme`: from `|fSunXExtreme|` east on the horizon at the sunrise
//! midpoint to `|fSunXExtreme|` overhead at solar noon, and on to `|fSunXExtreme|` west on the
//! horizon at the sunset midpoint, moving evenly in time over each leg ([`turn_from_noon`]). With
//! the vanilla settings it meets the horizon at azimuth 118.4° and 241.6°, south of east and west.
//!
//! When the disc becomes visible was not measured: `sun_alpha` ramps around the twilight
//! midpoints, where the disc meets the horizon, and the horizon fade hides it below.
//!
//! Below the horizon nothing was measured and nothing shows: over the night the disc runs the
//! mirrored lines under the horizon, from where it sets through `|fSunXExtreme|` straight down at
//! solar midnight to where it rises, which keeps the path continuous.

use std::f32::consts::FRAC_PI_2;

use glam::Vec3;

use super::climate_times::ClimateTimes;
use super::day_time::DayTime;
use super::sky::{SkyGameSettings, SkyPosition};
use super::solar_turn::turn_from_noon;

/// The disc's reach east and up, as shares of `|fSunXExtreme|`, `turn` radians from solar noon.
fn reach_east_and_up(turn: f32) -> (f32, f32) {
    let legs = turn.abs() / FRAC_PI_2;
    if legs <= 1.0 {
        (-legs.copysign(turn), 1.0 - legs)
    } else {
        let below = legs - 1.0;
        (-(1.0 - below).copysign(turn), -below)
    }
}

pub fn sun_position(
    time: DayTime,
    times: &ClimateTimes,
    settings: &SkyGameSettings,
) -> SkyPosition {
    let radius = settings.sun_x_extreme().abs();
    let (east, up) = reach_east_and_up(turn_from_noon(time.hour(), times));
    SkyPosition::from_bethesda_direction(Vec3::new(
        radius * east,
        settings.sun_y_extreme(),
        radius * up,
    ))
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{at, at_hour, default_climate, vanilla_settings};
    use super::*;

    fn second_climate() -> ClimateTimes {
        ClimateTimes::new(3.0, 5.0, 15.0, 21.0).unwrap()
    }

    fn disc(hour: f32) -> SkyPosition {
        sun_position(at_hour(hour), &default_climate(), &vanilla_settings())
    }

    fn degrees_apart(a: SkyPosition, b: SkyPosition) -> f32 {
        a.direction().angle_between(b.direction()).to_degrees()
    }

    /// The discs the probe found (`results_v13`): `DefaultClimate` at 08:00 and 12:00, then the
    /// second climate (sunrise 03-05, sunset 15-21) every hour from 05:00 to 18:00.
    #[test]
    fn the_disc_stands_within_a_degree_of_every_disc_the_probe_saw() {
        let default = default_climate();
        let second = second_climate();
        let measured = [
            (&default, 8.0, 10.2, 123.5),
            (&default, 12.0, 55.9, 163.2),
            (&second, 5.0, 8.6, 122.7),
            (&second, 6.0, 18.3, 127.6),
            (&second, 7.0, 29.2, 134.0),
            (&second, 8.0, 40.0, 142.0),
            (&second, 9.0, 49.6, 152.6),
            (&second, 10.0, 56.9, 165.5),
            (&second, 11.0, 61.5, 180.2),
            (&second, 12.0, 56.8, 194.8),
            (&second, 13.0, 49.5, 207.7),
            (&second, 14.0, 39.8, 218.2),
            (&second, 15.0, 29.0, 226.1),
            (&second, 16.0, 18.1, 232.5),
            (&second, 17.0, 8.5, 237.5),
            (&second, 18.0, 0.7, 241.3),
        ];
        for (times, hour, elevation_deg, azimuth_deg) in measured {
            let place = sun_position(at_hour(hour), times, &vanilla_settings());
            let measured = SkyPosition {
                azimuth_deg,
                elevation_deg,
            };
            let gap = degrees_apart(place, measured);
            assert!(
                gap < 1.0,
                "{times:?} {hour}: {gap} degrees from {measured:?}"
            );
        }
    }

    #[test]
    fn at_solar_noon_the_disc_is_due_south_as_high_as_the_x_extreme() {
        let noon = disc(13.0);
        assert!((noon.azimuth_deg - 180.0).abs() < 1e-3, "{noon:?}");
        let expected = 600.0_f32.atan2(325.0).to_degrees();
        assert!((noon.elevation_deg - expected).abs() < 1e-3, "{noon:?}");
    }

    #[test]
    fn the_disc_meets_the_horizon_south_of_east_and_west_at_the_twilight_midpoints() {
        let times = second_climate();
        let settings = vanilla_settings();
        let east_of_south = 600.0_f32.atan2(325.0).to_degrees();
        for (hour, azimuth_deg) in [(4.0, 180.0 - east_of_south), (18.0, 180.0 + east_of_south)] {
            let place = sun_position(at_hour(hour), &times, &settings);
            assert!(place.elevation_deg.abs() < 1e-3, "{hour}: {place:?}");
            assert!(
                (place.azimuth_deg - azimuth_deg).abs() < 1e-3,
                "{hour}: {place:?}"
            );
        }
    }

    #[test]
    fn the_disc_is_up_exactly_between_the_twilight_midpoints() {
        for hour in [0.0, 4.0, 6.99, 19.01, 22.0, 23.99] {
            assert!(disc(hour).elevation_deg < 1e-3, "{hour}");
        }
        for hour in [7.01, 8.0, 10.0, 13.0, 16.0, 18.99] {
            assert!(disc(hour).elevation_deg > 0.0, "{hour}");
        }
    }

    #[test]
    fn the_disc_is_symmetric_about_solar_noon() {
        let times = second_climate();
        let settings = vanilla_settings();
        for offset in [0.5, 3.0, 5.0, 8.0, 10.5] {
            let before = sun_position(at_hour(11.0 - offset), &times, &settings);
            let after = sun_position(at_hour(11.0 + offset), &times, &settings);
            assert!((before.elevation_deg - after.elevation_deg).abs() < 1e-3);
            assert!((before.azimuth_deg + after.azimuth_deg - 360.0).abs() < 1e-2);
        }
    }

    #[test]
    fn the_disc_rises_in_the_east_and_sets_in_the_west() {
        assert!((90.0..180.0).contains(&disc(8.0).azimuth_deg));
        assert!((180.0..270.0).contains(&disc(18.0).azimuth_deg));
    }

    #[test]
    fn at_solar_midnight_the_disc_is_straight_below_the_y_extreme() {
        let midnight = disc(1.0);
        let expected = -(600.0_f32.atan2(325.0).to_degrees());
        assert!(
            (midnight.elevation_deg - expected).abs() < 1e-3,
            "{midnight:?}"
        );
    }

    #[test]
    fn the_disc_never_jumps_through_the_whole_day_or_over_midnight() {
        let settings = vanilla_settings();
        for times in [default_climate(), second_climate()] {
            let mut previous = sun_position(at(0, 0.0), &times, &settings);
            for step in 1..=4800_u32 {
                let current = sun_position(
                    at(step / 2400, (step % 2400) as f32 * 0.01),
                    &times,
                    &settings,
                );
                let moved = degrees_apart(previous, current);
                assert!(moved < 2.0, "{times:?} step {step}: {moved} degrees");
                previous = current;
            }
        }
    }
}
