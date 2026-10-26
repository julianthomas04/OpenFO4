//! The lightning of storms: when a strike comes, where it lands and how hard it flashes. Strikes
//! are scheduled in real seconds of world time, so a fast clock does not turn a storm into a
//! strobe.

use glam::FloatExt;

use super::tuning::{
    FARTHEST_FLASH_INTENSITY, FARTHEST_STRIKE_DISTANCE_M, MOST_FREQUENT_STRIKE_BYTE,
    MOST_FREQUENT_STRIKE_INTERVAL_SECONDS, NEAREST_STRIKE_DISTANCE_M, RAREST_STRIKE_BYTE,
    RAREST_STRIKE_INTERVAL_SECONDS, SPEED_OF_SOUND_M_PER_S,
};
use crate::random::RandomSource;
use crate::weather::{TransitionSide, WeatherId, WeatherRollTraits};

/// One strike of lightning: the renderer flashes it and audio plays its thunder after the delay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightningStrike {
    /// The weather whose lightning colour and thunder sounds the strike takes.
    pub weather: WeatherId,
    pub distance_m: f32,
    /// Real seconds from the flash to its thunder.
    pub thunder_delay_s: f32,
    /// `0..=1`; weaker for a far strike and for a weather whose thunder is fading.
    pub flash_intensity: f32,
}

/// A weather that strikes, on its side of a transition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct StrikeSource {
    pub weather: WeatherId,
    /// How strong the thunder of the side is, above 0.
    pub intensity: f32,
    /// The mean real seconds between strikes at full intensity.
    pub mean_interval_seconds: f32,
}

impl StrikeSource {
    /// Strikes per real second.
    fn rate(&self) -> f32 {
        self.intensity / self.mean_interval_seconds
    }
}

/// Which weathers strike is read off the data: one with a thunder sound is a storm. A storm
/// strikes while its thunder has faded in and has not faded out.
pub(super) fn strike_source(
    weather: WeatherId,
    traits: &WeatherRollTraits,
    side: TransitionSide,
) -> Option<StrikeSource> {
    if !traits.has_thunder_sound {
        return None;
    }
    let intensity = traits.lightning.fade.intensity(side);
    (intensity > 0.0).then(|| StrikeSource {
        weather,
        intensity,
        mean_interval_seconds: mean_strike_interval_seconds(traits.lightning.frequency_byte()),
    })
}

/// The mean real seconds between the strikes of a weather with the frequency byte
/// `frequency_byte` at full intensity. The game's byte runs the other way round: the lower, the
/// more frequent.
pub(super) fn mean_strike_interval_seconds(frequency_byte: u8) -> f32 {
    let span = f32::from(RAREST_STRIKE_BYTE - MOST_FREQUENT_STRIKE_BYTE);
    let frequent =
        ((f32::from(RAREST_STRIKE_BYTE) - f32::from(frequency_byte)) / span).clamp(0.0, 1.0);
    RAREST_STRIKE_INTERVAL_SECONDS.lerp(MOST_FREQUENT_STRIKE_INTERVAL_SECONDS, frequent)
}

/// A strike of `weather` at a random distance. It flashes at `intensity` times the strength its
/// distance allows.
pub(super) fn strike_at(
    weather: WeatherId,
    intensity: f32,
    rng: &mut dyn RandomSource,
) -> LightningStrike {
    let distance_m = NEAREST_STRIKE_DISTANCE_M.lerp(FARTHEST_STRIKE_DISTANCE_M, rng.unit());
    let farness = f32::inverse_lerp(
        NEAREST_STRIKE_DISTANCE_M,
        FARTHEST_STRIKE_DISTANCE_M,
        distance_m,
    )
    .clamp(0.0, 1.0);
    LightningStrike {
        weather,
        distance_m,
        thunder_delay_s: distance_m / SPEED_OF_SOUND_M_PER_S,
        flash_intensity: 1.0_f32.lerp(FARTHEST_FLASH_INTENSITY, farness) * intensity,
    }
}

/// Waits for the next strike. The wait is exponential, as for events that come at random at a
/// steady rate. It is counted in expected strikes, so the rate can change while it runs.
#[derive(Debug, Default)]
pub(super) struct StrikeClock {
    /// The expected strikes still to wait for; drawn when the wait starts.
    wait: Option<f32>,
}

