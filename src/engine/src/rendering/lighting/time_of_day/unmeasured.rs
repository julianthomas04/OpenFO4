//! The rules no game data or public source pins down. Each is a hypothesis to be
//! replaced by in-game measurement; nothing outside this module encodes them.
//!
//! - Slot keyframes: each twilight window is split in quarters, and the sun rises and
//!   sets at the window midpoints (the rule the water daylight factor uses).
//! - Sun arc: a circle through due east and due west whose plane tilts so the noon
//!   sun stands due south at `atan2(|fSunYExtreme|, |fSunXExtreme|)` elevation; it
//!   rises at the sunrise midpoint, sets at the sunset midpoint and keeps going below the horizon.
//! - Moon path: the same circle; on day 0 the moon is in [`MOON_PHASE_AT_DAY_ZERO`] and
//!   its distance behind the sun is that phase's share of the cycle, growing by one
//!   cycle in `8 * phase_length` days (a full moon rises at the sunset midpoint).
//! - Stars: one revolution per `fStarsRotateDays`, in the same sense as the sun, starting
//!   at [`STAR_ANGLE_AT_DAY_ZERO_RAD`]; their opacity is the luminance of the weather Stars colour, taken as linear.
//! - Moon size: [`MOON_SIZE_UNITS_PER_DEGREE`] size units make one degree of the moon's apparent diameter.
//! - Star horizon fade: the stars fade in over the first [`STAR_HORIZON_FADE_SINE`] of the sine of elevation.
//! - Night sky brightness: the moon texture is shown at [`MOON_RADIANCE`] and the star layer at
//!   [`STAR_RADIANCE`] times the sky radiance scale.
//! - Light: the sun lights the ground over [`SUN_LIGHT_FADE_DEG`] of elevation, the moon over
//!   `fMasserAngleFadeStart..End` (ignoring phase); the moon only takes over as the sun fades:
//!   the direction turns from the sun's to the moon's along the shorter arc (through the upper
//!   hemisphere when the two are opposite) as the moon's weight times `1 - sun weight` grows,
//!   and the stronger weight is the strength. Colour and
//!   intensity come from the weather slots.
//! - Sun alpha: `fSunAlphaTransTime` is a number of hours; the sun's alpha ramps linearly over
//!   that long, centred on the midpoint of each twilight window (see [`sun_alpha`]).

use std::f32::consts::PI;

use glam::{Quat, Vec3};

use super::climate_times::{ClimateTimes, ClimateWindow};
use super::day_time::DayTime;
use super::light::LightSource;
use super::moon::{MoonError, MoonPhase, cycle_days, moon_visibility};
use super::sky::{SkyGameSettings, SkyPosition, linear_fade};
use super::slot::TimeSlot;
use super::wrap::{HOURS_PER_DAY, wrap_hours};

/// Elevations (start, end) in degrees over which the sun direct light fades in.
pub const SUN_LIGHT_FADE_DEG: (f32, f32) = (0.0, 5.0);

/// The moon phase on day 0, which also fixes where it stands relative to the sun then.
pub const MOON_PHASE_AT_DAY_ZERO: MoonPhase = MoonPhase::Full;

/// Sign of the star dome turn about its axis: -1 makes a star on the east horizon
/// climb towards the south, like the sun, for the right-handed turn `StarRotation` states.
pub const STAR_ROTATION_SENSE: f64 = -1.0;

/// The star dome turn at day 0, 00:00.
pub const STAR_ANGLE_AT_DAY_ZERO_RAD: f32 = 0.0;

const STAR_LUMINANCE_WEIGHTS: Vec3 = Vec3::new(0.2126, 0.7152, 0.0722);

/// A point on the day timeline: a position inside one of the climate windows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Keyframe {
    pub window: ClimateWindow,
    /// 0 at the window begin, 1 at its end.
    pub fraction: f32,
    pub slot: TimeSlot,
}

impl Keyframe {
    pub fn hour(&self, times: &ClimateTimes) -> f32 {
        let (begin, end) = times.window(self.window);
        begin + (end - begin) * self.fraction
    }

