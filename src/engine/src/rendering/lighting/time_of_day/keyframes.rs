//! When each time-of-day slot is pure, as the probe measured it on the fog and sky slot colours
//! under two climates, `DefaultClimate` (sunrise 05:00-09:00, sunset 17:00-21:00) and one with
//! sunrise 03:00-05:00 and sunset 15:00-21:00. `Day` holds from sunrise end to sunset begin, and
//! the other slots step away from it, each window in four equal steps that split its length plus
//! [`NIGHT_PADDING_HOURS`], so that `Night` lies that far outside the window: `LateSunrise`,
//! `Sunrise`, `EarlySunrise` and `Night` back from sunrise end, `EarlySunset`, `Sunset`,
//! `LateSunset` and `Night` forward from sunset begin. Neighbouring slots cross linearly between
//! keyframes, and the fog distances ramp from their Day half to their Night half between the `Day`
//! and `Night` keyframes ([`night_weight`]).

use super::climate_times::{ClimateTimes, ClimateWindow};
use super::slot::TimeSlot;
use super::wrap::{HOURS_PER_DAY, wrap_hours};

/// How far the `Night` keyframes lie before sunrise begin and after sunset end.
pub const NIGHT_PADDING_HOURS: f32 = 2.0;

/// Keyframe steps from the inner edge of a twilight window to its `Night` keyframe.
const NIGHT_STEPS: f32 = 4.0;

/// The hours from a window's `Day` keyframe to its `Night` keyframe.
fn night_ramp_hours(window: ClimateWindow, times: &ClimateTimes) -> f32 {
    let length = match window {
        ClimateWindow::Sunrise => times.sunrise_end() - times.sunrise_begin(),
        ClimateWindow::Sunset => times.sunset_end() - times.sunset_begin(),
    };
    length + NIGHT_PADDING_HOURS
}

/// A point on the day timeline: `steps` keyframe steps away from the inner edge of a twilight
/// window, back from sunrise end or forward from sunset begin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Keyframe {
    pub window: ClimateWindow,
    pub steps: f32,
    pub slot: TimeSlot,
}

impl Keyframe {
    fn hour(self, times: &ClimateTimes) -> f32 {
        let away = self.steps * night_ramp_hours(self.window, times) / NIGHT_STEPS;
        match self.window {
            ClimateWindow::Sunrise => times.sunrise_end() - away,
            ClimateWindow::Sunset => times.sunset_begin() + away,
        }
    }
}

const fn keyframe(window: ClimateWindow, steps: f32, slot: TimeSlot) -> Keyframe {
    Keyframe {
        window,
        steps,
        slot,
    }
}

/// The day keyframes in time order. The span from the last one back to the first wraps over
/// midnight; `Day` holds between sunrise end and sunset begin.
pub const KEYFRAMES: [Keyframe; 10] = [
    keyframe(ClimateWindow::Sunrise, NIGHT_STEPS, TimeSlot::Night),
    keyframe(ClimateWindow::Sunrise, 3.0, TimeSlot::EarlySunrise),
    keyframe(ClimateWindow::Sunrise, 2.0, TimeSlot::Sunrise),
    keyframe(ClimateWindow::Sunrise, 1.0, TimeSlot::LateSunrise),
    keyframe(ClimateWindow::Sunrise, 0.0, TimeSlot::Day),
    keyframe(ClimateWindow::Sunset, 0.0, TimeSlot::Day),
    keyframe(ClimateWindow::Sunset, 1.0, TimeSlot::EarlySunset),
    keyframe(ClimateWindow::Sunset, 2.0, TimeSlot::Sunset),
    keyframe(ClimateWindow::Sunset, 3.0, TimeSlot::LateSunset),
    keyframe(ClimateWindow::Sunset, NIGHT_STEPS, TimeSlot::Night),
];

/// The hour of every keyframe, ascending from the first and at most a day after it: the evening
/// `Night` keyframe reaches the next morning's only on the shortest night [`ClimateTimes`]
/// allows.
pub fn keyframe_hours(times: &ClimateTimes) -> [f32; KEYFRAMES.len()] {
    KEYFRAMES.map(|keyframe| keyframe.hour(times))
}

/// How far the day has turned from its `Day` keyframes to its `Night` keyframes: 0 from sunrise
/// end to sunset begin, 1 between the evening and the next morning `Night` keyframe, and linear
/// in between.
pub fn night_weight(hour: f32, times: &ClimateTimes) -> f32 {
    let since_sunset_begin = wrap_hours(hour - times.sunset_begin());
    let until_sunrise_end = HOURS_PER_DAY - (times.sunset_begin() - times.sunrise_end());
    if since_sunset_begin >= until_sunrise_end {
        return 0.0;
    }
    let evening = since_sunset_begin / night_ramp_hours(ClimateWindow::Sunset, times);
    let morning =
        (until_sunrise_end - since_sunset_begin) / night_ramp_hours(ClimateWindow::Sunrise, times);
    evening.min(morning).min(1.0)
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact binary fractions of an hour"
)]
mod tests {
    use proptest::prelude::*;

    use super::super::test_support::default_climate;
    use super::*;

    fn second_climate() -> ClimateTimes {
        ClimateTimes::new(3.0, 5.0, 15.0, 21.0).unwrap()
    }

    #[test]
    fn the_keyframe_table_covers_every_slot() {
        for slot in TimeSlot::ALL {
            assert!(KEYFRAMES.iter().any(|k| k.slot == slot), "{slot:?}");
        }
    }

    #[test]
    fn under_the_default_climate_the_keyframes_sit_where_the_probe_saw_pure_slots() {
        assert_eq!(
            keyframe_hours(&default_climate()),
            [3.0, 4.5, 6.0, 7.5, 9.0, 17.0, 18.5, 20.0, 21.5, 23.0]
        );
    }

