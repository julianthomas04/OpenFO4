//! The rules no game data or public source pins down. Each is a hypothesis to be
//! replaced by in-game measurement; nothing outside this module encodes them.
//!
//! - Moon path: a circle through due east and due west whose plane tilts so its highest point
//!   stands due south at `atan2(|fSunYExtreme|, |fSunXExtreme|)` elevation. On day 0 the moon
//!   is in [`MOON_PHASE_AT_DAY_ZERO`], and its distance behind a body that rises on that circle
//!   at the sunrise midpoint and sets at the sunset midpoint is that phase's share of the cycle,
//!   growing by one cycle in `8 * phase_length` days (a full moon rises at the sunset midpoint).
//! - Stars: one revolution per `fStarsRotateDays`, in the same sense as the sun, starting
//!   at [`STAR_ANGLE_AT_DAY_ZERO_RAD`]; their opacity is the luminance of the weather Stars colour, taken as linear.
//! - Moon size: [`MOON_SIZE_UNITS_PER_DEGREE`] size units make one degree of the moon's apparent diameter.
//! - Star horizon fade: the stars fade in over the first [`STAR_HORIZON_FADE_SINE`] of the sine of elevation.
//! - Moon brightness: the moon texture is shown at [`MOON_RADIANCE`] times the sky radiance
//!   scale.
//! - Manual sun: with the lighting override on, the panel's sun lights the ground over
//!   [`SUN_LIGHT_FADE_DEG`] of elevation.
//! - Sun alpha: `fSunAlphaTransTime` is a number of hours; the sun's alpha ramps linearly over
//!   that long, centred on the midpoint of each twilight window (see [`sun_alpha`]).
//! - Sun glare size: `fSunGlareSize` (glare sprite) is in the moon's unit,
//!   [`SUN_GLARE_SIZE_UNITS_PER_DEGREE`] per degree of the sprite's quad edge.
//! - Sun glare brightness: the glare sprite shows at [`SUN_GLARE_RADIANCE`] times the weather's
//!   `SunGlare` colour and the sky radiance scale. The probe saw no glare at all with the Sun
//!   colour black and `SunGlare` grey 96 or 192. Both sun sprites are cut at the horizon over
//!   [`SUN_HORIZON_FADE_SINE`], and both take the sky radiance scale. `fSunMinimumGlareScale` and
//!   the weather's own glare byte (not exported yet) are not applied.
//! - Clouds: a layer scrolls [`CLOUD_SCROLL_SCALE`] texture widths per second for each unit of
//!   its weather's layer speed, and blends into the sky's horizon colour over the first
//!   [`CLOUD_HORIZON_HAZE_SINE`] of the sine of elevation.

use std::f32::consts::PI;

use glam::Vec3;

use super::climate_times::ClimateTimes;
use super::day_time::DayTime;
use super::moon::{MoonError, MoonPhase, cycle_days};
use super::sky::{SkyGameSettings, SkyPosition, linear_fade};
use super::wrap::{HOURS_PER_DAY, wrap_hours};
use crate::rendering::lighting::cell_lighting::luminance;

/// Elevations (start, end) in degrees over which the manual sun's direct light fades in.
pub const SUN_LIGHT_FADE_DEG: (f32, f32) = (0.0, 5.0);

/// The moon phase on day 0, which also fixes where it stands relative to the sun then.
pub const MOON_PHASE_AT_DAY_ZERO: MoonPhase = MoonPhase::Full;

/// Sign of the star dome turn about its axis: -1 makes a star on the east horizon
/// climb towards the south, like the sun, for the right-handed turn `StarRotation` states.
pub const STAR_ROTATION_SENSE: f64 = -1.0;

/// The star dome turn at day 0, 00:00.
pub const STAR_ANGLE_AT_DAY_ZERO_RAD: f32 = 0.0;

/// Elevation of the moon path's highest point, from the horizontal reach and the southward offset
/// of the sun extremes.
fn noon_elevation_rad(settings: &SkyGameSettings) -> f32 {
    settings
        .sun_y_extreme()
        .abs()
        .atan2(settings.sun_x_extreme().abs())
}

