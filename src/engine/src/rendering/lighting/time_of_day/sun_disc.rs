//! Where the sun disc stands, as the probe found it in the sky at 12:00 and 08:00. In Bethesda
//! axes (X east, Y north, Z up) the disc sits on a sphere of radius `|fSunXExtreme|` around the
//! viewer, at the constant north-south offset `fSunYExtreme`, and reaches `|fSunXExtreme| sin t`
//! east of it, where `t` turns a quarter circle from the sunrise begin to solar noon and another
//! from solar noon to the sunset end. At those two ends the disc stands due east and due west of
//! the sphere's centre line, below the horizon; it rises once the sphere clears the horizon,
//! about 5.1 hours before solar noon in the default climate (07:55), and sets as long after.
//!
//! When the disc becomes visible was not measured: `sun_alpha` ramps around the twilight
//! midpoints (07:00, 19:00), so it is already 1 when the disc clears the horizon and the horizon
//! fade alone hides it before then.
//!
//! Below the horizon nothing was measured and nothing shows: there `t` turns through the other
//! half circle over the night, and the disc sinks in a straight line in the sphere's height,
//! which keeps the path continuous.

use std::f32::consts::{FRAC_PI_2, PI};

use glam::Vec3;

use super::climate_times::ClimateTimes;
use super::day_time::DayTime;
use super::sky::{SkyGameSettings, SkyPosition};
use super::sun_light::solar_noon;
use super::wrap::{HOURS_PER_DAY, wrap_hours};

/// The hours from solar noon to `hour`, within half a day either way.
fn hours_from_noon(hour: f32, times: &ClimateTimes) -> f32 {
    let half_day = HOURS_PER_DAY / 2.0;
    wrap_hours(hour - solar_noon(times) + half_day) - half_day
}

/// The turn from solar noon: a quarter circle from the sunrise begin to solar noon and another
/// from solar noon to the sunset end, the other half circle over the night.
fn turn_from_noon(hour: f32, times: &ClimateTimes) -> f32 {
    let offset = hours_from_noon(hour, times);
    let noon = solar_noon(times);
    let half_day = if offset < 0.0 {
        noon - times.sunrise_begin()
    } else {
        times.sunset_end() - noon
    };
    let turn = if offset.abs() <= half_day {
        FRAC_PI_2 * offset.abs() / half_day
    } else {
        let half_night = HOURS_PER_DAY / 2.0 - half_day;
        FRAC_PI_2 + FRAC_PI_2 * (offset.abs() - half_day) / half_night
    };
    turn.copysign(offset).clamp(-PI, PI)
}

