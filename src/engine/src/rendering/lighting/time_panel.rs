use std::ops::RangeInclusive;

use super::time_of_day::{SkyMoment, SlotBlend};
use crate::rendering::post::night_sky::{AssetState, SkyAssetStatus};
use crate::sim::commands::ClockEdit;
use crate::sim::world_clock::{GameClock, WorldClock};

const HOURS_PER_DAY: f32 = 24.0;
const MINUTES_PER_HOUR: f64 = 60.0;
const MINUTES_PER_DAY: f64 = 24.0 * MINUTES_PER_HOUR;
const WHOLE_MINUTE_EPSILON: f64 = 1e-3;
const SECONDS_PER_MINUTE: f64 = 60.0;
const SECONDS_PER_HOUR: f64 = 3600.0;
const SECONDS_PER_GAME_DAY_AT_SCALE_1: f64 = MINUTES_PER_DAY * SECONDS_PER_MINUTE;
pub const TIME_SCALE_RANGE: RangeInclusive<f32> = 0.1..=10_000.0;

/// The clock values the panel edits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeControls {
    pub hour: f32,
    pub paused: bool,
    pub time_scale: f32,
}

impl TimeControls {
    pub fn of(clock: &WorldClock) -> Self {
        Self {
            hour: clock.hour(),
            paused: clock.paused(),
            time_scale: clock.time_scale(),
        }
    }
}

/// `sky` is where the clock last put the sky, `None` before the first exterior frame;
/// `sky_assets` is `None` while the exterior has no clocked sky to draw.
pub fn time_panel(
    ui: &mut egui::Ui,
    clock: &WorldClock,
    sky: Option<&SkyMoment>,
    sky_assets: Option<&SkyAssetStatus>,
    edits: &mut Vec<ClockEdit>,
) {
    let before = TimeControls::of(clock);
    let mut after = before;

    ui.add(
        egui::Slider::new(&mut after.hour, 0.0..=HOURS_PER_DAY)
            .text("time of day")
            .custom_formatter(|hour, _| format_hour(hour as f32))
            .custom_parser(parse_hour),
    );
    ui.checkbox(&mut after.paused, "Pause time");
    ui.horizontal(|ui| {
        ui.add(
            egui::Slider::new(&mut after.time_scale, TIME_SCALE_RANGE)
                .logarithmic(true)
                .clamping(egui::SliderClamping::Edits)
                .suffix("x")
                .text("time scale"),
        );
        if ui.button("Reset").clicked() {
            after.time_scale = clock.start_time_scale();
        }
    });
    ui.label(format!("date: {}", format_date(&clock.date())));
    ui.label(format!("days passed: {}", clock.days_passed()));
    ui.label(format_time_scale(clock.time_scale()));
    ui.label(weather_slot_label(sky.map(|moment| &moment.blend)));
    ui.label(moon_label(sky));
    ui.label(sky_assets.map_or_else(|| "Sky assets: -".to_owned(), sky_assets_label));

    edits.extend(clock_edits(&before, &after));
}

/// The edits that turn `before` into `after`; a value the user did not touch produces none, so
/// the running clock is never overwritten by its own readout.
#[allow(
    clippy::float_cmp,
    reason = "an untouched slider hands back the exact value"
)]
pub fn clock_edits(before: &TimeControls, after: &TimeControls) -> Vec<ClockEdit> {
    let mut edits = Vec::new();
    if after.paused != before.paused {
        edits.push(ClockEdit::SetPaused(after.paused));
    }
    if after.time_scale != before.time_scale {
        edits.push(ClockEdit::SetTimeScale(after.time_scale));
    }
    if after.hour != before.hour {
        edits.push(ClockEdit::SetHour(after.hour));
    }
    edits
}

