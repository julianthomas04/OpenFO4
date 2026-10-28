//! Where the sun disc stands, as the probe found it in the sky under two climates (`DefaultClimate`
//! at 08:00 and 12:00; sunrise 03-05 / sunset 15-21 every hour from 05:00 to 18:00). In Bethesda
//! axes (X east, Y north, Z up) the disc runs in straight lines around the viewer at the constant
//! north-south offset `fSunYExtreme`: from `|fSunXExtreme|` east on the horizon to `|fSunXExtreme|`
//! overhead at solar noon, and on to `|fSunXExtreme|` west on the horizon, moving evenly in time
//! over each leg. Each leg runs [`DISC_LEG_EXTENSION_HOURS`] longer than the twilight midpoint's
//! distance from solar noon, so the disc's centre stands about half a degree up at the midpoints.
//! With the vanilla settings it meets the horizon at azimuth 118.4° and 241.6°, south of east and
//! west.
//!
//! At the second climate's sunset midpoint the probe saw the disc at 0.45 of its full light, as
//! `sun_alpha`'s ramp centred there gives at the captures' lag (0.44), and uncut down to the
//! frame's edge 0.21 degrees below the horizon. The horizon fade is kept below that.
//!
//! Below the horizon nothing was measured and nothing shows: over the night the disc runs the
//! mirrored lines under the horizon, from where it sets through `|fSunXExtreme|` straight down at
//! solar midnight to where it rises, which keeps the path continuous.

use std::f32::consts::FRAC_PI_2;

use glam::Vec3;

use super::climate_times::ClimateTimes;
use super::day_time::DayTime;
use super::sky::{SkyGameSettings, SkyPosition};
use super::solar_turn::turn_with_longer_day_legs;

/// How many hours longer than the twilight midpoint's distance from solar noon each day leg of
/// the disc runs: fitted to the 14 discs the probe saw whole under two climates (0.0725 h under
/// each alone), it takes the angle rms from 0.44 to 0.11 degrees, and to 0.057 together with the
/// captures' 0.009 h lag. Half `fSunAlphaTransTime` (0.075 h) fits as well; whether the extension
/// is that setting's, a share of the leg or a constant is open.
pub const DISC_LEG_EXTENSION_HOURS: f32 = 0.07;

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
    let (east, up) = reach_east_and_up(turn_with_longer_day_legs(
        time.hour(),
        times,
        DISC_LEG_EXTENSION_HOURS,
    ));
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

    /// The discs the probe saw whole, away from the frame's border (`results_v14` rerun, item 11:
    /// "Where does the sun disc meet the horizon, and how does it fade there?").
    #[test]
    fn the_disc_stands_within_a_fifth_of_a_degree_of_every_whole_disc_the_probe_saw() {
        let default = default_climate();
        let second = second_climate();
        let measured = [
            (&default, 8.0, 10.19, 123.46),
            (&default, 12.0, 55.91, 163.25),
            (&default, 12.0, 55.88, 163.27),
            (&second, 5.0, 8.59, 122.67),
            (&second, 6.0, 18.28, 127.61),
            (&second, 8.0, 39.98, 142.01),
            (&second, 9.0, 49.61, 152.55),
            (&second, 10.0, 56.92, 165.52),
            (&second, 11.0, 61.53, 180.16),
            (&second, 12.0, 56.83, 194.80),
            (&second, 13.0, 49.46, 207.72),
            (&second, 14.0, 39.81, 218.21),
            (&second, 16.0, 18.14, 232.55),
            (&second, 17.0, 8.48, 237.45),
        ];
        for (times, hour, elevation_deg, azimuth_deg) in measured {
            let place = sun_position(at_hour(hour), times, &vanilla_settings());
            let measured = SkyPosition {
                azimuth_deg,
                elevation_deg,
            };
            let gap = degrees_apart(place, measured);
            assert!(
                gap < 0.2,
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
    fn the_disc_meets_the_horizon_south_of_east_and_west_just_outside_the_twilight_midpoints() {
        let times = second_climate();
        let settings = vanilla_settings();
        let east_of_south = 600.0_f32.atan2(325.0).to_degrees();
        for (hour, azimuth_deg) in [
            (4.0 - DISC_LEG_EXTENSION_HOURS, 180.0 - east_of_south),
            (18.0 + DISC_LEG_EXTENSION_HOURS, 180.0 + east_of_south),
        ] {
            let place = sun_position(at_hour(hour), &times, &settings);
            assert!(place.elevation_deg.abs() < 1e-3, "{hour}: {place:?}");
            assert!(
                (place.azimuth_deg - azimuth_deg).abs() < 1e-3,
                "{hour}: {place:?}"
            );
        }
    }

    #[test]
    fn the_disc_is_up_from_just_before_the_sunrise_midpoint_to_just_after_the_sunset_midpoint() {
        for hour in [0.0, 4.0, 6.92, 19.08, 22.0, 23.99] {
            assert!(disc(hour).elevation_deg < 1e-3, "{hour}");
        }
        for hour in [6.94, 7.0, 8.0, 10.0, 13.0, 16.0, 19.0, 19.06] {
            assert!(disc(hour).elevation_deg > 0.0, "{hour}");
        }
    }

    #[test]
    fn at_the_twilight_midpoints_the_disc_stands_about_half_a_degree_up() {
        for hour in [7.0, 19.0] {
            let elevation = disc(hour).elevation_deg;
            assert!((0.4..0.8).contains(&elevation), "{hour}: {elevation}");
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
