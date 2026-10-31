//! Where the sun's shadows are cast from, as the probe measured it with an upright caster over a
//! flat board (31 hours under two climates, day and night). It is neither the light's direction
//! nor the disc's:
//!
//! - its azimuth is the disc's at every hour with the disc up (within 0.4 degrees);
//! - its elevation is the disc's unit direction with the up part raised to at least
//!   [`MIN_RISE`] and otherwise scaled by [`RISE_SHARE`]: 52 degrees at solar noon, about 28
//!   where the disc meets the horizon;
//! - at night it repeats the day hour the light's turn mirrors ([`mirrored_day_hour`]), so
//!   shadows fall all night, as the light does.
//!
//! Over all 26 hours whose shadow fits the board the rule is 0.35 degrees rms off the measured
//! direction (worst 0.6); the light's own direction is 9 degrees rms off.

use glam::Vec3;

use super::climate_times::ClimateTimes;
use super::day_time::DayTime;
use super::sky::{SkyGameSettings, SkyPosition};
use super::solar_turn::mirrored_day_hour;
use super::sun_disc::disc_vector;

/// The least up part of the shadow direction over the disc's unit horizontal part (fitted).
pub const MIN_RISE: f32 = 0.524;
/// The share of the disc's up part the shadow direction keeps above [`MIN_RISE`] (fitted).
pub const RISE_SHARE: f32 = 0.688;

pub fn shadow_position(
    time: DayTime,
    times: &ClimateTimes,
    settings: &SkyGameSettings,
) -> SkyPosition {
    let disc = disc_vector(mirrored_day_hour(time.hour(), times), times, settings).normalize();
    SkyPosition::from_bethesda_direction(Vec3::new(
        disc.x,
        disc.y,
        (RISE_SHARE * disc.z).max(MIN_RISE),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::sun_disc::sun_position;
    use super::super::test_support::{at_hour, default_climate, vanilla_settings};
    use super::*;

    fn shadow(hour: f32) -> SkyPosition {
        shadow_position(at_hour(hour), &default_climate(), &vanilla_settings())
    }

    fn assert_close(actual: f32, expected: f32, tolerance: f32, what: &str) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{what}: expected {expected}, got {actual}"
        );
    }

    #[test]
    fn by_day_the_shadows_come_from_the_discs_azimuth() {
        for hour in [8.0, 10.0, 12.0, 15.0, 18.0] {
            let disc = sun_position(at_hour(hour), &default_climate(), &vanilla_settings());
            assert_close(
                shadow(hour).azimuth_deg,
                disc.azimuth_deg,
                1e-3,
                &format!("{hour}"),
            );
        }
    }

    #[test]
    fn the_shadow_elevation_matches_the_probe() {
        for (hour, azimuth, elevation) in [
            (13.0, 180.1, 52.1),
            (12.0, 163.4, 45.3),
            (10.0, 137.6, 32.5),
            (9.0, 129.4, 29.5),
            (16.0, 222.6, 32.4),
        ] {
            let place = shadow(hour);
            assert_close(place.azimuth_deg, azimuth, 0.6, &format!("{hour} azimuth"));
            assert_close(
                place.elevation_deg,
                elevation,
                0.6,
                &format!("{hour} elevation"),
            );
        }
    }

    #[test]
    fn at_night_the_shadows_repeat_the_mirrored_day_hour() {
        for (night, day) in [(0.0, 14.0), (3.0, 11.0), (21.0, 17.0), (23.0, 15.0)] {
            assert_eq!(shadow(night), shadow(day), "{night} mirrors {day}");
        }
        let midnight = shadow(0.0);
        assert_close(midnight.azimuth_deg, 197.7, 0.8, "00:00 azimuth");
        assert_close(midnight.elevation_deg, 44.9, 0.8, "00:00 elevation");
    }

    #[test]
    fn where_the_disc_meets_the_horizon_the_shadows_stay_raised() {
        assert_close(
            shadow(7.0).elevation_deg,
            MIN_RISE.atan().to_degrees(),
            0.1,
            "07:00",
        );
    }
}