/// `HH:MM` with the minute truncated, as the sim and the Pip-Boy show the clock. The epsilon
/// absorbs the `f32` error of an hour that sits on a whole minute. Only 24.0 itself reads `24:00`.
pub fn format_hour(hour: f32) -> String {
    let hour = if hour.is_nan() { 0.0 } else { hour };
    let minutes = (f64::from(hour) * MINUTES_PER_HOUR + WHOLE_MINUTE_EPSILON)
        .floor()
        .clamp(0.0, MINUTES_PER_DAY) as u32;
    let last_minute = MINUTES_PER_DAY as u32 - 1;
    let minutes = if hour < HOURS_PER_DAY {
        minutes.min(last_minute)
    } else {
        minutes
    };
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// Reads what [`format_hour`] writes (`H:MM`, `HH:MM`) or a plain decimal hour; `None` outside
/// `0..=24`.
pub fn parse_hour(text: &str) -> Option<f64> {
    let text = text.trim();
    let hour = match text.split_once(':') {
        Some((hours, minutes)) => {
            let minutes: u8 = minutes.trim().parse().ok()?;
            if minutes >= 60 {
                return None;
            }
            f64::from(hours.trim().parse::<u8>().ok()?) + f64::from(minutes) / MINUTES_PER_HOUR
        }
        None => text.parse().ok()?,
    };
    (0.0..=f64::from(HOURS_PER_DAY))
        .contains(&hour)
        .then_some(hour)
}

pub fn format_date(date: &GameClock) -> String {
    format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
}

/// `Weather slot: LateSunrise -> Day (42%)`, or just the slot while the blend holds one.
pub fn weather_slot_label(blend: Option<&SlotBlend>) -> String {
    match blend {
        None => "Weather slot: -".to_owned(),
        Some(blend) if blend.from == blend.to => {
            format!("Weather slot: {}", blend.from.json_key())
        }
        Some(blend) => format!(
            "Weather slot: {} -> {} ({}%)",
            blend.from.json_key(),
            blend.to.json_key(),
            (blend.t * 100.0).round() as u32
        ),
    }
}

/// `Moon: HalfWax`; `none` where the climate shows no moon.
pub fn moon_label(sky: Option<&SkyMoment>) -> String {
    match sky {
        None => "Moon: -".to_owned(),
        Some(moment) => match &moment.moon {
            Some(moon) => format!("Moon: {:?}", moon.phase),
            None => "Moon: none".to_owned(),
        },
    }
}

fn state_word(state: &AssetState) -> &'static str {
    match state {
        AssetState::NotRequested => "not requested",
        AssetState::Loaded => "loaded",
        AssetState::Placeholder => "placeholder",
        AssetState::Failed(_) => "failed",
    }
}

/// `Sky assets: star texture loaded, star mesh loaded, sun loaded, sun glare loaded, moon 7 loaded, 1 failed`.
pub fn sky_assets_label(status: &SkyAssetStatus) -> String {
    let count = |wanted: fn(&AssetState) -> bool| status.moon.iter().filter(|s| wanted(s)).count();
    let counts = [
        (count(|s| matches!(s, AssetState::Loaded)), "loaded"),
        (
            count(|s| matches!(s, AssetState::Placeholder)),
            "placeholder",
        ),
        (count(|s| matches!(s, AssetState::Failed(_))), "failed"),
    ];
    let moon: Vec<String> = counts
        .into_iter()
        .filter(|(number, _)| *number > 0)
        .map(|(number, word)| format!("{number} {word}"))
        .collect();
    let moon = if moon.is_empty() {
        state_word(&AssetState::NotRequested).to_owned()
    } else {
        moon.join(", ")
    };
    format!(
        "Sky assets: star texture {}, star mesh {}, sun {}, sun glare {}, moon {moon}",
        state_word(&status.star_texture),
        state_word(&status.star_mesh),
        state_word(&status.sun_texture),
        state_word(&status.sun_glare_texture),
    )
}

/// `20x (one game day = 1 h 12 min)`.
pub fn format_time_scale(time_scale: f32) -> String {
    let seconds = SECONDS_PER_GAME_DAY_AT_SCALE_1 / f64::from(time_scale);
    let day_length = if seconds < SECONDS_PER_MINUTE {
        format!("{seconds:.1} s")
    } else {
        let minutes = (seconds / SECONDS_PER_MINUTE).round() as u64;
        let hours_in_minutes = SECONDS_PER_HOUR / SECONDS_PER_MINUTE;
        if (minutes as f64) < hours_in_minutes {
            format!("{minutes} min")
        } else {
            let per_hour = hours_in_minutes as u64;
            format!("{} h {} min", minutes / per_hour, minutes % per_hour)
        }
    };
    format!("{time_scale}x (one game day = {day_length})")
}

#[cfg(test)]
mod tests {
    use super::super::time_of_day::{
        LightSource, MoonKind, MoonPhase, MoonSky, SkyPosition, StarRotation, TimeSlot,
    };
    use super::*;
    use crate::rendering::post::night_sky::{AssetFailure, AssetFailureKind};

    fn controls() -> TimeControls {
        TimeControls {
            hour: 11.0,
            paused: false,
            time_scale: 20.0,
        }
    }

    #[test]
    fn hours_read_as_hh_mm() {
        assert_eq!(format_hour(0.0), "00:00");
        assert_eq!(format_hour(9.5), "09:30");
        assert_eq!(format_hour(11.0), "11:00");
        assert_eq!(format_hour(18.25), "18:15");
    }