impl StrikeClock {
    /// Lets `seconds` of real time pass while the weathers in `sources` strike, and returns the
    /// strike that fell in that time. At most one strike comes per call.
    pub fn advance(
        &mut self,
        sources: &[StrikeSource],
        seconds: f32,
        rng: &mut dyn RandomSource,
    ) -> Option<LightningStrike> {
        let rate: f32 = sources.iter().map(StrikeSource::rate).sum();
        if !(rate > 0.0 && seconds > 0.0) {
            return None;
        }
        let wait = self.wait.get_or_insert_with(|| exponential(rng.unit()));
        *wait -= rate * seconds;
        if *wait > 0.0 {
            return None;
        }
        self.wait = None;
        let source = pick_source(sources, rng)?;
        Some(strike_at(source.weather, source.intensity, rng))
    }
}

/// A draw of the exponential distribution with a mean of 1 from `unit` in `[0, 1)`.
fn exponential(unit: f32) -> f32 {
    -(-unit).ln_1p()
}

/// The source a strike comes from, in proportion to the rates of the sources.
fn pick_source(sources: &[StrikeSource], rng: &mut dyn RandomSource) -> Option<StrikeSource> {
    if let [only] = sources {
        return Some(*only);
    }
    let mut picked = *sources.first()?;
    let total: f32 = sources.iter().map(StrikeSource::rate).sum();
    let mut target = rng.unit() * total;
    for source in sources {
        picked = *source;
        target -= source.rate();
        if target < 0.0 {
            break;
        }
    }
    Some(picked)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{A, B, ScriptedRandom, calm, stormy};
    use super::*;
    use crate::random::SplitMix64;
    use crate::weather::FadeRange;

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-3,
            "{actual} is not {expected}"
        );
    }

    fn source(weather: WeatherId, intensity: f32, mean_interval_seconds: f32) -> StrikeSource {
        StrikeSource {
            weather,
            intensity,
            mean_interval_seconds,
        }
    }

    fn strike_with_unit(unit: f32, intensity: f32) -> LightningStrike {
        strike_at(A, intensity, &mut ScriptedRandom::new(&[], &[unit]))
    }

    #[test]
    fn the_rarest_byte_strikes_every_minute_and_the_most_frequent_every_five_seconds() {
        assert_close(mean_strike_interval_seconds(255), 60.0);
        assert_close(mean_strike_interval_seconds(15), 5.0);
    }

    #[test]
    fn bytes_between_the_ends_interpolate_and_those_below_the_most_frequent_stay_there() {
        assert_close(mean_strike_interval_seconds(135), 32.5);
        assert_close(mean_strike_interval_seconds(14), 5.0);
        assert_close(mean_strike_interval_seconds(0), 5.0);
    }

    #[test]
    fn the_vanilla_rain_and_radstorm_bytes_strike_about_a_minute_and_half_a_minute_apart() {
        assert_close(mean_strike_interval_seconds(250), 58.854);
        assert_close(mean_strike_interval_seconds(133), 32.042);
    }

    #[test]
    fn a_strike_lands_between_the_nearest_and_the_farthest_distance() {
        assert_eq!(strike_with_unit(0.0, 1.0).distance_m, 300.0);
        assert_eq!(strike_with_unit(0.5, 1.0).distance_m, 1650.0);
        let farthest = strike_with_unit(1.0 - 1.0 / 16_777_216.0, 1.0).distance_m;
        assert!((2999.9..=3000.0).contains(&farthest), "{farthest}");
    }

    #[test]
    fn thunder_follows_the_flash_after_the_distance_at_the_speed_of_sound() {
        assert_close(strike_with_unit(0.0, 1.0).thunder_delay_s, 300.0 / 343.0);
        assert_close(strike_with_unit(0.5, 1.0).thunder_delay_s, 1650.0 / 343.0);
        let unit_of_343_m = (343.0 - 300.0) / 2700.0;
        assert_close(strike_with_unit(unit_of_343_m, 1.0).thunder_delay_s, 1.0);
    }

    #[test]
    fn a_far_strike_flashes_weaker_than_a_near_one() {
        assert_eq!(strike_with_unit(0.0, 1.0).flash_intensity, 1.0);
        assert_close(strike_with_unit(0.5, 1.0).flash_intensity, 0.675);
        let farthest = strike_with_unit(1.0 - 1.0 / 16_777_216.0, 1.0).flash_intensity;
        assert_close(farthest, 0.35);
    }

    #[test]
    fn the_flash_scales_with_the_strength_of_the_thunder() {
        assert_close(strike_with_unit(0.0, 0.5).flash_intensity, 0.5);
        assert_close(strike_with_unit(0.5, 0.5).flash_intensity, 0.3375);
    }

    #[test]
    fn a_strike_names_the_weather_it_comes_from() {
        let strike = strike_at(B, 1.0, &mut ScriptedRandom::new(&[], &[0.0]));
        assert_eq!(strike.weather, B);
    }

    #[test]
    fn an_exponential_draw_is_zero_at_the_bottom_and_ln_two_at_the_median() {
        assert_eq!(exponential(0.0), 0.0);
        assert_close(exponential(0.5), std::f32::consts::LN_2);
    }

    #[test]
    fn the_largest_draw_gives_a_finite_wait() {
        let wait = exponential(1.0 - 1.0 / 16_777_216.0);
        assert!(wait.is_finite() && wait > 16.0, "{wait}");
    }

    #[test]
    fn exponential_draws_average_their_mean_of_one() {
        let mut rng = SplitMix64::new(11);
        let draws = 50_000;
        let mean = (0..draws).map(|_| exponential(rng.unit())).sum::<f32>() / draws as f32;
        assert!((mean - 1.0).abs() < 0.02, "{mean}");
    }

    #[test]
    fn a_weather_without_a_thunder_sound_never_strikes() {
        for side in [
            TransitionSide::Settled,
            TransitionSide::Incoming { progress: 1.0 },
            TransitionSide::Outgoing { progress: 0.0 },
        ] {
            assert_eq!(strike_source(A, &calm(), side), None, "{side:?}");
        }
    }

    #[test]
    fn a_settled_storm_strikes_at_the_pace_of_its_frequency_byte() {
        let found = strike_source(B, &stormy(), TransitionSide::Settled).unwrap();
        assert_eq!(found.weather, B);
        assert_eq!(found.intensity, 1.0);
        assert_close(found.mean_interval_seconds, 5.0);
    }

    #[test]
    fn the_thunder_fade_gates_a_storm_on_either_side_of_a_transition() {
        let mut traits = stormy();
        traits.lightning.fade = FadeRange {
            begin_fade_in: 0.5,
            end_fade_out: 0.8,
        };
        let intensity = |side| strike_source(A, &traits, side).map(|source| source.intensity);
        assert_eq!(intensity(TransitionSide::Incoming { progress: 0.25 }), None);
        assert_eq!(
            intensity(TransitionSide::Incoming { progress: 0.75 }),
            Some(0.5)
        );
        assert_eq!(
            intensity(TransitionSide::Outgoing { progress: 0.4 }),
            Some(0.5)
        );
        assert_eq!(intensity(TransitionSide::Outgoing { progress: 0.8 }), None);
    }

    #[test]
    fn a_storm_with_the_zero_fade_range_of_the_game_data_strikes_when_settled_and_as_it_arrives() {
        let mut traits = stormy();
        traits.lightning.fade = FadeRange {
            begin_fade_in: 0.0,
            end_fade_out: 0.0,
        };
        let intensity = |side| strike_source(A, &traits, side).map(|source| source.intensity);
        assert_eq!(intensity(TransitionSide::Settled), Some(1.0));
        assert_eq!(
            intensity(TransitionSide::Incoming { progress: 0.5 }),
            Some(0.5)
        );
        assert_eq!(intensity(TransitionSide::Outgoing { progress: 0.5 }), None);
    }

    #[test]
    fn a_lone_source_is_picked_without_a_draw() {
        let only = source(A, 1.0, 5.0);
        assert_eq!(
            pick_source(&[only], &mut ScriptedRandom::silent()),
            Some(only)
        );
    }

    #[test]
    fn no_source_is_picked_from_nothing() {
        assert_eq!(pick_source(&[], &mut ScriptedRandom::silent()), None);
    }

    #[test]
    fn several_sources_are_picked_in_proportion_to_their_rates() {
        let frequent = source(A, 0.5, 5.0);
        let rare = source(B, 0.5, 60.0);
        let pick = |unit| {
            pick_source(&[frequent, rare], &mut ScriptedRandom::new(&[], &[unit]))
                .unwrap()
                .weather
        };
        // The frequent source holds 0.1 of the 0.1083 strikes per second, which is up to 0.923.
        assert_eq!(pick(0.0), A);
        assert_eq!(pick(0.92), A);
        assert_eq!(pick(0.93), B);
        assert_eq!(pick(0.999), B);
    }

    #[test]
    fn nothing_strikes_without_a_source_or_without_time() {
        let mut clock = StrikeClock::default();
        let storm = source(A, 1.0, 5.0);
        let mut rng = ScriptedRandom::silent();
        assert_eq!(clock.advance(&[], 10.0, &mut rng), None);
        for seconds in [0.0, -1.0, f32::NAN] {
            assert_eq!(clock.advance(&[storm], seconds, &mut rng), None);
        }
    }

    #[test]
    fn the_wait_is_drawn_once_and_runs_down_with_the_rate_of_the_sources() {
        let mut clock = StrikeClock::default();
        let storm = source(A, 1.0, 5.0);
        let mut rng = ScriptedRandom::new(&[], &[0.5, 0.0]);
        // A wait of ln 2 expected strikes at 0.2 strikes per second is 3.47 s.
        assert_eq!(clock.advance(&[storm], 3.0, &mut rng), None);
        let strike = clock.advance(&[storm], 0.5, &mut rng).unwrap();
        assert_eq!(strike.weather, A);
        assert_eq!(strike.distance_m, 300.0);
        assert!(rng.is_spent());
    }

    #[test]
    fn a_weaker_source_waits_proportionally_longer() {
        let mut clock = StrikeClock::default();
        let half = source(A, 0.5, 5.0);
        let mut rng = ScriptedRandom::new(&[], &[0.5, 0.0]);
        assert_eq!(clock.advance(&[half], 6.9, &mut rng), None);
        assert!(clock.advance(&[half], 0.1, &mut rng).is_some());
    }

    #[test]
    fn the_rate_changes_with_the_sources_while_the_wait_runs() {
        let mut clock = StrikeClock::default();
        let full = source(A, 1.0, 5.0);
        let faded = source(A, 0.1, 5.0);
        let mut rng = ScriptedRandom::new(&[], &[0.5, 0.0]);
        // 3 s at the full rate leave 0.093 expected strikes, which the faded source needs 4.7 s for.
        assert_eq!(clock.advance(&[full], 3.0, &mut rng), None);
        assert_eq!(clock.advance(&[faded], 4.0, &mut rng), None);
        assert!(clock.advance(&[faded], 1.0, &mut rng).is_some());
    }

    #[test]
    fn a_wait_survives_a_pause_in_the_sources() {
        let mut clock = StrikeClock::default();
        let storm = source(A, 1.0, 5.0);
        let mut rng = ScriptedRandom::new(&[], &[0.5, 0.0]);
        assert_eq!(clock.advance(&[storm], 3.0, &mut rng), None);
        assert_eq!(clock.advance(&[], 100.0, &mut rng), None);
        assert!(clock.advance(&[storm], 0.5, &mut rng).is_some());
    }

    #[test]
    fn after_a_strike_the_next_wait_is_drawn_afresh() {
        let mut clock = StrikeClock::default();
        let storm = source(A, 1.0, 5.0);
        let mut rng = ScriptedRandom::new(&[], &[0.5, 0.0, 0.5]);
        assert!(clock.advance(&[storm], 10.0, &mut rng).is_some());
        assert_eq!(clock.advance(&[storm], 1.0, &mut rng), None);
        assert!(rng.is_spent());
    }

    #[test]
    fn a_long_step_brings_one_strike_not_a_burst() {
        let mut clock = StrikeClock::default();
        let storm = source(A, 1.0, 5.0);
        let mut rng = ScriptedRandom::new(&[], &[0.5, 0.0]);
        assert!(clock.advance(&[storm], 1000.0, &mut rng).is_some());
        assert!(rng.is_spent());
    }

    #[test]
    fn strikes_come_at_the_mean_interval_over_a_long_storm() {
        let mut clock = StrikeClock::default();
        let storm = source(A, 1.0, 5.0);
        let mut rng = SplitMix64::new(3);
        let steps = 200_000;
        let strikes = (0..steps)
            .filter(|_| clock.advance(&[storm], 0.05, &mut rng).is_some())
            .count();
        // 10 000 s at a mean of 5 s is 2 000 strikes.
        assert!((1_850..2_150).contains(&strikes), "{strikes}");
    }
}