    #[test]
    fn each_window_splits_its_length_plus_the_padding_into_four_steps() {
        // Sunrise 03-05 steps (2 + 2) / 4 = 1 h, sunset 15-21 steps (6 + 2) / 4 = 2 h.
        assert_eq!(
            keyframe_hours(&second_climate()),
            [1.0, 2.0, 3.0, 4.0, 5.0, 15.0, 17.0, 19.0, 21.0, 23.0]
        );
    }

    #[test]
    fn night_lies_the_padding_outside_both_windows() {
        let times = ClimateTimes::new(5.5, 8.0, 16.0, 20.5).unwrap();
        let hours = keyframe_hours(&times);
        assert_eq!(hours[0], 5.5 - NIGHT_PADDING_HOURS);
        assert_eq!(hours[9], 20.5 + NIGHT_PADDING_HOURS);
    }

    #[test]
    fn an_early_sunrise_puts_the_first_keyframe_before_midnight() {
        let times = ClimateTimes::new(1.0, 4.0, 15.0, 18.0).unwrap();
        let hours = keyframe_hours(&times);
        assert_eq!(hours[0], -1.0);
        assert_eq!(hours[9], 20.0);
    }

    #[test]
    fn on_the_shortest_night_allowed_the_night_keyframes_meet() {
        let times = ClimateTimes::new(2.0, 3.0, 20.0, 22.0).unwrap();
        let hours = keyframe_hours(&times);
        assert_eq!(hours[9] - hours[0], HOURS_PER_DAY);
    }

    /// `results_v13`, "Day and night halves of FNAM": the fitted night weight under both climates.
    #[test]
    fn the_night_weight_follows_the_fog_halves_measured_under_both_climates() {
        let default = default_climate();
        let second = second_climate();
        let measured = [
            (&default, 0.0, 1.000),
            (&default, 3.0, 0.991),
            (&default, 4.0, 0.822),
            (&default, 5.0, 0.648),
            (&default, 5.5, 0.580),
            (&default, 6.0, 0.511),
            (&default, 6.5, 0.422),
            (&default, 7.0, 0.355),
            (&default, 8.0, 0.169),
            (&default, 12.0, 0.000),
            (&default, 18.0, 0.153),
            (&default, 19.0, 0.353),
            (&default, 20.0, 0.516),
            (&default, 21.0, 0.632),
            (&default, 22.0, 0.828),
            (&default, 23.0, 1.000),
            (&second, 1.0, 0.987),
            (&second, 2.5, 0.600),
            (&second, 3.5, 0.388),
            (&second, 4.25, 0.172),
            (&second, 16.5, 0.202),
            (&second, 18.0, 0.381),
            (&second, 20.0, 0.613),
            (&second, 22.5, 0.925),
        ];
        for (times, hour, expected) in measured {
            let weight = night_weight(hour, times);
            assert!(
                (weight - expected).abs() < 0.04,
                "{times:?} {hour}: {weight}"
            );
        }
    }

    #[test]
    fn the_night_weight_is_0_by_day_and_1_between_the_night_keyframes() {
        let times = second_climate();
        for hour in [5.0, 10.0, 15.0] {
            assert_eq!(night_weight(hour, &times), 0.0, "{hour}");
        }
        for hour in [23.0, 0.0, 1.0] {
            assert_eq!(night_weight(hour, &times), 1.0, "{hour}");
        }
        assert_eq!(night_weight(3.0, &times), 0.5);
        assert_eq!(night_weight(19.0, &times), 0.5);
    }

    #[test]
    fn the_night_weight_wraps_a_morning_ramp_that_starts_before_midnight() {
        let times = ClimateTimes::new(1.0, 4.0, 15.0, 18.0).unwrap();
        assert!((night_weight(23.5, &times) - 0.9).abs() < 1e-6);
        assert_eq!(night_weight(20.0, &times), 1.0);
    }

    #[test]
    fn with_no_day_between_the_windows_only_that_instant_is_all_day() {
        let times = ClimateTimes::new(5.0, 12.0, 12.0, 21.0).unwrap();
        assert_eq!(night_weight(12.0, &times), 0.0);
        assert!(night_weight(11.9, &times) > 0.0);
        assert!(night_weight(12.1, &times) > 0.0);
    }

    fn valid_climate() -> impl Strategy<Value = ClimateTimes> {
        prop::collection::vec(0.0f32..24.0, 4).prop_filter_map("ordered and valid", |mut bounds| {
            bounds.sort_by(f32::total_cmp);
            ClimateTimes::new(bounds[0], bounds[1], bounds[2], bounds[3]).ok()
        })
    }

    proptest! {
        #[test]
        fn the_keyframes_ascend_within_one_day_for_any_climate(times in valid_climate()) {
            let hours = keyframe_hours(&times);
            prop_assert!(hours.windows(2).all(|pair| pair[0] <= pair[1]), "{hours:?}");
            prop_assert!(hours[9] - hours[0] <= HOURS_PER_DAY + 1e-4, "{hours:?}");
        }

        #[test]
        fn the_night_weight_stays_within_zero_and_one_and_moves_steadily(
            hour in 0.0f32..23.99,
            times in valid_climate(),
        ) {
            let weight = night_weight(hour, &times);
            prop_assert!((0.0..=1.0).contains(&weight), "{weight}");
            let next = night_weight(hour + 1e-3, &times);
            prop_assert!((next - weight).abs() <= 1e-3 / NIGHT_PADDING_HOURS + 1e-4, "{weight} -> {next}");
        }
    }
}
