use super::climate_times::ClimateTimes;
use super::day_time::DayTime;
use super::keyframes::{KEYFRAMES, keyframe_hours};
use super::slot::TimeSlot;
use super::wrap::{HOURS_PER_DAY, wrap_hours};

/// Where an hour sits between two neighbouring keyframes: `t` is 0 at `from`
/// and 1 at `to`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlotBlend {
    pub from: TimeSlot,
    pub to: TimeSlot,
    pub t: f32,
}

impl SlotBlend {
    /// The share `slot` takes in the blend: `1 - t` as `from`, `t` as `to`, 0 for any other slot.
    pub fn weight_of(&self, slot: TimeSlot) -> f32 {
        let from = if self.from == slot { 1.0 - self.t } else { 0.0 };
        let to = if self.to == slot { self.t } else { 0.0 };
        from + to
    }
}

/// The slots surrounding a moment of the day. The timeline is a ring: after the
/// last keyframe it continues into the first of the next day.
pub fn slot_blend(time: DayTime, times: &ClimateTimes) -> SlotBlend {
    let keyframe_hours = keyframe_hours(times);
    let first_hour = keyframe_hours[0];
    let hour = first_hour + wrap_hours(time.hour() - first_hour);
    let from_index = keyframe_hours
        .partition_point(|keyframe_hour| *keyframe_hour <= hour)
        .saturating_sub(1);
    let to_index = (from_index + 1) % KEYFRAMES.len();
    let from_hour = keyframe_hours[from_index];
    let to_hour = if to_index == 0 {
        first_hour + HOURS_PER_DAY
    } else {
        keyframe_hours[to_index]
    };
    let span = to_hour - from_hour;
    let t = if span > 0.0 {
        ((hour - from_hour) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    SlotBlend {
        from: KEYFRAMES[from_index].slot,
        to: KEYFRAMES[to_index].slot,
        t,
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact binary fractions of an hour"
)]
mod tests {
    use proptest::prelude::*;

    use super::super::test_support::{at, at_hour, default_climate};
    use super::*;
    use TimeSlot::*;

    fn slot_blend(hour: f32, times: &ClimateTimes) -> SlotBlend {
        super::slot_blend(at_hour(hour), times)
    }

    #[test]
    fn a_slot_weighs_one_minus_t_as_from_t_as_to_and_nothing_otherwise() {
        let blend = SlotBlend {
            from: LateSunset,
            to: Night,
            t: 0.25,
        };
        assert_eq!(blend.weight_of(LateSunset), 0.75);
        assert_eq!(blend.weight_of(Night), 0.25);
        assert_eq!(blend.weight_of(Day), 0.0);
        let held = SlotBlend {
            from: Night,
            to: Night,
            t: 0.25,
        };
        assert_eq!(held.weight_of(Night), 1.0);
    }

    const SLOTS_BY_KEYFRAME: [(TimeSlot, TimeSlot); 10] = [
        (Night, EarlySunrise),
        (EarlySunrise, Sunrise),
        (Sunrise, LateSunrise),
        (LateSunrise, Day),
        (Day, Day),
        (Day, EarlySunset),
        (EarlySunset, Sunset),
        (Sunset, LateSunset),
        (LateSunset, Night),
        (Night, Night),
    ];

    const VANILLA_CLIMATES: [([f32; 4], [f32; 10]); 3] = [
        (
            [5.0, 9.0, 17.0, 21.0],
            [3.0, 4.5, 6.0, 7.5, 9.0, 17.0, 18.5, 20.0, 21.5, 23.0],
        ),
        (
            [5.5, 8.0, 16.0, 20.5],
            [
                3.5, 4.625, 5.75, 6.875, 8.0, 16.0, 17.625, 19.25, 20.875, 22.5,
            ],
        ),
        (
            [4.0, 10.0, 16.0, 22.0],
            [2.0, 4.0, 6.0, 8.0, 10.0, 16.0, 18.0, 20.0, 22.0, 24.0],
        ),
    ];

    fn climate([a, b, c, d]: [f32; 4]) -> ClimateTimes {
        ClimateTimes::new(a, b, c, d).unwrap()
    }

    #[test]
    fn at_every_keyframe_hour_of_every_vanilla_climate_the_blend_starts_that_keyframe() {
        for (bounds, keyframe_hours) in VANILLA_CLIMATES {
            let times = climate(bounds);
            for (hour, (from, to)) in keyframe_hours.into_iter().zip(SLOTS_BY_KEYFRAME) {
                let blend = slot_blend(hour.rem_euclid(HOURS_PER_DAY), &times);
                assert_eq!((blend.from, blend.to), (from, to), "{bounds:?} at {hour}");
                assert_eq!(blend.t, 0.0, "{bounds:?} at {hour}");
            }
        }
    }

    #[test]
    fn halfway_between_keyframes_the_blend_is_half() {
        let times = default_climate();
        for (hour, from, to) in [
            (3.75, Night, EarlySunrise),
            (5.25, EarlySunrise, Sunrise),
            (6.75, Sunrise, LateSunrise),
            (8.25, LateSunrise, Day),
            (17.75, Day, EarlySunset),
            (19.25, EarlySunset, Sunset),
            (20.75, Sunset, LateSunset),
            (22.25, LateSunset, Night),
        ] {
            assert_eq!(
                slot_blend(hour, &times),
                SlotBlend { from, to, t: 0.5 },
                "{hour}"
            );
        }
    }

    /// The share of the later slot the probe solved at the hours between two pure slots, from the
    /// fog's fully fogged boards and the sky alike (`results_v10`).
    #[test]
    fn the_blend_follows_the_slot_shares_measured_under_the_default_climate() {
        let times = default_climate();
        for (hour, from, to, measured) in [
            (6.5, Sunrise, LateSunrise, 0.32),
            (7.0, Sunrise, LateSunrise, 0.67),
            (8.0, LateSunrise, Day, 0.34),
            (8.5, LateSunrise, Day, 0.71),
            (17.5, Day, EarlySunset, 0.33),
            (18.0, Day, EarlySunset, 0.67),
            (20.5, Sunset, LateSunset, 0.36),
            (21.0, Sunset, LateSunset, 0.71),
        ] {
            let blend = slot_blend(hour, &times);
            assert_eq!((blend.from, blend.to), (from, to), "{hour}");
            assert!((blend.t - measured).abs() < 0.05, "{hour}: {}", blend.t);
        }
    }

    #[test]
    fn the_blend_holds_day_through_the_afternoon_and_night_around_midnight() {
        let times = default_climate();
        for hour in [9.5, 12.0, 16.9] {
            let blend = slot_blend(hour, &times);
            assert_eq!((blend.from, blend.to), (Day, Day), "{hour}");
        }
        for hour in [23.5, 0.0, 2.0, 2.9] {
            let blend = slot_blend(hour, &times);
            assert_eq!((blend.from, blend.to), (Night, Night), "{hour}");
        }
    }

    #[test]
    fn the_night_to_night_span_wraps_over_midnight() {
        let times = default_climate();
        let late = slot_blend(23.5, &times);
        let early = slot_blend(1.0, &times);
        assert_eq!((late.from, late.to), (Night, Night));
        assert_eq!((early.from, early.to), (Night, Night));
        assert_eq!(late.t, 0.125);
        assert_eq!(early.t, 0.5);
        assert!(slot_blend(2.999, &times).t > 0.99);
    }

    #[test]
    fn the_blend_is_the_same_just_before_and_after_midnight() {
        let times = default_climate();
        let before = slot_blend(23.9999, &times);
        let after = slot_blend(0.0, &times);
        assert_eq!((before.from, before.to), (after.from, after.to));
        assert!((before.t - after.t).abs() < 1e-4);
    }

    #[test]
    fn a_first_keyframe_before_midnight_still_wraps_the_night() {
        let times = ClimateTimes::new(1.0, 4.0, 15.0, 18.0).unwrap();
        assert_eq!(
            slot_blend(21.5, &times),
            SlotBlend {
                from: Night,
                to: Night,
                t: 0.5
            }
        );
        let blend = slot_blend(23.5, &times);
        assert_eq!((blend.from, blend.to), (Night, EarlySunrise));
        assert_eq!(blend.t, 0.4);
    }

    #[test]
    fn the_blend_does_not_depend_on_the_day_count() {
        let times = default_climate();
        for hour in [0.0, 3.3, 6.25, 12.0, 18.75, 23.5] {
            assert_eq!(
                super::slot_blend(at(0, hour), &times),
                super::slot_blend(at(900, hour), &times)
            );
        }
    }

    #[test]
    fn the_last_instant_of_the_day_is_still_in_the_final_night_span() {
        let blend = slot_blend(23.999_998, &default_climate());
        assert_eq!((blend.from, blend.to), (Night, Night));
        assert!((0.0..=1.0).contains(&blend.t));
    }

    #[test]
    fn a_zero_length_day_span_does_not_divide_by_zero() {
        let times = ClimateTimes::new(5.0, 12.0, 12.0, 21.0).unwrap();
        let blend = slot_blend(12.0, &times);
        assert_eq!((blend.from, blend.to, blend.t), (Day, EarlySunset, 0.0));
        let blend = slot_blend(11.999, &times);
        assert_eq!((blend.from, blend.to), (LateSunrise, Day));
        assert!(blend.t > 0.99);
    }

    fn valid_climate() -> impl Strategy<Value = ClimateTimes> {
        prop::collection::vec(0.0f32..24.0, 4).prop_filter_map(
            "windows at least half an hour wide and ordered",
            |mut bounds| {
                bounds.sort_by(f32::total_cmp);
                let wide = bounds[1] - bounds[0] >= 0.5 && bounds[3] - bounds[2] >= 0.5;
                if !wide {
                    return None;
                }
                ClimateTimes::new(bounds[0], bounds[1], bounds[2], bounds[3]).ok()
            },
        )
    }

    proptest! {
        #[test]
        fn t_stays_within_zero_and_one_for_any_hour_and_climate(
            hour in 0.0f32..24.0,
            times in valid_climate(),
        ) {
            let blend = slot_blend(hour, &times);
            prop_assert!((0.0..=1.0).contains(&blend.t), "{blend:?}");
        }
    }
}
