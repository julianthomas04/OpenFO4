use super::climate_times::ClimateTimes;
use super::day_time::DayTime;
use super::slot::TimeSlot;
use super::unmeasured::KEYFRAMES;
use super::wrap::HOURS_PER_DAY;

/// Where an hour sits between two neighbouring keyframes: `t` is 0 at `from`
/// and 1 at `to`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlotBlend {
    pub from: TimeSlot,
    pub to: TimeSlot,
    pub t: f32,
}

/// The slots surrounding a moment of the day. The timeline is a ring: after the
/// last keyframe it continues into the first of the next day.
pub fn slot_blend(time: DayTime, times: &ClimateTimes) -> SlotBlend {
    let keyframe_hours = KEYFRAMES.map(|keyframe| keyframe.hour(times));
    let first_hour = keyframe_hours[0];
    let mut hour = time.hour();
    if hour < first_hour {
        hour += HOURS_PER_DAY;
    }
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
            [5.0, 6.0, 7.0, 8.0, 9.0, 17.0, 18.0, 19.0, 20.0, 21.0],
        ),
        (
            [5.5, 8.0, 16.0, 20.5],
            [
                5.5, 6.125, 6.75, 7.375, 8.0, 16.0, 17.125, 18.25, 19.375, 20.5,
            ],
        ),
        (
            [4.0, 10.0, 16.0, 22.0],
            [4.0, 5.5, 7.0, 8.5, 10.0, 16.0, 17.5, 19.0, 20.5, 22.0],
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
                let blend = slot_blend(hour, &times);
                assert_eq!((blend.from, blend.to), (from, to), "{bounds:?} at {hour}");
                assert_eq!(blend.t, 0.0, "{bounds:?} at {hour}");
            }
        }
    }

    #[test]
    fn halfway_between_keyframes_the_blend_is_half() {
        let times = default_climate();
        for (hour, from, to) in [
            (5.5, Night, EarlySunrise),
            (6.5, EarlySunrise, Sunrise),
            (7.5, Sunrise, LateSunrise),
            (8.5, LateSunrise, Day),
            (17.5, Day, EarlySunset),
            (18.5, EarlySunset, Sunset),
            (19.5, Sunset, LateSunset),
            (20.5, LateSunset, Night),
        ] {
            assert_eq!(
                slot_blend(hour, &times),
                SlotBlend { from, to, t: 0.5 },
                "{hour}"
            );
        }
    }

    #[test]
    fn the_blend_holds_day_through_the_afternoon_and_night_through_the_small_hours() {
        let times = default_climate();
        for hour in [9.5, 12.0, 16.9] {
            let blend = slot_blend(hour, &times);
            assert_eq!((blend.from, blend.to), (Day, Day), "{hour}");
        }
        for hour in [21.5, 23.0, 0.0, 2.0, 4.9] {
            let blend = slot_blend(hour, &times);
            assert_eq!((blend.from, blend.to), (Night, Night), "{hour}");
        }
    }

    #[test]
    fn the_night_to_night_span_wraps_over_midnight() {
        let times = default_climate();
        let late = slot_blend(23.0, &times);
        let early = slot_blend(1.0, &times);
        assert_eq!((late.from, late.to), (Night, Night));
        assert_eq!((early.from, early.to), (Night, Night));
        assert!(late.t > 0.0 && early.t > late.t, "{late:?} {early:?}");
        assert!((slot_blend(21.0, &times).t - 0.0).abs() < 1e-6);
        assert!(slot_blend(4.999, &times).t > 0.99);
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
