/// How far into a transition an effect that comes and goes with the weather, such as
/// precipitation or thunder, fades in and out, as the game stores it. Both values are fractions
/// of the transition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FadeRange {
    /// Where in the transition to the weather the effect starts to fade in.
    pub begin_fade_in: f32,
    /// Where in the transition away from the weather the effect has faded out.
    pub end_fade_out: f32,
}

/// The part a weather plays in a transition. `progress` is how far the transition has come,
/// `0..=1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransitionSide {
    /// Nothing is changing.
    Settled,
    /// The weather the transition leads to.
    Incoming { progress: f32 },
    /// The weather the transition leaves.
    Outgoing { progress: f32 },
}

impl TransitionSide {
    /// How much of the world the weather is: all of it while settled, as much as the transition
    /// has come while it comes in and as much as is left of it while it goes out.
    pub fn weight(self) -> f32 {
        match self {
            Self::Settled => 1.0,
            Self::Incoming { progress } => progress,
            Self::Outgoing { progress } => 1.0 - progress,
        }
    }
}

impl FadeRange {
    /// How strongly the effect shows on `side`, `0..=1`. A settled weather shows it fully
    /// whatever its range. Fractions outside `0..=1` are clamped and NaN counts as 0, so the
    /// result is always a number.
    pub fn intensity(self, side: TransitionSide) -> f32 {
        match side {
            TransitionSide::Settled => 1.0,
            TransitionSide::Incoming { progress } => self.fade_in(progress),
            TransitionSide::Outgoing { progress } => self.fade_out(progress),
        }
    }

    /// Ramps from 0 at `begin_fade_in` to 1 at the end of the transition. An effect that begins
    /// at the end of the transition or later shows only once the transition is complete.
    fn fade_in(self, progress: f32) -> f32 {
        let progress = fraction(progress);
        let begin = fraction(self.begin_fade_in);
        if begin >= 1.0 {
            return if progress >= 1.0 { 1.0 } else { 0.0 };
        }
        ((progress - begin) / (1.0 - begin)).clamp(0.0, 1.0)
    }

    /// Falls from 1 at the start of the transition to 0 at `end_fade_out`. An effect that ends
    /// at the start of the transition or earlier is gone as soon as the transition has begun.
    fn fade_out(self, progress: f32) -> f32 {
        let progress = fraction(progress);
        let end = fraction(self.end_fade_out);
        if end <= 0.0 {
            return if progress <= 0.0 { 1.0 } else { 0.0 };
        }
        1.0 - (progress / end).clamp(0.0, 1.0)
    }
}