    /// How much daylight the slot stands for: 0 for night, 1 for full day, rising
    /// with the fraction through sunrise and falling through sunset.
    pub fn daylight_weight(&self) -> f32 {
        match self.window {
            ClimateWindow::Sunrise => self.fraction,
            ClimateWindow::Sunset => 1.0 - self.fraction,
        }
    }
}

const fn keyframe(window: ClimateWindow, fraction: f32, slot: TimeSlot) -> Keyframe {
    Keyframe {
        window,
        fraction,
        slot,
    }
}

/// The day keyframes in time order. The span from the last one back to the
/// first wraps over midnight; `Day` holds between sunrise end and sunset begin,
/// `Night` between sunset end and the next sunrise begin.
pub const KEYFRAMES: [Keyframe; 10] = [
    keyframe(ClimateWindow::Sunrise, 0.0, TimeSlot::Night),
    keyframe(ClimateWindow::Sunrise, 0.25, TimeSlot::EarlySunrise),
    keyframe(ClimateWindow::Sunrise, 0.5, TimeSlot::Sunrise),
    keyframe(ClimateWindow::Sunrise, 0.75, TimeSlot::LateSunrise),
    keyframe(ClimateWindow::Sunrise, 1.0, TimeSlot::Day),
    keyframe(ClimateWindow::Sunset, 0.0, TimeSlot::Day),
    keyframe(ClimateWindow::Sunset, 0.25, TimeSlot::EarlySunset),
    keyframe(ClimateWindow::Sunset, 0.5, TimeSlot::Sunset),
    keyframe(ClimateWindow::Sunset, 0.75, TimeSlot::LateSunset),
    keyframe(ClimateWindow::Sunset, 1.0, TimeSlot::Night),
];

/// Elevation of the noon sun, from the horizontal reach and the southward offset of the extremes.
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

pub fn sun_position(
    time: DayTime,
    times: &ClimateTimes,
    settings: &SkyGameSettings,
) -> SkyPosition {
    let rise = times.sunrise_midpoint();
    let up_hours = times.sunset_midpoint() - rise;
    orbit_position(time.hour(), rise, up_hours, noon_elevation_rad(settings))
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
    stars_colour_linear
        .dot(STAR_LUMINANCE_WEIGHTS)
        .clamp(0.0, 1.0)
}

/// 0 while the sun is at or below the horizon, 1 from [`SUN_LIGHT_FADE_DEG`]'s end upwards.
pub fn sun_light_weight(elevation_deg: f32) -> f32 {
    let (fade_start, fade_end) = SUN_LIGHT_FADE_DEG;
    linear_fade(elevation_deg, fade_start, fade_end)
}

/// Geometry of the exterior one directional light. The direction turns from the sun's to
/// the moon's as `from_moon_weight` goes from 0 to 1, so it never snaps while the light is strong.
pub fn light_source(
    sun: SkyPosition,
    moon: Option<SkyPosition>,
    settings: &SkyGameSettings,
) -> LightSource {
    let sun_weight = sun_light_weight(sun.elevation_deg);
    let moon_weight = moon.map_or(0.0, |moon| moon_visibility(moon.elevation_deg, settings));
    let moon_share = moon_weight * (1.0 - sun_weight);
    let total = sun_weight + moon_share;
    let from_moon_weight = if total > 0.0 { moon_share / total } else { 0.5 };
    let direction = match moon {
        Some(moon) => turn_towards(sun.direction(), moon.direction(), from_moon_weight),
        None => sun.direction(),
    };
    LightSource {
        direction,
        from_moon_weight,
        strength: sun_weight.max(moon_weight),
    }
}

/// Directions this close to opposite have no unique shorter arc.
const OPPOSITE_COS: f32 = -0.9999;

