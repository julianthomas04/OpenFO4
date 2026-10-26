//! The decisions of a roll: how long a weather stays settled before the next roll, which weather
//! the roll picks and how long the transition to a weather takes.

use glam::FloatExt;

use super::tuning::{
    BASE_ROLL_INTERVAL_HOURS, MAX_TRANSITION_HOURS, MIN_TRANSITION_HOURS,
    MOST_VOLATILE_INTERVAL_FRACTION, TRANSITION_BYTE_PER_HOUR,
};
use crate::random::RandomSource;
use crate::weather::{TableWeather, WeatherRollTable};

/// Game hours that a weather with `volatility_mult` stays settled in a climate of
/// `climate_volatility` before the next roll. The more volatile the climate and the weather, the
/// sooner.
pub(super) fn roll_interval_hours(climate_volatility: u8, volatility_mult: f32) -> f64 {
    let volatility = f64::from(climate_volatility) / f64::from(u8::MAX);
    let fraction = 1.0_f64.lerp(MOST_VOLATILE_INTERVAL_FRACTION, volatility);
    BASE_ROLL_INTERVAL_HOURS * fraction / f64::from(volatility_mult)
}

/// Game hours a transition to a weather with the transition byte `trans_delta_raw` takes.
pub(super) fn transition_hours(trans_delta_raw: u8) -> f64 {
    (f64::from(trans_delta_raw) / TRANSITION_BYTE_PER_HOUR)
        .clamp(MIN_TRANSITION_HOURS, MAX_TRANSITION_HOURS)
}

/// A weather drawn by the chances of the climate. The current weather is among the candidates, so
/// a roll can keep it.
pub(super) fn pick_weighted(table: &WeatherRollTable, rng: &mut dyn RandomSource) -> TableWeather {
    table.pick(rng.below(table.total_chance()))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::super::test_support::{A, ScriptedRandom, calm, table};
    use super::*;
    use crate::random::SplitMix64;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-4,
            "{actual} is not {expected}"
        );
    }

    #[test]
    fn without_volatility_a_settled_weather_stays_for_the_base_interval() {
        assert_close(roll_interval_hours(0, 1.0), 8.0);
    }

    #[test]
    fn the_highest_volatility_byte_leaves_a_quarter_of_the_interval() {
        assert_close(roll_interval_hours(255, 1.0), 2.0);
    }

    #[test]
    fn the_volatility_multiplier_divides_the_interval() {
        assert_close(roll_interval_hours(0, 2.0), 4.0);
        assert_close(roll_interval_hours(255, 4.0), 0.5);
    }

    #[test]
    fn in_the_commonwealth_clear_weather_stays_about_four_hours_and_a_radstorm_about_one() {
        assert_close(roll_interval_hours(175, 1.0), 3.8824);
        assert_close(roll_interval_hours(175, 4.0), 0.9706);
    }

    proptest! {
        #[test]
        fn a_more_volatile_climate_or_weather_never_rolls_later(
            volatility in 0_u8..=255,
            more_volatility in 0_u8..=255,
            mult in 0.1_f32..10.0,
            more_mult in 0.1_f32..10.0,
        ) {
            let (low, high) = (volatility.min(more_volatility), volatility.max(more_volatility));
            prop_assert!(roll_interval_hours(high, mult) <= roll_interval_hours(low, mult));
            let (low, high) = (mult.min(more_mult), mult.max(more_mult));
            prop_assert!(roll_interval_hours(volatility, high) <= roll_interval_hours(volatility, low));
        }
    }

    #[test]
    fn a_transition_takes_the_byte_over_sixteen_hours_within_its_limits() {
        let hours: Vec<f64> = [0, 4, 16, 25, 48, 125, 255]
            .into_iter()
            .map(transition_hours)
            .collect();
        assert_eq!(hours, [0.25, 0.25, 1.0, 1.5625, 3.0, 3.0, 3.0]);
    }

    fn commonwealth() -> WeatherRollTable {
        let chances = [51, 13, 11, 6, 6, 6, 5, 4];
        table(175, vec![calm(); chances.len()], &chances)
    }

    #[test]
    fn a_roll_picks_the_weather_of_the_ticket_it_draws() {
        let mut rng = ScriptedRandom::new(&[63], &[]);
        assert_eq!(pick_weighted(&commonwealth(), &mut rng).id().index(), 1);
    }

    #[test]
    fn the_draw_covers_exactly_the_sum_of_the_chances() {
        let mut rng = ScriptedRandom::new(&[0], &[]);
        pick_weighted(&commonwealth(), &mut rng);
        assert_eq!(rng.asked_below(), [102]);
    }

    #[test]
    fn picks_follow_the_chances_over_many_rolls() {
        let table = table(0, vec![calm(); 2], &[3, 1]);
        let mut rng = SplitMix64::new(7);
        let rolls = 20_000;
        let first = (0..rolls)
            .filter(|_| pick_weighted(&table, &mut rng).id() == A)
            .count();
        let share = first as f32 / rolls as f32;
        assert!((share - 0.75).abs() < 0.01, "{share}");
    }
}