    #[test]
    fn an_hour_truncates_to_the_minute() {
        assert_eq!(format_hour(10.0 + 29.9 / 60.0), "10:29");
        assert_eq!(format_hour(10.0 + 59.9 / 60.0), "10:59");
        assert_eq!(format_hour(10.0 + 30.1 / 60.0), "10:30");
    }

    #[test]
    fn an_hour_on_a_whole_minute_does_not_fall_back_a_minute() {
        for minute_of_day in 0..1440_u32 {
            let hour = (f64::from(minute_of_day) / 60.0) as f32;
            assert_eq!(
                format_hour(hour),
                format!("{:02}:{:02}", minute_of_day / 60, minute_of_day % 60),
                "minute {minute_of_day}"
            );
        }
    }

    #[test]
    fn the_text_matches_the_clock_minute_the_pip_boy_shows() {
        let mut clock = WorldClock::fixture();
        clock.advance(123.4);
        let minute = clock.date().minute_of_day;
        assert_eq!(
            format_hour(clock.hour()),
            format!("{:02}:{:02}", minute / 60, minute % 60)
        );
    }

    #[test]
    fn the_end_of_the_day_reads_23_59_until_the_hour_is_exactly_24() {
        assert_eq!(format_hour(23.999), "23:59");
        assert_eq!(format_hour(24.0), "24:00");
    }

    #[test]
    fn an_hour_outside_the_day_is_clamped() {
        assert_eq!(format_hour(-3.0), "00:00");
        assert_eq!(format_hour(f32::NAN), "00:00");
        assert_eq!(format_hour(30.0), "24:00");
    }

    #[test]
    fn typed_times_parse_to_decimal_hours() {
        assert_eq!(parse_hour("09:30"), Some(9.5));
        assert_eq!(parse_hour(" 7:15 "), Some(7.25));
        assert_eq!(parse_hour("24:00"), Some(24.0));
        assert_eq!(parse_hour("6.5"), Some(6.5));
        assert_eq!(parse_hour("0"), Some(0.0));
    }