/// Angle along the orbit circle: 0 at rising, pi at setting, back to 2 pi at the
/// next rising, with the body above the horizon for `up_hours` of every day.
fn orbit_angle(hour: f32, rise_hour: f32, up_hours: f32) -> f32 {
    let since_rise = wrap_hours(hour - rise_hour);
    if since_rise < up_hours {
        PI * since_rise / up_hours
    } else {
        PI + PI * (since_rise - up_hours) / (HOURS_PER_DAY - up_hours)
    }
}

/// East is `(1, 0, 0)` and the noon direction `(0, -cos e, sin e)` in Bethesda axes
/// (X east, Y north, Z up).
fn orbit_position(hour: f32, rise_hour: f32, up_hours: f32, noon_elevation: f32) -> SkyPosition {
    let angle = orbit_angle(hour, rise_hour, up_hours);
    SkyPosition::from_bethesda_direction(Vec3::new(
        angle.cos(),
        -angle.sin() * noon_elevation.cos(),
        angle.sin() * noon_elevation.sin(),
    ))
}

/// The moon lag behind the sun grows continuously with the clock, so its apparent
/// day is a little over 24 hours and the moon does not jump at midnight.
pub fn moon_position(
    time: DayTime,
    phase_length: u32,
    times: &ClimateTimes,
    settings: &SkyGameSettings,
) -> Result<SkyPosition, MoonError> {
    let cycle = cycle_days(phase_length)? as f64;
    let hours_per_day = f64::from(HOURS_PER_DAY);
    let sunset = f64::from(times.sunset_midpoint());
    let hours_since_first_sunset = time.elapsed_days() * hours_per_day - sunset;
    let day_zero_share = MOON_PHASE_AT_DAY_ZERO.index() as f64 / MoonPhase::ALL.len() as f64;
    let cycles_behind = hours_since_first_sunset / (hours_per_day * cycle) + day_zero_share;
    let lag_hours = (cycles_behind.rem_euclid(1.0) * hours_per_day) as f32;
    let up_hours = HOURS_PER_DAY - (times.sunset_midpoint() - times.sunrise_midpoint());
    Ok(orbit_position(
        time.hour(),
        times.sunset_midpoint() + lag_hours,
        up_hours,
        noon_elevation_rad(settings),
    ))
}

/// The star layer opacity from the weather linear Stars colour: its luminance, within 0 to 1.
pub fn star_alpha(stars_colour_linear: Vec3) -> f32 {
    luminance(stars_colour_linear).clamp(0.0, 1.0)
}

/// 0 while the sun is at or below the horizon, 1 from [`SUN_LIGHT_FADE_DEG`]'s end upwards.
pub fn sun_light_weight(elevation_deg: f32) -> f32 {
    let (fade_start, fade_end) = SUN_LIGHT_FADE_DEG;
    linear_fade(elevation_deg, fade_start, fade_end)
}

/// The sun's alpha at `hour`: 0 while it is down, ramping linearly to 1 over `trans_time` hours
/// centred on each twilight window's midpoint (sunrise and sunset), 1 in between. `trans_time`
/// is `fSunAlphaTransTime`; this reading of it is a hypothesis. It also drives the water's
/// daylight factor.
pub fn sun_alpha(hour: f32, times: &ClimateTimes, trans_time: f32) -> f32 {
    let rise = times.sunrise_midpoint();
    let set = times.sunset_midpoint();
    let half = trans_time * 0.5;
    if hour <= rise - half || hour >= set + half {
        0.0
    } else if hour < rise + half {
        (hour - (rise - half)) / trans_time
    } else if hour > set - half {
        ((set + half) - hour) / trans_time
    } else {
        1.0
    }
}

/// How many `iMasserSize` / `iSecundaSize` units make one degree of the moon's apparent
/// diameter. The quad edge has this diameter, so the visible disc is smaller by the share of
/// the texture the disc covers: calibrate against the textures, not the size alone. The
/// settings' unit is undocumented; 30 gives Masser (90) a 3 degree disc and
/// Secunda (75) 2.5 degrees, several times the real sun's half-degree disc.
pub const MOON_SIZE_UNITS_PER_DEGREE: f32 = 30.0;

