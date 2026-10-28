//! The exterior's one directional light over the day, as the probe measured it on its light
//! boards. It is not the sun disc: it stays 26 to 37 degrees up all day and all night, swings
//! between the south-east and the south-west, and at night runs back along its day path.
//!
//! In Bethesda axes (X east, Y north, Z up) the light vector is
//! `(EAST_REACH sin a, -(SOUTH_BASE + SOUTH_SWING |cos a|), UP_BASE + UP_SWING |cos a|)` with
//! `a = 2 pi (solar noon - hour) / 24`. Its direction points towards the light, and its length,
//! 1 at solar noon and at solar midnight, scales the weather's sunlight colour.

use std::f32::consts::TAU;

use glam::Vec3;

use super::climate_times::ClimateTimes;
use super::day_time::DayTime;
use super::light::LightSource;
use super::sky::SkyPosition;
use super::wrap::HOURS_PER_DAY;

const EAST_REACH: f32 = 0.547;
const SOUTH_BASE: f32 = 0.243;
const SOUTH_SWING: f32 = 0.558;
const UP_BASE: f32 = 0.298;
const UP_SWING: f32 = 0.301;

/// The hour the light stands due south: halfway between the sunrise and sunset midpoints.
pub fn solar_noon(times: &ClimateTimes) -> f32 {
    f32::midpoint(times.sunrise_midpoint(), times.sunset_midpoint())
}

fn light_vector(hour: f32, times: &ClimateTimes) -> Vec3 {
    let angle = TAU * (solar_noon(times) - hour) / HOURS_PER_DAY;
    let swing = angle.cos().abs();
    Vec3::new(
        EAST_REACH * angle.sin(),
        -(SOUTH_BASE + SOUTH_SWING * swing),
        UP_BASE + UP_SWING * swing,
    )
}

pub fn light_source(time: DayTime, times: &ClimateTimes) -> LightSource {
    let vector = light_vector(time.hour(), times);
    let noon_length = (SOUTH_BASE + SOUTH_SWING).hypot(UP_BASE + UP_SWING);
    LightSource {
        direction: SkyPosition::from_bethesda_direction(vector).direction(),
        strength: vector.length() / noon_length,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{at, at_hour, default_climate};
    use super::*;

    fn light(hour: f32) -> LightSource {
        light_source(at_hour(hour), &default_climate())
    }

    fn position(light: LightSource) -> SkyPosition {
        let d = light.direction;
        SkyPosition {
            azimuth_deg: d.x.atan2(-d.z).to_degrees().rem_euclid(360.0),
            elevation_deg: d.y.asin().to_degrees(),
        }
    }

    fn assert_close(actual: f32, expected: f32, tolerance: f32, what: &str) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{what}: expected {expected}, got {actual}"
        );
    }

    #[test]
    fn solar_noon_is_halfway_between_the_twilight_midpoints() {
        assert_close(solar_noon(&default_climate()), 13.0, 1e-6, "default");
        let late = ClimateTimes::new(5.5, 8.0, 16.0, 20.5).unwrap();
        assert_close(solar_noon(&late), 12.5, 1e-6, "other climate");
    }

    #[test]
    fn at_solar_noon_and_midnight_the_light_is_due_south_at_full_strength() {
        for hour in [13.0, 1.0] {
            let light = light(hour);
            let place = position(light);
            assert_close(place.azimuth_deg, 180.0, 1e-3, "azimuth");
            assert_close(place.elevation_deg, 36.8, 0.1, "elevation");
            assert_close(light.strength, 1.0, 1e-3, "strength");
        }
    }

    #[test]
    fn the_light_matches_the_measured_hours() {
        for (hour, elevation, azimuth, strength) in [
            (6.5, 28.2, 120.2, 0.712),
            (12.0, 36.5, 169.7, 0.989),
            (18.5, 28.2, 239.8, 0.712),
            (22.0, 34.4, 211.2, 0.904),
        ] {
            let light = light(hour);
            let place = position(light);
            assert_close(place.elevation_deg, elevation, 0.1, "elevation");
            assert_close(place.azimuth_deg, azimuth, 0.1, "azimuth");
            assert_close(light.strength, strength, 1e-3, "strength");
        }
    }

    #[test]
    fn the_light_is_east_at_the_sunrise_midpoint_and_west_at_the_sunset_midpoint() {
        let east = position(light(7.0));
        let west = position(light(19.0));
        assert_close(east.azimuth_deg, 114.0, 0.1, "east");
        assert_close(west.azimuth_deg, 246.0, 0.1, "west");
        assert_close(east.elevation_deg, west.elevation_deg, 1e-3, "same height");
    }

    #[test]
    fn at_night_the_light_retraces_its_day_path_mirrored_about_the_twilight_midpoints() {
        for offset in [0.5, 2.0, 3.0, 5.5] {
            let night = light(7.0 - offset);
            let day = light(7.0 + offset);
            assert!(night.direction.abs_diff_eq(day.direction, 1e-5), "{offset}");
            assert_close(night.strength, day.strength, 1e-5, "strength");
        }
    }

    #[test]
    fn the_light_never_sets_and_never_leaves_its_band() {
        for step in 0..240 {
            let light = light(step as f32 * 0.1);
            let place = position(light);
            assert!(
                (26.0..37.0).contains(&place.elevation_deg),
                "{step}: {place:?}"
            );
            assert!((0.66..=1.0 + 1e-6).contains(&light.strength), "{step}");
            assert_close(light.direction.length(), 1.0, 1e-5, "unit");
        }
    }

    #[test]
    fn the_light_is_continuous_across_midnight_and_the_same_every_day() {
        let times = default_climate();
        let before = light_source(at(3, 23.9999), &times);
        let after = light_source(at(4, 0.0), &times);
        assert!(before.direction.abs_diff_eq(after.direction, 1e-4));
        assert_eq!(
            light_source(at(0, 9.25), &times),
            light_source(at(40, 9.25), &times)
        );
    }

    #[test]
    fn the_climate_moves_solar_noon() {
        let times = ClimateTimes::new(5.5, 8.0, 16.0, 20.5).unwrap();
        let light = light_source(at_hour(12.5), &times);
        assert_close(position(light).azimuth_deg, 180.0, 1e-3, "azimuth");
    }
}