/// The unit direction `t` of the way along the shorter arc from `from` to `to`. For opposite
/// directions the arc passes over the zenith (the upper hemisphere), so the choice is
/// deterministic and keeps the light above the ground.
fn turn_towards(from: Vec3, to: Vec3, t: f32) -> Vec3 {
    let cos = from.dot(to).clamp(-1.0, 1.0);
    if cos > OPPOSITE_COS {
        let angle = cos.acos();
        if angle < f32::EPSILON {
            return from;
        }
        let along = (from * ((1.0 - t) * angle).sin() + to * (t * angle).sin()) / angle.sin();
        return along.normalize();
    }
    let axis = from
        .cross(Vec3::Y)
        .try_normalize()
        .unwrap_or_else(|| from.cross(Vec3::X).normalize());
    let quarter = Quat::from_axis_angle(axis, PI / 2.0) * from;
    let turn = if quarter.y >= 0.0 { PI } else { -PI };
    (Quat::from_axis_angle(axis, turn * t) * from).normalize()
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
/// Secunda (75) 2.5 degrees, a few times the sun's 1 degree disc.
pub const MOON_SIZE_UNITS_PER_DEGREE: f32 = 30.0;

/// The sine of the elevation at which the stars are fully faded in; they fade from 0 at the horizon.
pub const STAR_HORIZON_FADE_SINE: f32 = 0.05;

/// Radiance of a fully lit moon texel at sky radiance scale 1.
pub const MOON_RADIANCE: f32 = 4.0;

/// Radiance of a fully opaque star texel at sky radiance scale 1.
pub const STAR_RADIANCE: f32 = 2.0;

/// The apparent diameter in degrees of a moon whose size setting is `size`.
pub fn moon_angular_diameter_deg(size: f32) -> f32 {
    size / MOON_SIZE_UNITS_PER_DEGREE
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact binary fractions of an hour"
)]
mod tests {
    use super::super::test_support::{
        at, at_hour, default_climate, goodneighbor_climate, vanilla_settings,
    };
    use proptest::prelude::*;

    use super::*;
    use crate::rendering::lighting::state::SunSettings;

    const NOON_ELEVATION_DEG: f32 = 28.4;

    fn sun_position(hour: f32, times: &ClimateTimes, settings: &SkyGameSettings) -> SkyPosition {
        super::sun_position(at_hour(hour), times, settings)
    }

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
    fn the_keyframe_table_covers_every_slot_and_alternates_windows_in_order() {
        for slot in TimeSlot::ALL {
            assert!(KEYFRAMES.iter().any(|k| k.slot == slot), "{slot:?}");
        }
        let (sunrise, sunset) = KEYFRAMES.split_at(5);
        for (window, frames) in [
            (ClimateWindow::Sunrise, sunrise),
            (ClimateWindow::Sunset, sunset),
        ] {
            assert!(frames.iter().all(|k| k.window == window));
            assert_eq!(frames[0].fraction, 0.0);
            assert_eq!(frames[4].fraction, 1.0);
            assert!(
                frames
                    .windows(2)
                    .all(|pair| pair[0].fraction < pair[1].fraction)
            );
        }
    }

    #[test]
    fn daylight_weights_run_zero_to_one_across_sunrise_and_back_across_sunset() {
        let weights: Vec<f32> = KEYFRAMES.iter().map(Keyframe::daylight_weight).collect();
        assert_eq!(
            weights,
            [0.0, 0.25, 0.5, 0.75, 1.0, 1.0, 0.75, 0.5, 0.25, 0.0]
        );
    }

    #[test]
    fn a_slot_that_appears_twice_has_the_same_weight_both_times() {
        for slot in [TimeSlot::Day, TimeSlot::Night] {
            let weights: Vec<f32> = KEYFRAMES
                .iter()
                .filter(|k| k.slot == slot)
                .map(Keyframe::daylight_weight)
                .collect();
            assert_eq!(weights.len(), 2, "{slot:?}");
            assert_eq!(weights[0], weights[1], "{slot:?}");
        }
    }

    #[test]
    fn keyframe_hours_follow_the_window_fractions() {
        let times = default_climate();
        let hours: Vec<f32> = KEYFRAMES.iter().map(|k| k.hour(&times)).collect();
        assert_eq!(
            hours,
            [5.0, 6.0, 7.0, 8.0, 9.0, 17.0, 18.0, 19.0, 20.0, 21.0]
        );
    }