/// The sine of the elevation at which the stars are fully faded in; they fade from 0 at the horizon.
pub const STAR_HORIZON_FADE_SINE: f32 = 0.05;

/// Radiance of a fully lit moon texel at sky radiance scale 1.
pub const MOON_RADIANCE: f32 = 4.0;

/// The apparent diameter in degrees of a moon whose size setting is `size`.
pub fn moon_angular_diameter_deg(size: f32) -> f32 {
    size / MOON_SIZE_UNITS_PER_DEGREE
}

/// How many `fSunGlareSize` units make one degree of the edge of the sun's glare sprite.
/// Hypothesis: the moon's unit, so the vanilla 600 gives a 20 degree quad.
pub const SUN_GLARE_SIZE_UNITS_PER_DEGREE: f32 = MOON_SIZE_UNITS_PER_DEGREE;

/// The sine of the elevation at which the sun sprites are fully faded in; they fade from 0 at the horizon.
pub const SUN_HORIZON_FADE_SINE: f32 = 0.02;

/// Radiance of a fully opaque, white sun glare texel at sky radiance scale 1.
pub const SUN_GLARE_RADIANCE: f32 = 2.0;

/// The edge in degrees of the glare sprite of a sun glare size `size`.
pub fn sun_glare_edge_deg(size: f32) -> f32 {
    size / SUN_GLARE_SIZE_UNITS_PER_DEGREE
}

/// How many texture widths a cloud layer scrolls per second for each unit of its weather's
/// layer speed, which is at most 0.1. No public source gives the speed a unit; the scale is a
/// heuristic. Taken as one, the user saw the clouds move too fast. At one half, the main
/// Commonwealth layer (speed 0.03, about three texture widths across the dome) scrolls 0.015
/// widths a second and crosses the sky in 200 seconds (about 3.3 minutes), the pace of real
/// clouds a kilometre up in a moderate wind.
pub const CLOUD_SCROLL_SCALE: f32 = 0.5;

/// The sine of the elevation at which the clouds are clear of the horizon haze. Below it they
/// blend into the sky's horizon colour, entirely at the horizon.
pub const CLOUD_HORIZON_HAZE_SINE: f32 = 0.15;

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact binary fractions of an hour"
)]
mod tests {
    use super::super::test_support::{at, default_climate, goodneighbor_climate, vanilla_settings};
    use super::*;

    const NOON_ELEVATION_DEG: f32 = 28.4;

    fn moon_position(
        hour: f32,
        days: u32,
        phase_length: u32,
        times: &ClimateTimes,
        settings: &SkyGameSettings,
    ) -> Result<SkyPosition, MoonError> {
        super::moon_position(at(days, hour), phase_length, times, settings)
    }

