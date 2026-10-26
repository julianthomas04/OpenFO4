//! Lightning flashes: the brief brightening of the exterior's sky and ambient light when lightning
//! strikes. Pure: the simulation decides when strikes happen, the frame loop feeds them to
//! [`LightningFlashes`], and [`super::state::LightingState::set_lightning`] applies the light the
//! flashes add together.
//!
//! The game data carries the lightning colour but not the look of a flash. The envelope and the
//! gains in [`tuning`] are working values, to be retuned by eye.

use glam::Vec3;

use super::cell_lighting::srgb_to_linear;

/// The numbers that shape a flash; the rest of this module is mechanism.
pub mod tuning {
    /// Seconds a pulse takes to rise from dark to its peak.
    pub const ATTACK_S: f32 = 0.02;
    /// Time constant of a pulse's decay: it falls to 1/e this long after its peak.
    pub const DECAY_S: f32 = 0.07;
    /// Seconds from the start of a pulse until it has died out.
    pub const PULSE_LENGTH_S: f32 = 0.5;
    /// Seconds from the strike to the start of the second pulse.
    pub const SECOND_PULSE_DELAY_S: f32 = 0.15;
    /// Peak of the second pulse relative to the first.
    pub const SECOND_PULSE_AMPLITUDE: f32 = 0.6;
    /// Seconds from the strike until its flash is over.
    pub const FLASH_LENGTH_S: f32 = SECOND_PULSE_DELAY_S + PULSE_LENGTH_S;
    /// Linear radiance one unit of flash light adds to the sky's zenith, horizon and ground terms
    /// and to the fog.
    pub const FLASH_SKY_GAIN: f32 = 1.0;
    /// Linear radiance one unit of flash light adds to every face of the exterior ambient cube.
    pub const FLASH_AMBIENT_GAIN: f32 = 0.25;
}

const _: () = assert!(
    tuning::ATTACK_S > 0.0
        && tuning::ATTACK_S < tuning::PULSE_LENGTH_S
        && tuning::DECAY_S > 0.0
        && tuning::SECOND_PULSE_DELAY_S >= 0.0
        && tuning::SECOND_PULSE_AMPLITUDE >= 0.0
        && tuning::SECOND_PULSE_AMPLITUDE <= 1.0,
    "the flash needs an attack that fits in its pulse, a positive decay time and a second pulse of 0 to 1 times the first"
);

/// How bright a flash is `age_s` seconds after its strike, from 0 to 1: a pulse, then a weaker one
/// [`tuning::SECOND_PULSE_DELAY_S`] later. Exactly 0 before the strike and from
/// [`tuning::FLASH_LENGTH_S`] on, with no jump at either end.
pub fn flash_envelope(age_s: f32) -> f32 {
    if !(0.0..tuning::FLASH_LENGTH_S).contains(&age_s) {
        return 0.0;
    }
    pulse(age_s) + tuning::SECOND_PULSE_AMPLITUDE * pulse(age_s - tuning::SECOND_PULSE_DELAY_S)
}

/// One pulse `since_start_s` seconds after it began: a linear rise to 1, then an exponential decay
/// shifted and rescaled so that it still starts at 1 and reaches 0 exactly when the pulse ends.
fn pulse(since_start_s: f32) -> f32 {
    use tuning::{ATTACK_S, DECAY_S, PULSE_LENGTH_S};

    if since_start_s <= 0.0 {
        return 0.0;
    }
    if since_start_s < ATTACK_S {
        return since_start_s / ATTACK_S;
    }
    let decayed = |age_s: f32| (-(age_s - ATTACK_S) / DECAY_S).exp();
    let at_end = decayed(PULSE_LENGTH_S);
    ((decayed(since_start_s) - at_end) / (1.0 - at_end)).max(0.0)
}

/// One lightning strike as the weather frame carries it. The flash's light at its peak is
/// `colour * intensity`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Strike {
    pub intensity: f32,
    /// Linear colour, as [`Self::from_srgb8`] makes it from the weather's lightning colour.
    pub colour: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
enum StrikeError {
    #[error("intensity {0} is not a finite, non-negative number")]
    Intensity(f32),
    #[error("colour {0} has a channel that is not a finite, non-negative number")]
    Colour(Vec3),
}

impl Strike {
    /// A strike in the weather's lightning colour, stored as 0 to 255 per channel, converted to
    /// linear light as the weather's other colours are.
    pub fn from_srgb8(intensity: f32, colour: [u8; 3]) -> Self {
        let [red, green, blue] = colour.map(srgb_to_linear);
        Self {
            intensity,
            colour: Vec3::new(red, green, blue),
        }
    }