    #[test]
    fn the_noon_sun_stands_due_south_at_the_extremes_elevation() {
        let position = sun_position(13.0, &default_climate(), &vanilla_settings());
        assert_close(position.azimuth_deg, 180.0, 0.01, "azimuth");
        assert_close(
            position.elevation_deg,
            NOON_ELEVATION_DEG,
            0.05,
            "elevation",
        );
    }

    #[test]
    fn the_sun_rises_in_the_east_and_sets_in_the_west_at_the_window_midpoints() {
        let times = default_climate();
        let settings = vanilla_settings();
        let rise = sun_position(7.0, &times, &settings);
        assert_close(rise.elevation_deg, 0.0, 1e-3, "rise elevation");
        assert_close(rise.azimuth_deg, 90.0, 0.01, "rise azimuth");
        let set = sun_position(19.0, &times, &settings);
        assert_close(set.elevation_deg, 0.0, 1e-3, "set elevation");
        assert_close(set.azimuth_deg, 270.0, 0.01, "set azimuth");
    }

    #[test]
    fn the_sun_is_up_exactly_between_rise_and_set_for_every_vanilla_climate() {
        let settings = vanilla_settings();
        for (times, rise, set) in [
            (default_climate(), 7.0, 19.0),
            (
                ClimateTimes::new(5.5, 8.0, 16.0, 20.5).unwrap(),
                6.75,
                18.25,
            ),
            (ClimateTimes::new(4.0, 10.0, 16.0, 22.0).unwrap(), 7.0, 19.0),
        ] {
            for step in 0..96 {
                let hour = step as f32 * 0.25 + 0.125;
                let elevation = sun_position(hour, &times, &settings).elevation_deg;
                if hour > rise && hour < set {
                    assert!(elevation > 0.0, "hour {hour}: {elevation}");
                } else {
                    assert!(elevation < 0.0, "hour {hour}: {elevation}");
                }
            }
        }
    }

    #[test]
    fn the_sun_arc_is_symmetric_about_solar_noon() {
        let times = ClimateTimes::new(5.5, 8.0, 16.0, 20.5).unwrap();
        let settings = vanilla_settings();
        let noon = f32::midpoint(times.sunrise_midpoint(), times.sunset_midpoint());
        for offset in [0.5, 2.0, 5.0, 5.49, 8.0, 11.0] {
            let before = sun_position(noon - offset, &times, &settings);
            let after = sun_position(noon + offset, &times, &settings);
            assert_close(before.elevation_deg, after.elevation_deg, 1e-3, "elevation");
            assert_close(
                angular_gap(before.azimuth_deg, 360.0 - after.azimuth_deg),
                0.0,
                1e-2,
                "azimuth mirrors across south",
            );
        }
    }

    #[test]
    fn the_sun_is_highest_at_solar_noon() {
        let times = default_climate();
        let settings = vanilla_settings();
        let noon = sun_position(13.0, &times, &settings).elevation_deg;
        for step in 0..240 {
            let hour = step as f32 * 0.1;
            assert!(sun_position(hour, &times, &settings).elevation_deg <= noon + 1e-3);
        }
    }

    #[test]
    fn the_sun_is_due_north_below_the_horizon_at_solar_midnight() {
        let position = sun_position(1.0, &default_climate(), &vanilla_settings());
        assert_close(position.azimuth_deg, 0.0, 0.01, "azimuth");
        assert_close(
            position.elevation_deg,
            -NOON_ELEVATION_DEG,
            0.05,
            "elevation",
        );
    }

    #[test]
    fn the_sun_path_is_continuous_across_midnight_and_the_same_every_day() {
        let times = default_climate();
        let settings = vanilla_settings();
        assert_same_place(
            super::sun_position(at(3, 23.9999), &times, &settings),
            super::sun_position(at(4, 0.0), &times, &settings),
            "midnight",
        );
        for hour in [0.0, 3.3, 7.0, 12.5, 23.9] {
            assert_same_place(
                super::sun_position(at(0, hour), &times, &settings),
                super::sun_position(at(40, hour), &times, &settings),
                "forty days later",
            );
        }
    }