pub fn sun_position(
    time: DayTime,
    times: &ClimateTimes,
    settings: &SkyGameSettings,
) -> SkyPosition {
    let radius = settings.sun_x_extreme().abs();
    let north = settings.sun_y_extreme();
    let turn = turn_from_noon(time.hour(), times);
    let reach = radius * turn.cos();
    let height = if reach > north.abs() {
        (reach * reach - north * north).sqrt()
    } else {
        reach - north.abs()
    };
    SkyPosition::from_bethesda_direction(Vec3::new(-radius * turn.sin(), north, height))
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{at, at_hour, default_climate, vanilla_settings};
    use super::*;

    fn disc(hour: f32) -> SkyPosition {
        sun_position(at_hour(hour), &default_climate(), &vanilla_settings())
    }

    fn degrees_apart(a: SkyPosition, b: SkyPosition) -> f32 {
        a.direction().angle_between(b.direction()).to_degrees()
    }

    #[test]
    fn the_disc_stands_within_four_degrees_of_where_the_probe_saw_it() {
        for (hour, elevation_deg, azimuth_deg) in [(12.0, 55.9, 163.2), (8.0, 10.2, 123.5)] {
            let measured = SkyPosition {
                azimuth_deg,
                elevation_deg,
            };
            let gap = degrees_apart(disc(hour), measured);
            assert!(gap < 4.0, "{hour}: {gap} degrees from {measured:?}");
        }
    }

    #[test]
    fn at_solar_noon_the_disc_is_due_south_on_the_sphere_of_the_extremes() {
        let noon = disc(13.0);
        assert!((noon.azimuth_deg - 180.0).abs() < 1e-3, "{noon:?}");
        let expected = (600.0_f32 * 600.0 - 325.0 * 325.0).sqrt().atan2(325.0);
        assert!((noon.elevation_deg - expected.to_degrees()).abs() < 1e-3);
    }

    #[test]
    fn the_disc_is_symmetric_about_solar_noon() {
        for offset in [0.5, 3.0, 5.0, 8.0, 10.5] {
            let before = disc(13.0 - offset);
            let after = disc(13.0 + offset);
            assert!((before.elevation_deg - after.elevation_deg).abs() < 1e-3);
            assert!((before.azimuth_deg + after.azimuth_deg - 360.0).abs() < 1e-2);
        }
    }

    #[test]
    fn the_disc_rises_at_07_55_and_sets_at_18_05_in_the_default_climate() {
        // The sphere clears the horizon at acos(325 / 600) = 57.2 degrees of turn, 57.2 / 90 * 8 h
        // = 5.08 h from solar noon: 07:55 and 18:05.
        for hour in [0.0, 4.0, 7.5, 7.88, 18.12, 18.5, 22.0] {
            assert!(disc(hour).elevation_deg < 0.0, "{hour}");
        }
        for hour in [7.95, 8.0, 10.0, 13.0, 16.0, 18.0, 18.05] {
            assert!(disc(hour).elevation_deg > 0.0, "{hour}");
        }
    }

    #[test]
    fn the_disc_rises_in_the_east_and_sets_in_the_west() {
        assert!((90.0..180.0).contains(&disc(8.0).azimuth_deg));
        assert!((180.0..270.0).contains(&disc(18.0).azimuth_deg));
    }

    #[test]
    fn the_disc_never_jumps_through_the_whole_day_or_over_midnight() {
        let times = default_climate();
        let settings = vanilla_settings();
        let mut previous = sun_position(at(0, 0.0), &times, &settings);
        for step in 1..=4800_u32 {
            let current = sun_position(
                at(step / 2400, (step % 2400) as f32 * 0.01),
                &times,
                &settings,
            );
            let moved = degrees_apart(previous, current);
            assert!(moved < 2.0, "step {step}: {moved} degrees");
            previous = current;
        }
    }

    #[test]
    fn the_disc_follows_an_asymmetric_climate_window() {
        // Sunrise 05:30-08:00, sunset 16:00-20:30: solar noon 12:30, a 7 h morning and an 8 h
        // afternoon. Expected places worked out by hand from the rule.
        let times = ClimateTimes::new(5.5, 8.0, 16.0, 20.5).unwrap();
        let settings = vanilla_settings();
        for (hour, elevation_deg, azimuth_deg) in [
            (8.0, -0.549, 122.608),
            (10.0, 40.602, 135.514),
            (12.5, 57.203, 180.0),
            (15.0, 44.105, 221.032),
            (17.0, 19.283, 234.980),
        ] {
            let place = sun_position(at_hour(hour), &times, &settings);
            assert!(
                (place.elevation_deg - elevation_deg).abs() < 0.01,
                "{hour}: {place:?}"
            );
            assert!(
                (place.azimuth_deg - azimuth_deg).abs() < 0.01,
                "{hour}: {place:?}"
            );
        }
    }

    #[test]
    fn at_the_sunrise_begin_and_the_sunset_end_the_disc_is_a_full_quarter_turn_from_noon() {
        // A quarter turn puts the disc 600 east or west and 325 south of the viewer with its
        // height at 0 - 325 on the line below the horizon: elevation -25.468, azimuth 118.443 / 241.557.
        let times = ClimateTimes::new(5.5, 8.0, 16.0, 20.5).unwrap();
        let settings = vanilla_settings();
        for (hour, azimuth_deg) in [(5.5, 118.443), (20.5, 241.557)] {
            let place = sun_position(at_hour(hour), &times, &settings);
            assert!(
                (place.elevation_deg + 25.468).abs() < 0.01,
                "{hour}: {place:?}"
            );
            assert!(
                (place.azimuth_deg - azimuth_deg).abs() < 0.01,
                "{hour}: {place:?}"
            );
        }
    }
}