    fn validate(&self) -> Result<(), StrikeError> {
        let usable = |value: f32| value.is_finite() && value >= 0.0;
        if !usable(self.intensity) {
            return Err(StrikeError::Intensity(self.intensity));
        }
        if !self.colour.to_array().into_iter().all(usable) {
            return Err(StrikeError::Colour(self.colour));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct ActiveFlash {
    strike: Strike,
    age_s: f32,
}

impl ActiveFlash {
    fn light(&self) -> Vec3 {
        self.strike.colour * (self.strike.intensity * flash_envelope(self.age_s))
    }
}

/// The flashes of the strikes that are still alive, and the light they add together.
#[derive(Debug, Clone, Default)]
pub struct LightningFlashes {
    active: Vec<ActiveFlash>,
}

impl LightningFlashes {
    /// Advances every flash by `dt_s` real seconds (a negative or NaN step is no time), starts a
    /// flash for each of `strikes` and returns the light all flashes add together. A new flash
    /// starts at age 0, so its light rises from the next update on. Flashes that have ended are
    /// dropped; a strike with an unusable number is reported and left out.
    pub fn update(&mut self, dt_s: f32, strikes: &[Strike]) -> Vec3 {
        let step_s = dt_s.max(0.0);
        for flash in &mut self.active {
            flash.age_s += step_s;
        }
        self.active
            .retain(|flash| flash.age_s < tuning::FLASH_LENGTH_S);
        for strike in strikes {
            match strike.validate() {
                Ok(()) => self.active.push(ActiveFlash {
                    strike: *strike,
                    age_s: 0.0,
                }),
                Err(error) => tracing::warn!("lightning strike ignored: {error}"),
            }
        }
        self.active.iter().map(ActiveFlash::light).sum()
    }

    /// Whether no flash is alive, so the light is none.
    pub fn is_idle(&self) -> bool {
        self.active.is_empty()
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the envelope is exact at its start, its peak and its end"
)]
mod tests {
    use proptest::prelude::*;

    use super::tuning::{
        ATTACK_S, DECAY_S, FLASH_LENGTH_S, SECOND_PULSE_AMPLITUDE, SECOND_PULSE_DELAY_S,
    };
    use super::*;
    use crate::audio::test_support::SharedBuffer;

    #[test]
    fn the_envelope_is_exactly_zero_before_the_strike() {
        for age in [-10.0, -0.000_1, -0.0, f32::NEG_INFINITY, f32::NAN] {
            assert_eq!(flash_envelope(age), 0.0, "{age}");
        }
    }

    #[test]
    fn the_envelope_rises_from_nothing_to_its_peak_over_the_attack() {
        assert_eq!(flash_envelope(0.0), 0.0);
        assert!((flash_envelope(ATTACK_S / 2.0) - 0.5).abs() < 1e-6);
        assert_eq!(flash_envelope(ATTACK_S), 1.0);
        let rise: Vec<f32> = (0..=20)
            .map(|step| flash_envelope(ATTACK_S * step as f32 / 20.0))
            .collect();
        assert!(rise.windows(2).all(|pair| pair[0] < pair[1]), "{rise:?}");
    }

    #[test]
    fn after_its_peak_the_first_pulse_only_falls_until_the_second_begins() {
        let steps = 50;
        let span = SECOND_PULSE_DELAY_S - ATTACK_S;
        let fall: Vec<f32> = (0..=steps)
            .map(|step| flash_envelope(ATTACK_S + span * step as f32 / steps as f32))
            .collect();
        assert!(fall.windows(2).all(|pair| pair[0] > pair[1]), "{fall:?}");
    }

    #[test]
    fn the_decay_is_exponential_with_the_decay_time_constant() {
        // Two samples one time constant apart differ by a factor of e.
        let early = ATTACK_S + 0.01;
        let ratio = flash_envelope(early) / flash_envelope(early + DECAY_S);
        assert!(
            (ratio - std::f32::consts::E).abs() < 0.02,
            "ratio {ratio} is not e"
        );
    }

    #[test]
    fn a_weaker_second_pulse_follows_after_a_dip() {
        let dip = flash_envelope(SECOND_PULSE_DELAY_S);
        let second_peak = flash_envelope(SECOND_PULSE_DELAY_S + ATTACK_S);
        assert!(second_peak > dip, "the second pulse rises out of the dip");
        assert!(second_peak < 1.0, "the second peak is below the first");
        assert!(
            second_peak >= SECOND_PULSE_AMPLITUDE,
            "the second pulse adds its whole amplitude"
        );
        let later_maximum = (1..=200)
            .map(|step| flash_envelope(SECOND_PULSE_DELAY_S + 0.001 * step as f32))
            .fold(0.0_f32, f32::max);
        assert!(later_maximum <= second_peak + 1e-5, "{later_maximum}");
    }

    #[test]
    fn the_envelope_is_exactly_zero_from_the_end_on_and_does_not_jump_there() {
        assert_eq!(flash_envelope(FLASH_LENGTH_S), 0.0);
        assert_eq!(flash_envelope(FLASH_LENGTH_S + 1.0), 0.0);
        assert_eq!(flash_envelope(f32::INFINITY), 0.0);
        assert!(flash_envelope(FLASH_LENGTH_S.next_down()) < 1e-4);
    }

    /// The envelope every tenth of a millisecond, from just before the strike to just after the end.
    fn sweep() -> Vec<f32> {
        let step = 1e-4;
        let samples = (FLASH_LENGTH_S / step) as usize + 200;
        (0..samples)
            .map(|index| flash_envelope((index as f32 - 100.0) * step))
            .collect()
    }

    #[test]
    fn the_envelope_stays_between_zero_and_one_over_the_whole_flash() {
        for (index, level) in sweep().into_iter().enumerate() {
            assert!((0.0..=1.0).contains(&level), "sample {index}: {level}");
        }
    }

    #[test]
    fn the_envelope_never_jumps_over_the_whole_flash() {
        for (index, pair) in sweep().windows(2).enumerate() {
            assert!(
                (pair[1] - pair[0]).abs() < 0.01,
                "jump after sample {index}: {} -> {}",
                pair[0],
                pair[1]
            );
        }
    }

    proptest! {
        #[test]
        fn the_envelope_is_exactly_zero_for_any_age_outside_the_flash(
            age in prop::num::f32::ANY,
        ) {
            prop_assume!(!(0.0..FLASH_LENGTH_S).contains(&age));
            prop_assert_eq!(flash_envelope(age), 0.0);
        }
    }

    const COLOUR: Vec3 = Vec3::new(0.5, 0.25, 1.0);

    fn strike(intensity: f32) -> Strike {
        Strike {
            intensity,
            colour: COLOUR,
        }
    }

    fn assert_light(actual: Vec3, expected: Vec3) {
        assert!(
            actual.abs_diff_eq(expected, 1e-6),
            "{actual} is not {expected}"
        );
    }

    #[test]
    fn without_strikes_there_is_no_light() {
        let mut flashes = LightningFlashes::default();
        assert_eq!(flashes.update(0.1, &[]), Vec3::ZERO);
        assert!(flashes.is_idle());
    }

    #[test]
    fn a_strike_lights_its_colour_times_its_intensity_at_the_peak() {
        let mut flashes = LightningFlashes::default();
        assert_eq!(flashes.update(0.0, &[strike(0.5)]), Vec3::ZERO);
        assert!(!flashes.is_idle());
        assert_light(flashes.update(ATTACK_S, &[]), COLOUR * 0.5);
    }

    #[test]
    fn a_new_flash_starts_at_age_zero_whatever_the_step_it_arrives_in() {
        let mut flashes = LightningFlashes::default();
        assert_eq!(flashes.update(0.3, &[strike(1.0)]), Vec3::ZERO);
        assert_light(flashes.update(ATTACK_S, &[]), COLOUR);
    }

    #[test]
    fn a_strike_follows_the_envelope_as_time_passes() {
        let mut flashes = LightningFlashes::default();
        flashes.update(0.0, &[strike(2.0)]);
        let mut age = 0.0;
        for step in [0.01, 0.02, 0.09, 0.05, 0.23] {
            age += step;
            let light = flashes.update(step, &[]);
            assert_light(light, COLOUR * (2.0 * flash_envelope(age)));
        }
    }

    #[test]
    fn strikes_add_up_each_at_its_own_age() {
        let first = strike(1.0);
        let second = Strike {
            intensity: 0.5,
            colour: Vec3::new(1.0, 0.0, 0.0),
        };
        let mut flashes = LightningFlashes::default();
        flashes.update(0.0, &[first]);
        flashes.update(0.1, &[second]);
        let light = flashes.update(0.02, &[]);
        let expected =
            first.colour * flash_envelope(0.12) + second.colour * (0.5 * flash_envelope(0.02));
        assert_light(light, expected);
    }

    #[test]
    fn strikes_arriving_together_start_together() {
        let mut flashes = LightningFlashes::default();
        flashes.update(0.0, &[strike(1.0), strike(0.5)]);
        assert_light(flashes.update(ATTACK_S, &[]), COLOUR * 1.5);
    }

    #[test]
    fn a_flash_is_dropped_once_its_length_has_passed() {
        let mut flashes = LightningFlashes::default();
        flashes.update(0.0, &[strike(1.0)]);
        flashes.update(FLASH_LENGTH_S - 0.05, &[]);
        assert!(!flashes.is_idle());
        assert_eq!(flashes.update(0.1, &[]), Vec3::ZERO);
        assert!(flashes.is_idle());
    }

    #[test]
    fn a_flash_that_has_lasted_exactly_its_length_is_over() {
        let mut flashes = LightningFlashes::default();
        flashes.update(0.0, &[strike(1.0)]);
        assert_eq!(flashes.update(FLASH_LENGTH_S, &[]), Vec3::ZERO);
        assert!(flashes.is_idle());
    }

    #[test]
    fn flashes_end_independently_of_each_other() {
        let early = strike(1.0);
        let late = Strike {
            intensity: 0.5,
            colour: Vec3::new(0.0, 1.0, 0.0),
        };
        let mut flashes = LightningFlashes::default();
        flashes.update(0.0, &[early]);
        flashes.update(0.5, &[late]);
        let light = flashes.update(0.2, &[]);
        assert_light(light, late.colour * (0.5 * flash_envelope(0.2)));
        flashes.update(0.2, &[]);
        assert!(!flashes.is_idle());
        flashes.update(FLASH_LENGTH_S, &[]);
        assert!(flashes.is_idle());
    }

    #[test]
    fn the_light_depends_on_the_elapsed_time_not_on_how_it_is_sliced() {
        let mut one_step = LightningFlashes::default();
        let mut many_steps = LightningFlashes::default();
        one_step.update(0.0, &[strike(1.0)]);
        many_steps.update(0.0, &[strike(1.0)]);
        let whole = one_step.update(0.2, &[]);
        let mut sliced = Vec3::ZERO;
        for _ in 0..20 {
            sliced = many_steps.update(0.01, &[]);
        }
        assert!(whole.abs_diff_eq(sliced, 1e-4), "{whole} vs {sliced}");
    }

    #[test]
    fn a_negative_or_nan_step_is_no_time() {
        let mut flashes = LightningFlashes::default();
        flashes.update(0.0, &[strike(1.0)]);
        let still = flashes.update(0.05, &[]);
        for step in [-1.0, f32::NAN, f32::NEG_INFINITY] {
            assert_eq!(flashes.update(step, &[]), still, "{step}");
        }
    }

    #[test]
    fn a_strike_with_an_unusable_number_is_reported_and_leaves_no_flash() {
        let unusable = [
            strike(f32::NAN),
            strike(-1.0),
            strike(f32::INFINITY),
            Strike {
                intensity: 1.0,
                colour: Vec3::new(f32::NAN, 0.0, 0.0),
            },
            Strike {
                intensity: 1.0,
                colour: Vec3::new(0.0, -0.5, 0.0),
            },
            Strike {
                intensity: 1.0,
                colour: Vec3::new(0.0, 0.0, f32::INFINITY),
            },
        ];
        for bad in unusable {
            let log = SharedBuffer::default();
            let guard = log.capture("warn");
            let mut flashes = LightningFlashes::default();
            flashes.update(0.0, &[bad]);
            drop(guard);
            assert!(
                log.drain().contains("lightning strike ignored"),
                "{bad:?} is not reported"
            );
            assert_eq!(flashes.update(ATTACK_S, &[]), Vec3::ZERO, "{bad:?}");
            assert!(flashes.is_idle(), "{bad:?}");
        }
    }

    #[test]
    fn an_unusable_strike_does_not_spoil_the_usable_ones_beside_it() {
        let mut flashes = LightningFlashes::default();
        flashes.update(0.0, &[strike(f32::NAN), strike(1.0), strike(-2.0)]);
        assert_light(flashes.update(ATTACK_S, &[]), COLOUR);
    }

    #[test]
    fn a_strike_names_which_of_its_numbers_is_unusable() {
        assert_eq!(strike(0.0).validate(), Ok(()));
        assert!(matches!(
            strike(f32::NAN).validate(),
            Err(StrikeError::Intensity(_))
        ));
        let bad_colour = Strike {
            intensity: 1.0,
            colour: Vec3::new(0.0, f32::NAN, 0.0),
        };
        assert!(matches!(bad_colour.validate(), Err(StrikeError::Colour(_))));
    }

    #[test]
    fn the_weather_lightning_colour_is_converted_to_linear_like_the_sky_colours() {
        let strike = Strike::from_srgb8(0.75, [255, 0, 128]);
        assert_eq!(strike.intensity, 0.75);
        assert_eq!(strike.colour.x, 1.0);
        assert_eq!(strike.colour.y, 0.0);
        // Mid grey in sRGB is about 0.2159 in linear light.
        assert!(
            (strike.colour.z - 0.2159).abs() < 1e-4,
            "{}",
            strike.colour.z
        );
    }
}