    #[test]
    fn the_sun_moves_smoothly_through_the_day() {
        let times = default_climate();
        let settings = vanilla_settings();
        let mut previous = sun_position(0.0, &times, &settings);
        for step in 1..2400 {
            let current = sun_position(step as f32 * 0.01, &times, &settings);
            let moved = previous
                .direction()
                .angle_between(current.direction())
                .to_degrees();
            assert!(moved < 0.5, "step {step}: {moved} degrees");
            previous = current;
        }
    }

    #[test]
    fn the_sun_direction_is_a_unit_vector_in_the_sun_settings_convention() {
        let times = default_climate();
        let settings = vanilla_settings();
        for step in 0..48 {
            let position = sun_position(step as f32 * 0.5, &times, &settings);
            let direction = position.direction();
            assert_close(direction.length(), 1.0, 1e-5, "length");
            let sun = SunSettings {
                azimuth_deg: position.azimuth_deg,
                elevation_deg: position.elevation_deg,
                ..SunSettings::default()
            };
            assert!(direction.abs_diff_eq(sun.direction_to_sun(), 1e-5));
            assert_eq!(direction.y > 0.0, position.elevation_deg > 0.0);
        }
    }

    #[test]
    fn a_flat_extreme_puts_the_sun_on_the_horizon_all_day() {
        let settings =
            SkyGameSettings::new(600.0, 0.0, 1.0027, Vec3::new(-0.09, 0.35, 1.0), 5.0, 10.0)
                .unwrap();
        let position = sun_position(13.0, &default_climate(), &settings);
        assert_close(position.elevation_deg, 0.0, 1e-3, "elevation");
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
    fn half_a_cycle_after_the_first_sunset_the_moon_sits_where_the_sun_is() {
        let times = default_climate();
        let settings = vanilla_settings();
        assert_same_place(
            moon_position(19.0, 16, 4, &times, &settings).unwrap(),
            sun_position(19.0, &times, &settings),
            "new moon at sunset",
        );
        for hour in [0.0, 7.0, 13.0, 22.5] {
            let moon = moon_position(hour, 16, 4, &times, &settings).unwrap();
            let sun = sun_position(hour, &times, &settings);
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
    fn in_goodneighbors_climate_the_sun_is_up_for_eleven_and_a_half_hours() {
        let times = goodneighbor_climate();
        let settings = vanilla_settings();
        let up = (0..2400)
            .filter(|step| {
                super::sun_position(at_hour(*step as f32 * 0.01), &times, &settings).elevation_deg
                    > 0.0
            })
            .count();
        assert!((up as f32 * 0.01 - 11.5).abs() < 0.02, "{up}");
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
    fn star_alpha_weighs_each_channel_by_its_luminance_share() {
        assert!((star_alpha(Vec3::X) - 0.2126).abs() < 1e-6);
        assert!((star_alpha(Vec3::Y) - 0.7152).abs() < 1e-6);
        assert!((star_alpha(Vec3::Z) - 0.0722).abs() < 1e-6);
    }

    fn at_sky(elevation_deg: f32, azimuth_deg: f32) -> SkyPosition {
        SkyPosition {
            azimuth_deg,
            elevation_deg,
        }
    }

    fn light(sun_elevation: f32, moon_elevation: f32) -> LightSource {
        light_source(
            at_sky(sun_elevation, 90.0),
            Some(at_sky(moon_elevation, 270.0)),
            &vanilla_settings(),
        )
    }

    #[test]
    fn a_high_sun_is_the_whole_light_even_with_the_moon_up() {
        for moon_elevation in [-30.0, 20.0, 60.0] {
            let source = light(40.0, moon_elevation);
            assert_eq!(source.strength, 1.0);
            assert!(
                source
                    .direction
                    .abs_diff_eq(at_sky(40.0, 90.0).direction(), 1e-6)
            );
        }
        assert_eq!(light(40.0, -30.0).from_moon_weight, 0.0);
    }

    #[test]
    fn a_high_moon_after_dark_is_the_whole_light() {
        let source = light(-30.0, 40.0);
        assert_eq!(source.from_moon_weight, 1.0);
        assert_eq!(source.strength, 1.0);
        assert!(
            source
                .direction
                .abs_diff_eq(at_sky(40.0, 270.0).direction(), 1e-6)
        );
    }

    #[test]
    fn with_neither_body_up_the_strength_is_zero() {
        for (sun, moon) in [(-10.0, 2.0), (-30.0, -30.0), (0.0, 5.0)] {
            assert_eq!(light(sun, moon).strength, 0.0, "{sun} {moon}");
        }
    }

    fn angle(a: Vec3, b: Vec3) -> f32 {
        a.dot(b).clamp(-1.0, 1.0).acos()
    }

    #[test]
    fn partial_weights_turn_the_direction_along_the_arc_by_the_moons_share() {
        // Sun weight 0.5, moon weight 0.5: the moon's share is 0.25 of 0.75, a third.
        let source = light(2.5, 7.5);
        assert!((source.from_moon_weight - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(source.strength, 0.5);
        let sun = at_sky(2.5, 90.0).direction();
        let moon = at_sky(7.5, 270.0).direction();
        let total = angle(sun, moon);
        assert!((angle(sun, source.direction) - total / 3.0).abs() < 1e-4);
        assert!((angle(source.direction, moon) - total * 2.0 / 3.0).abs() < 1e-4);
    }

    #[test]
    fn a_fully_up_sun_leaves_the_moon_no_share_of_the_direction() {
        let source = light(30.0, 60.0);
        assert_eq!(source.from_moon_weight, 0.0);
    }

    #[test]
    fn the_moon_has_the_whole_direction_once_the_sun_gives_no_light() {
        let source = light(0.0, 7.6);
        assert_eq!(source.from_moon_weight, 1.0);
        assert!(
            source
                .direction
                .abs_diff_eq(at_sky(7.6, 270.0).direction(), 1e-5)
        );
    }

    #[test]
    fn opposite_directions_turn_over_the_zenith_not_through_the_ground() {
        for from in [Vec3::X, Vec3::Z, -Vec3::X, Vec3::new(0.6, 0.0, 0.8)] {
            let to = -from;
            assert!(turn_towards(from, to, 0.0).abs_diff_eq(from, 1e-5));
            assert!(turn_towards(from, to, 1.0).abs_diff_eq(to, 1e-5));
            for step in 1..10 {
                let turned = turn_towards(from, to, step as f32 / 10.0);
                assert!((turned.length() - 1.0).abs() < 1e-5);
                assert!(turned.y >= -1e-5, "{from} at {step}: {turned}");
            }
        }
    }

    #[test]
    fn straight_up_to_straight_down_still_has_a_defined_path() {
        let turned = turn_towards(Vec3::Y, -Vec3::Y, 0.5);
        assert!((turned.length() - 1.0).abs() < 1e-5);
        assert!(turned.y.abs() < 1e-5);
    }

    #[test]
    fn turning_between_equal_directions_returns_them() {
        assert!(turn_towards(Vec3::X, Vec3::X, 0.7).abs_diff_eq(Vec3::X, 1e-6));
    }

    #[test]
    fn the_direction_never_jumps_while_the_light_is_strong_over_the_whole_moon_cycle() {
        for (name, times) in [
            ("DefaultClimate", default_climate()),
            ("Goodneighbor", goodneighbor_climate()),
        ] {
            let settings = vanilla_settings();
            let mut previous: Option<LightSource> = None;
            let steps_per_day = 2400;
            for step in 0..32 * steps_per_day {
                let time = at(step / steps_per_day, (step % steps_per_day) as f32 * 0.01);
                let source = light_source(
                    super::sun_position(time, &times, &settings),
                    Some(super::moon_position(time, 4, &times, &settings).unwrap()),
                    &settings,
                );
                if let Some(before) = previous.filter(|before| before.strength >= 0.5)
                    && source.strength >= 0.5
                {
                    let turn = angle(before.direction, source.direction).to_degrees();
                    assert!(turn < 15.0, "{name} step {step}: {turn} degrees");
                }
                previous = Some(source);
            }
        }
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
    fn the_sun_fades_in_over_its_elevation_band() {
        let (start, end) = SUN_LIGHT_FADE_DEG;
        assert_eq!(light(start, -30.0).strength, 0.0);
        assert!((light(f32::midpoint(start, end), -30.0).strength - 0.5).abs() < 1e-6);
        assert_eq!(light(end, -30.0).strength, 1.0);
    }

    #[test]
    fn a_rising_moon_fades_in_over_the_game_fade_angles() {
        assert_eq!(light(-30.0, 5.0).strength, 0.0);
        assert!((light(-30.0, 7.5).strength - 0.5).abs() < 1e-6);
        assert_eq!(light(-30.0, 10.0).strength, 1.0);
    }

    #[test]
    fn a_high_full_moon_through_dawn_never_dims_the_light() {
        let mut previous = light(-10.0, 40.0);
        for step in 1..=300 {
            let sun_elevation = -10.0 + step as f32 * 0.1;
            let current = light(sun_elevation, 40.0);
            assert_eq!(current.strength, 1.0, "sun at {sun_elevation}");
            assert!(
                (current.from_moon_weight - previous.from_moon_weight).abs() < 0.05,
                "colour weight jumps at sun elevation {sun_elevation}"
            );
            previous = current;
        }
        assert_eq!(previous.from_moon_weight, 0.0);
    }

    #[test]
    fn the_strength_is_continuous_while_the_sun_rises_against_any_moon_elevation() {
        for moon_elevation in [-20.0, 3.0, 7.0, 15.0, 60.0] {
            let mut previous = light(-10.0, moon_elevation).strength;
            for step in 1..=300 {
                let sun_elevation = -10.0 + step as f32 * 0.1;
                let strength = light(sun_elevation, moon_elevation).strength;
                assert!(
                    (strength - previous).abs() < 0.03,
                    "moon {moon_elevation}, sun {sun_elevation}"
                );
                previous = strength;
            }
        }
    }

    #[test]
    fn over_a_full_moon_day_the_light_is_the_sun_by_day_and_the_moon_by_night() {
        let times = default_climate();
        let settings = vanilla_settings();
        let source = |hour: f32| {
            light_source(
                super::sun_position(at_hour(hour), &times, &settings),
                Some(super::moon_position(at_hour(hour), 4, &times, &settings).unwrap()),
                &settings,
            )
        };
        for hour in [10.0, 13.0, 16.0] {
            let source = source(hour);
            assert_eq!(
                (source.from_moon_weight, source.strength),
                (0.0, 1.0),
                "{hour}"
            );
        }
        for hour in [22.0, 0.0, 2.0] {
            let source = source(hour);
            assert_eq!(
                (source.from_moon_weight, source.strength),
                (1.0, 1.0),
                "{hour}"
            );
        }
    }

    #[test]
    fn without_a_moon_the_sun_is_the_only_light() {
        let settings = vanilla_settings();
        let by_day = light_source(at_sky(40.0, 90.0), None, &settings);
        assert_eq!((by_day.from_moon_weight, by_day.strength), (0.0, 1.0));
        assert!(
            by_day
                .direction
                .abs_diff_eq(at_sky(40.0, 90.0).direction(), 1e-6)
        );
        let by_night = light_source(at_sky(-30.0, 90.0), None, &settings);
        assert_eq!(by_night.strength, 0.0);
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

    proptest! {
        #[test]
        fn the_light_is_always_a_unit_direction_with_weights_in_range(
            sun_elevation in -90.0f32..90.0,
            moon_elevation in -90.0f32..90.0,
            sun_azimuth in 0.0f32..360.0,
            moon_azimuth in 0.0f32..360.0,
        ) {
            let source = light_source(
                at_sky(sun_elevation, sun_azimuth),
                Some(at_sky(moon_elevation, moon_azimuth)),
                &vanilla_settings(),
            );
            prop_assert!((source.direction.length() - 1.0).abs() < 1e-5);
            prop_assert!((0.0..=1.0).contains(&source.from_moon_weight));
            prop_assert!((0.0..=1.0).contains(&source.strength));
        }
    }
}
