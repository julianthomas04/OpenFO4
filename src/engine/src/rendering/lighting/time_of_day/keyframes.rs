//! When each time-of-day slot is pure, as the probe measured it under `DefaultClimate`
//! (sunrise 05:00-09:00, sunset 17:00-21:00): fog and sky alike hold `EarlySunrise` at 04:30,
//! Sunrise at 06:00, `LateSunrise` at 07:30 and Day from 09:00 to 17:00, then `EarlySunset` at
//! 18:30, Sunset at 20:00 and `LateSunset` at about 21:30, and neighbouring slots cross linearly
//! between those keyframes.
//!
//! Only that one climate was measured, so the general rule is a reading: the keyframes step
//! [`SLOT_STEP_HOURS`] back from sunrise end and forward from sunset begin. Placing `Night` one step
//! beyond `EarlySunrise` and `LateSunset` (03:00 and 23:00) is an inference: the probe saw pure `Night`
//! at 00:00 only, and the step matches the 6 h night ramp of the fog distances.

use super::climate_times::{ClimateTimes, ClimateWindow};
use super::slot::TimeSlot;
use super::wrap::HOURS_PER_DAY;

/// The time between two neighbouring keyframes of a twilight.
pub const SLOT_STEP_HOURS: f32 = 1.5;

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
        match self.window {
            ClimateWindow::Sunrise => times.sunrise_end() - self.steps * SLOT_STEP_HOURS,
            ClimateWindow::Sunset => times.sunset_begin() + self.steps * SLOT_STEP_HOURS,
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
    keyframe(ClimateWindow::Sunrise, 4.0, TimeSlot::Night),
    keyframe(ClimateWindow::Sunrise, 3.0, TimeSlot::EarlySunrise),
    keyframe(ClimateWindow::Sunrise, 2.0, TimeSlot::Sunrise),
    keyframe(ClimateWindow::Sunrise, 1.0, TimeSlot::LateSunrise),
    keyframe(ClimateWindow::Sunrise, 0.0, TimeSlot::Day),
    keyframe(ClimateWindow::Sunset, 0.0, TimeSlot::Day),
    keyframe(ClimateWindow::Sunset, 1.0, TimeSlot::EarlySunset),
    keyframe(ClimateWindow::Sunset, 2.0, TimeSlot::Sunset),
    keyframe(ClimateWindow::Sunset, 3.0, TimeSlot::LateSunset),
    keyframe(ClimateWindow::Sunset, 4.0, TimeSlot::Night),
];

/// The hour of every keyframe, ascending from the first and less than a day after it. Where a
/// long day would push the evening keyframes past the next morning's, they stop at the first
/// keyframe of the next day, so the night between them has no length.
pub fn keyframe_hours(times: &ClimateTimes) -> [f32; KEYFRAMES.len()] {
    let mut hours = KEYFRAMES.map(|keyframe| keyframe.hour(times));
    let next_day = hours[0] + HOURS_PER_DAY;
    for index in 1..hours.len() {
        hours[index] = hours[index].clamp(hours[index - 1], next_day);
    }
    hours
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact binary fractions of an hour"
)]
mod tests {
    use super::super::test_support::default_climate;
    use super::*;

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
    fn the_keyframes_follow_sunrise_end_and_sunset_begin() {
        let times = ClimateTimes::new(5.5, 8.0, 16.0, 20.5).unwrap();
        assert_eq!(
            keyframe_hours(&times),
            [2.0, 3.5, 5.0, 6.5, 8.0, 16.0, 17.5, 19.0, 20.5, 22.0]
        );
    }

    #[test]
    fn an_early_sunrise_puts_the_first_keyframe_before_midnight() {
        let times = ClimateTimes::new(1.0, 4.0, 15.0, 18.0).unwrap();
        let hours = keyframe_hours(&times);
        assert_eq!(hours[0], -2.0);
        assert_eq!(hours[9], 21.0);
    }

    #[test]
    fn a_day_too_long_for_the_keyframes_ends_its_night_at_the_next_morning() {
        let times = ClimateTimes::new(2.0, 3.0, 20.0, 23.0).unwrap();
        let hours = keyframe_hours(&times);
        assert_eq!(hours[0], -3.0);
        assert_eq!(hours[8], 21.0);
        assert_eq!(hours[9], 21.0);
        assert!(hours.windows(2).all(|pair| pair[0] <= pair[1]), "{hours:?}");
    }
}