/// `value` held to `0..=1`; NaN counts as 0.
pub fn fraction(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the expected intensities are exactly representable"
)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn range(begin_fade_in: f32, end_fade_out: f32) -> FadeRange {
        FadeRange {
            begin_fade_in,
            end_fade_out,
        }
    }

    fn incoming(range: FadeRange, progress: f32) -> f32 {
        range.intensity(TransitionSide::Incoming { progress })
    }

    fn outgoing(range: FadeRange, progress: f32) -> f32 {
        range.intensity(TransitionSide::Outgoing { progress })
    }

    #[test]
    fn a_settled_weather_is_all_of_the_world() {
        assert_eq!(TransitionSide::Settled.weight(), 1.0);
    }

    #[test]
    fn a_weather_is_as_much_of_the_world_as_the_transition_has_brought_it_in_or_left_of_it() {
        let weights = |progress| {
            (
                TransitionSide::Incoming { progress }.weight(),
                TransitionSide::Outgoing { progress }.weight(),
            )
        };
        assert_eq!(weights(0.0), (0.0, 1.0));
        assert_eq!(weights(0.25), (0.25, 0.75));
        assert_eq!(weights(1.0), (1.0, 0.0));
    }

    #[test]
    fn a_settled_weather_shows_the_effect_fully_whatever_its_range() {
        for range in [
            range(0.0, 0.0),
            range(0.5, 1.0),
            range(1.0, 1.0),
            range(-1.0, 2.0),
        ] {
            assert_eq!(range.intensity(TransitionSide::Settled), 1.0, "{range:?}");
        }
    }

    #[test]
    fn an_incoming_effect_ramps_from_its_begin_point_to_the_end_of_the_transition() {
        let range = range(0.5, 1.0);
        let ramp: Vec<f32> = [0.0, 0.5, 0.75, 1.0]
            .into_iter()
            .map(|progress| incoming(range, progress))
            .collect();
        assert_eq!(ramp, [0.0, 0.0, 0.5, 1.0]);
    }

    #[test]
    fn an_incoming_effect_without_a_begin_point_follows_the_progress() {
        assert_eq!(incoming(range(0.0, 1.0), 0.25), 0.25);
    }

    #[test]
    fn an_incoming_effect_that_begins_at_the_end_shows_only_once_the_transition_is_complete() {
        for begin in [1.0, 1.5] {
            let range = range(begin, 1.0);
            assert_eq!(incoming(range, 0.0), 0.0, "{begin}");
            assert_eq!(incoming(range, 0.99), 0.0, "{begin}");
            assert_eq!(incoming(range, 1.0), 1.0, "{begin}");
        }
    }

    #[test]
    fn an_outgoing_effect_falls_to_zero_at_its_end_point() {
        let range = range(0.0, 0.5);
        let fall: Vec<f32> = [0.0, 0.25, 0.5, 0.75]
            .into_iter()
            .map(|progress| outgoing(range, progress))
            .collect();
        assert_eq!(fall, [1.0, 0.5, 0.0, 0.0]);
    }

    #[test]
    fn an_outgoing_effect_that_ends_with_the_transition_falls_with_the_progress() {
        assert_eq!(outgoing(range(0.0, 1.0), 0.25), 0.75);
    }

    #[test]
    fn an_outgoing_effect_that_ends_at_the_start_is_gone_as_soon_as_the_transition_runs() {
        for end in [0.0, -0.5] {
            let range = range(0.0, end);
            assert_eq!(outgoing(range, 0.0), 1.0, "{end}");
            assert_eq!(outgoing(range, 0.001), 0.0, "{end}");
        }
    }

    #[test]
    fn the_degenerate_zero_range_of_the_game_data_gives_numbers() {
        let range = range(0.0, 0.0);
        assert_eq!(incoming(range, 0.5), 0.5);
        assert_eq!(outgoing(range, 0.0), 1.0);
        assert_eq!(outgoing(range, 0.5), 0.0);
    }

    #[test]
    fn a_progress_outside_the_transition_is_clamped_to_its_ends() {
        let range = range(0.5, 0.5);
        assert_eq!(incoming(range, -1.0), 0.0);
        assert_eq!(incoming(range, 2.0), 1.0);
        assert_eq!(outgoing(range, -1.0), 1.0);
        assert_eq!(outgoing(range, 2.0), 0.0);
    }

    #[test]
    fn a_nan_progress_counts_as_the_start_of_the_transition() {
        let range = range(0.2, 0.8);
        assert_eq!(incoming(range, f32::NAN), 0.0);
        assert_eq!(outgoing(range, f32::NAN), 1.0);
    }

    #[test]
    fn a_fraction_keeps_a_value_in_range_and_holds_the_others_to_the_ends() {
        assert_eq!(fraction(0.25), 0.25);
        assert_eq!(fraction(0.0), 0.0);
        assert_eq!(fraction(1.0), 1.0);
        assert_eq!(fraction(-3.0), 0.0);
        assert_eq!(fraction(7.0), 1.0);
        assert_eq!(fraction(f32::NEG_INFINITY), 0.0);
        assert_eq!(fraction(f32::INFINITY), 1.0);
    }

    #[test]
    fn a_fraction_that_is_not_a_number_counts_as_zero() {
        assert_eq!(fraction(f32::NAN), 0.0);
    }

    fn any_fraction() -> impl Strategy<Value = f32> {
        prop_oneof![
            Just(f32::NAN),
            Just(f32::INFINITY),
            Just(f32::NEG_INFINITY),
            any::<f32>(),
            -0.5_f32..1.5,
        ]
    }

    proptest! {
        #[test]
        fn the_intensity_is_always_a_number_from_zero_to_one(
            begin in any_fraction(),
            end in any_fraction(),
            progress in any_fraction(),
        ) {
            let range = range(begin, end);
            for side in [
                TransitionSide::Settled,
                TransitionSide::Incoming { progress },
                TransitionSide::Outgoing { progress },
            ] {
                let intensity = range.intensity(side);
                prop_assert!((0.0..=1.0).contains(&intensity), "{side:?} gave {intensity}");
            }
        }

        #[test]
        fn a_transition_starts_with_the_old_weather_alone_and_ends_with_the_new_one_alone(
            begin in any_fraction(),
            end in any_fraction(),
        ) {
            let range = range(begin, end);
            prop_assert_eq!(incoming(range, 0.0), 0.0);
            prop_assert_eq!(outgoing(range, 0.0), 1.0);
            prop_assert_eq!(incoming(range, 1.0), 1.0);
            prop_assert_eq!(outgoing(range, 1.0), 0.0);
        }

        #[test]
        fn the_incoming_effect_never_falls_and_the_outgoing_one_never_rises(
            begin in 0.0_f32..=1.0,
            end in 0.0_f32..=1.0,
            a in 0.0_f32..=1.0,
            b in 0.0_f32..=1.0,
        ) {
            let range = range(begin, end);
            let (earlier, later) = if a <= b { (a, b) } else { (b, a) };
            prop_assert!(incoming(range, earlier) <= incoming(range, later));
            prop_assert!(outgoing(range, earlier) >= outgoing(range, later));
        }
    }
}