    #[test]
    fn nonsense_times_do_not_parse() {
        for bad in [
            "", "abc", "12:60", "25:00", "24:30", "-1", "25", "12:", ":30", "1:2:3",
        ] {
            assert_eq!(parse_hour(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_formatted_hour_parses_back_to_the_same_minute() {
        for hour in [0.0, 0.5, 11.0, 17.75, 23.983_333, 24.0] {
            let parsed = parse_hour(&format_hour(hour)).unwrap();
            assert!((parsed - f64::from(hour)).abs() < 1.0 / 60.0, "{hour}");
        }
    }

    #[test]
    fn untouched_controls_produce_no_edits() {
        assert_eq!(clock_edits(&controls(), &controls()), []);
    }

    #[test]
    fn each_changed_control_produces_exactly_its_own_edit() {
        let before = controls();
        assert_eq!(
            clock_edits(
                &before,
                &TimeControls {
                    hour: 3.5,
                    ..before
                }
            ),
            [ClockEdit::SetHour(3.5)]
        );
        assert_eq!(
            clock_edits(
                &before,
                &TimeControls {
                    paused: true,
                    ..before
                }
            ),
            [ClockEdit::SetPaused(true)]
        );
        assert_eq!(
            clock_edits(
                &before,
                &TimeControls {
                    time_scale: 1.0,
                    ..before
                }
            ),
            [ClockEdit::SetTimeScale(1.0)]
        );
    }

    #[test]
    fn several_changes_at_once_produce_one_edit_each() {
        let before = controls();
        let after = TimeControls {
            hour: 3.5,
            paused: true,
            time_scale: 1.0,
        };
        assert_eq!(
            clock_edits(&before, &after),
            [
                ClockEdit::SetPaused(true),
                ClockEdit::SetTimeScale(1.0),
                ClockEdit::SetHour(3.5)
            ]
        );
    }

    #[test]
    fn the_controls_mirror_the_clock() {
        let clock = WorldClock::fixture();
        assert_eq!(
            TimeControls::of(&clock),
            TimeControls {
                hour: 11.0,
                paused: false,
                time_scale: 20.0
            }
        );
    }

    #[test]
    fn the_default_time_scale_lies_inside_the_slider_range() {
        assert!(TIME_SCALE_RANGE.contains(&WorldClock::fixture().time_scale()));
        assert!(*TIME_SCALE_RANGE.end() <= crate::sim::world_clock::MAX_TIME_SCALE);
    }

    #[test]
    fn dates_read_as_year_month_day() {
        let date = WorldClock::fixture().date();
        assert_eq!(format_date(&date), "2287-10-23");
    }

    fn blend(from: TimeSlot, to: TimeSlot, t: f32) -> SlotBlend {
        SlotBlend { from, to, t }
    }

    #[test]
    fn the_slot_label_names_both_slots_and_the_percentage_between_them() {
        assert_eq!(
            weather_slot_label(Some(&blend(TimeSlot::LateSunrise, TimeSlot::Day, 0.4231))),
            "Weather slot: LateSunrise -> Day (42%)"
        );
        assert_eq!(
            weather_slot_label(Some(&blend(TimeSlot::Night, TimeSlot::EarlySunrise, 0.0))),
            "Weather slot: Night -> EarlySunrise (0%)"
        );
        assert_eq!(
            weather_slot_label(Some(&blend(TimeSlot::Sunset, TimeSlot::LateSunset, 1.0))),
            "Weather slot: Sunset -> LateSunset (100%)"
        );
    }

    #[test]
    fn the_slot_label_of_a_held_slot_names_it_alone() {
        assert_eq!(
            weather_slot_label(Some(&blend(TimeSlot::Day, TimeSlot::Day, 0.7))),
            "Weather slot: Day"
        );
    }

    #[test]
    fn the_slot_label_before_the_first_exterior_frame_is_a_dash() {
        assert_eq!(weather_slot_label(None), "Weather slot: -");
    }

    fn moment(moon: Option<MoonSky>) -> SkyMoment {
        let sun = SkyPosition {
            azimuth_deg: 90.0,
            elevation_deg: 10.0,
        };
        SkyMoment {
            blend: blend(TimeSlot::Day, TimeSlot::Day, 0.0),
            sun,
            moon,
            light: LightSource {
                direction: sun.direction(),
                from_moon_weight: 0.0,
                strength: 1.0,
            },
            star_rotation: StarRotation {
                axis: glam::Vec3::Y,
                angle_rad: 0.0,
            },
        }
    }

    fn failed() -> AssetState {
        AssetState::Failed(AssetFailure {
            kind: AssetFailureKind::NotFound,
            message: "missing".to_owned(),
        })
    }

    #[test]
    fn the_sky_assets_label_before_anything_is_requested() {
        assert_eq!(
            sky_assets_label(&SkyAssetStatus::default()),
            "Sky assets: star texture not requested, star mesh not requested, sun not requested, sun glare not requested, moon not requested"
        );
    }

    #[test]
    fn the_sky_assets_label_counts_the_moon_phases_by_state() {
        let mut status = SkyAssetStatus {
            star_texture: AssetState::Loaded,
            star_mesh: failed(),
            sun_texture: AssetState::Loaded,
            sun_glare_texture: failed(),
            ..SkyAssetStatus::default()
        };
        for (index, state) in status.moon.iter_mut().enumerate() {
            *state = match index {
                0 => failed(),
                1 => AssetState::Placeholder,
                _ => AssetState::Loaded,
            };
        }
        assert_eq!(
            sky_assets_label(&status),
            "Sky assets: star texture loaded, star mesh failed, sun loaded, sun glare failed, moon 6 loaded, 1 placeholder, 1 failed"
        );
    }

    #[test]
    fn the_moon_label_names_the_phase() {
        let moon = MoonSky {
            kind: MoonKind::Masser,
            phase: MoonPhase::HalfWax,
            position: SkyPosition {
                azimuth_deg: 270.0,
                elevation_deg: 30.0,
            },
            visibility: 1.0,
        };
        assert_eq!(moon_label(Some(&moment(Some(moon)))), "Moon: HalfWax");
    }

    #[test]
    fn the_moon_label_tells_a_climate_without_a_moon_from_no_data_yet() {
        assert_eq!(moon_label(Some(&moment(None))), "Moon: none");
        assert_eq!(moon_label(None), "Moon: -");
    }

    #[test]
    fn the_time_scale_names_the_length_of_a_game_day() {
        assert_eq!(format_time_scale(20.0), "20x (one game day = 1 h 12 min)");
        assert_eq!(format_time_scale(1.0), "1x (one game day = 24 h 0 min)");
        assert_eq!(format_time_scale(100.0), "100x (one game day = 14 min)");
        assert_eq!(format_time_scale(2000.0), "2000x (one game day = 43.2 s)");
        assert_eq!(format_time_scale(0.1), "0.1x (one game day = 240 h 0 min)");
    }
}