    fn assert_close(actual: f32, expected: f32, tolerance: f32, what: &str) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{what}: expected {expected}, got {actual}"
        );
    }

    fn angular_gap(a: f32, b: f32) -> f32 {
        let gap = (a - b).rem_euclid(360.0);
        gap.min(360.0 - gap)
    }

    fn assert_same_place(a: SkyPosition, b: SkyPosition, what: &str) {
        assert!(
            a.direction().angle_between(b.direction()).to_degrees() < 0.05,
            "{what}: {a:?} vs {b:?}"
        );
    }

    #[test]
    fn a_full_moon_rises_in_the_east_at_sunset_on_day_zero() {
        let rise = moon_position(19.0, 0, 4, &default_climate(), &vanilla_settings()).unwrap();
        assert_close(rise.elevation_deg, 0.0, 1e-3, "rise elevation");
        assert_close(rise.azimuth_deg, 90.0, 0.01, "rise azimuth");
    }

    #[test]
    fn the_moon_peaks_due_south_half_a_night_after_rising() {
        // The moon's day is 24 * 32 / 31 hours, so its peak is 6 * 32 / 31 hours after the first rise.
        let peak_hour = 19.0 + 6.0 * 32.0 / 31.0 - 24.0;
        let peak = moon_position(
            peak_hour as f32,
            1,
            4,
            &default_climate(),
            &vanilla_settings(),
        )
        .unwrap();
        assert_close(peak.azimuth_deg, 180.0, 0.1, "azimuth");
        assert_close(peak.elevation_deg, NOON_ELEVATION_DEG, 0.1, "elevation");
    }

    #[test]
    fn a_full_moon_is_up_through_the_night_and_down_through_the_day() {
        let times = default_climate();
        let settings = vanilla_settings();
        let elevation = |hour| {
            moon_position(hour, 0, 4, &times, &settings)
                .unwrap()
                .elevation_deg
        };
        for hour in [20.0, 22.0, 23.9, 0.0, 2.0, 4.0, 6.0] {
            assert!(elevation(hour) > 0.0, "hour {hour}");
        }
        for hour in [8.0, 10.0, 13.0, 16.0, 18.0] {
            assert!(elevation(hour) < 0.0, "hour {hour}");
        }
    }

    #[test]
    fn each_moonrise_is_a_little_over_a_day_after_the_last() {
        let times = default_climate();
        let settings = vanilla_settings();
        // pl 4 => 32 day cycle => moonrise k at 19 h + k * 24 * 32 / 31 h.
        for k in [0.0_f64, 1.0, 2.0, 7.0, 31.0] {
            let total_hours = 19.0 + k * 24.0 * 32.0 / 31.0;
            let days = (total_hours / 24.0).floor();
            let hour = (total_hours - days * 24.0) as f32;
            let rise = moon_position(hour, days as u32, 4, &times, &settings).unwrap();
            assert_close(rise.elevation_deg, 0.0, 0.05, "elevation at predicted rise");
            assert_close(
                angular_gap(rise.azimuth_deg, 90.0),
                0.0,
                0.5,
                "azimuth at rise",
            );
        }
    }

    #[test]
    fn half_a_cycle_after_the_first_sunset_the_moon_runs_where_a_body_rising_at_sunrise_would() {
        let times = default_climate();
        let settings = vanilla_settings();
        let day_body = |hour: f32| {
            orbit_position(
                hour,
                times.sunrise_midpoint(),
                times.sunset_midpoint() - times.sunrise_midpoint(),
                noon_elevation_rad(&settings),
            )
        };
        assert_same_place(
            moon_position(19.0, 16, 4, &times, &settings).unwrap(),
            day_body(19.0),
            "new moon at sunset",
        );
        for hour in [0.0, 7.0, 13.0, 22.5] {
            let moon = moon_position(hour, 16, 4, &times, &settings).unwrap();
            let sun = day_body(hour);
            let gap = moon.direction().angle_between(sun.direction()).to_degrees();
            assert!(gap < 10.0, "hour {hour}: {gap} degrees apart");
        }
    }

    #[test]
    fn the_moon_path_repeats_after_one_full_cycle() {
        let times = default_climate();
        let settings = vanilla_settings();
        for hour in [0.0, 6.5, 14.0, 21.0] {
            assert_same_place(
                moon_position(hour, 0, 4, &times, &settings).unwrap(),
                moon_position(hour, 32, 4, &times, &settings).unwrap(),
                "after 8 phases",
            );
        }
    }

    #[test]
    fn the_moon_does_not_jump_at_midnight() {
        let times = default_climate();
        let settings = vanilla_settings();
        for days in [0, 5, 31] {
            assert_same_place(
                moon_position(23.9999, days, 4, &times, &settings).unwrap(),
                moon_position(0.0, days + 1, 4, &times, &settings).unwrap(),
                "midnight",
            );
        }
    }

    #[test]
    fn a_zero_phase_length_is_an_error_for_the_moon_path() {
        assert_eq!(
            moon_position(12.0, 3, 0, &default_climate(), &vanilla_settings()),
            Err(MoonError::ZeroPhaseLength)
        );
    }

    fn goodneighbor_moon_at(total_hours: f64, phase_length: u32) -> SkyPosition {
        let time = DayTime::from_total_hours(total_hours).unwrap();
        super::moon_position(
            time,
            phase_length,
            &goodneighbor_climate(),
            &vanilla_settings(),
        )
        .unwrap()
    }

    #[test]
    fn in_goodneighbors_climate_with_three_day_phases_the_moon_rises_and_sets_where_expected() {
        // Day 11.5 h, night 12.5 h, cycle 24 days: the moon's apparent day is 24 * 24 / 23 h.
        let sunset = 18.25;
        let first_rise = goodneighbor_moon_at(sunset, 3);
        assert_close(first_rise.elevation_deg, 0.0, 0.05, "first rise elevation");
        assert_close(first_rise.azimuth_deg, 90.0, 0.01, "first rise azimuth");

        let first_set = goodneighbor_moon_at(sunset + 12.5 * 24.0 / 23.0, 3);
        assert_close(first_set.elevation_deg, 0.0, 0.05, "first set elevation");
        assert_close(first_set.azimuth_deg, 270.0, 0.5, "first set azimuth");

        for k in [1.0_f64, 5.0, 12.0, 23.0] {
            let rise = goodneighbor_moon_at(sunset + k * 24.0 * 24.0 / 23.0, 3);
            assert_close(rise.elevation_deg, 0.0, 0.05, "later rise elevation");
            assert_close(
                angular_gap(rise.azimuth_deg, 90.0),
                0.0,
                0.5,
                "later rise azimuth",
            );
        }
    }

    #[test]
    fn the_start_phase_decides_where_the_moon_stands_on_day_zero() {
        // MOON_PHASE_AT_DAY_ZERO is Full: the moon is opposite the sun at sunset.
        assert_eq!(MOON_PHASE_AT_DAY_ZERO, MoonPhase::Full);
        assert_eq!(MOON_PHASE_AT_DAY_ZERO.index(), 0);
    }

    #[test]
    fn star_alpha_is_the_clamped_luminance() {
        assert_eq!(star_alpha(Vec3::ZERO), 0.0);
        assert!((star_alpha(Vec3::ONE) - 1.0).abs() < 1e-6);
        assert!((star_alpha(Vec3::splat(0.25)) - 0.25).abs() < 1e-6);
        assert_eq!(star_alpha(Vec3::splat(8.0)), 1.0);
        assert_eq!(star_alpha(Vec3::splat(-1.0)), 0.0);
    }

    #[test]
    fn star_alpha_is_the_luminance_of_the_colour_below_its_cap() {
        let colour = Vec3::new(0.5, 0.25, 0.125);
        assert_eq!(star_alpha(colour), luminance(colour));
        assert!(luminance(colour) < 0.5, "the colour must not be capped");
    }

    #[test]
    fn the_sun_alpha_ramps_over_the_transition_centred_on_each_window_midpoint() {
        // Rise at 07:00, set at 19:00 for the default climate; the ramp is 2 h wide.
        let times = default_climate();
        let alpha = |hour| sun_alpha(hour, &times, 2.0);
        assert_eq!(alpha(5.9), 0.0);
        assert_eq!(alpha(6.0), 0.0);
        assert_eq!(alpha(6.5), 0.25);
        assert_eq!(alpha(7.0), 0.5);
        assert_eq!(alpha(8.0), 1.0);
        assert_eq!(alpha(13.0), 1.0);
        assert_eq!(alpha(18.0), 1.0);
        assert_eq!(alpha(19.5), 0.25);
        assert_eq!(alpha(20.0), 0.0);
        assert_eq!(alpha(23.0), 0.0);
    }

    #[test]
    fn a_zero_transition_makes_the_sun_alpha_a_step_at_the_midpoints() {
        let times = default_climate();
        assert_eq!(sun_alpha(6.99, &times, 0.0), 0.0);
        assert_eq!(sun_alpha(7.01, &times, 0.0), 1.0);
        assert_eq!(sun_alpha(18.99, &times, 0.0), 1.0);
        assert_eq!(sun_alpha(19.01, &times, 0.0), 0.0);
    }

    #[test]
    fn the_sun_light_weight_fades_in_over_the_first_degrees_of_elevation() {
        let (start, end) = SUN_LIGHT_FADE_DEG;
        assert_eq!(sun_light_weight(start - 1.0), 0.0);
        assert_eq!(sun_light_weight(start), 0.0);
        assert!((sun_light_weight(f32::midpoint(start, end)) - 0.5).abs() < 1e-6);
        assert_eq!(sun_light_weight(end), 1.0);
        assert_eq!(sun_light_weight(90.0), 1.0);
    }
}
