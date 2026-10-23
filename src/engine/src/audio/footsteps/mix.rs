//! Pitch and volume of one sound layer, from its descriptor, output model and category chain.
//!
//! Static attenuation values are positive attenuation, so they subtract from the level. The
//! formula is INFERRED from the descriptor fields, not documented by the game:
//!
//! - playback rate = 1 + (frequency shift + U(-frequency variance, +frequency variance)) / 100,
//!   both in percent;
//! - gain in dB = the sum of: minus the static attenuation in dB, U(-dB variance, +dB variance),
//!   minus the output model static attenuation, 20 log10 of the channel gain, and 20 log10 of
//!   the multiplier of every category of the chain up to the root.

use std::collections::{HashMap, HashSet};

use crate::assets::db::footsteps::{SoundDescriptor, SoundOutputModel};
use crate::assets::db::music::SoundCategory;
use crate::audio::rng::AudioRng;

/// The looping type lives in the second byte of the descriptor's flags: loop `0x08`, envelope
/// fast `0x10`, envelope slow `0x20` (the Skyrim descriptor layout, assumed to carry over). The
/// other bits of that byte, set on every footstep layer, do not loop.
const LOOPING_TYPE_MASK: u32 = 0x3800;
const FIRST_INPUT_CHANNEL: u32 = 0;
const LEFT_SPEAKER: &str = "L";
const RIGHT_SPEAKER: &str = "R";
const CENTIDB_PER_DB: f32 = 100.0;
const PERCENT: f32 = 100.0;
const DB_PER_DECADE_OF_AMPLITUDE: f32 = 20.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerMix {
    pub playback_rate: f64,
    pub gain_db: f32,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MixError {
    #[error("sound descriptor {0} names no sound category")]
    NoCategory(String),
    #[error("sound category {0} is not in the catalog")]
    UnknownCategory(String),
    #[error("sound category {0} has no static multiplier")]
    CategoryWithoutMultiplier(String),
    #[error("sound category {category} has the unusable multiplier {multiplier}")]
    InvalidMultiplier { category: String, multiplier: f32 },
    #[error("sound category {0} has a parent cycle")]
    CategoryCycle(String),
    #[error("sound output model {model} has no {speaker} share for input channel 0")]
    MissingChannel {
        model: String,
        speaker: &'static str,
    },
    #[error(
        "sound output model {model} sends {left} percent to L and {right} percent to R; \
         the footstep gain for unequal shares is undecided"
    )]
    UnequalChannelShares {
        model: String,
        left: i32,
        right: i32,
    },
    #[error("sound output model {model} has the negative share {percent} percent")]
    NegativeChannelShare { model: String, percent: i32 },
    #[error("sound descriptor {descriptor} yields the unusable playback rate {rate}")]
    InvalidPlaybackRate { descriptor: String, rate: f64 },
}

/// Whether the descriptor's looping type is set; a looping sound has no end for a step to have.
pub fn is_looping(descriptor: &SoundDescriptor) -> bool {
    descriptor.looping_flags & LOOPING_TYPE_MASK != 0
}

impl LayerMix {
    /// Draws the pitch and volume variance for one layer: the frequency variance first, then the
    /// dB variance.
    pub fn draw(
        descriptor: &SoundDescriptor,
        model: &SoundOutputModel,
        categories: &HashMap<String, SoundCategory>,
        rng: &mut dyn AudioRng,
    ) -> Result<Self, MixError> {
        let playback_rate = playback_rate(descriptor, rng)?;
        let gain_db = gain_db(descriptor, model, categories, rng)?;
        Ok(Self {
            playback_rate,
            gain_db,
        })
    }
}

fn playback_rate(descriptor: &SoundDescriptor, rng: &mut dyn AudioRng) -> Result<f64, MixError> {
    let variance = rng.symmetric(descriptor.freq_variance_pct.unsigned_abs() as f32);
    let rate = 1.0 + (f64::from(descriptor.freq_shift_pct) + f64::from(variance)) / 100.0;
    if rate > 0.0 {
        Ok(rate)
    } else {
        Err(MixError::InvalidPlaybackRate {
            descriptor: descriptor.form_key.clone(),
            rate,
        })
    }
}

fn gain_db(
    descriptor: &SoundDescriptor,
    model: &SoundOutputModel,
    categories: &HashMap<String, SoundCategory>,
    rng: &mut dyn AudioRng,
) -> Result<f32, MixError> {
    let variance = rng.symmetric(descriptor.db_variance.unsigned_abs() as f32);
    Ok(
        -(descriptor.static_attenuation_centidb as f32 / CENTIDB_PER_DB) + variance
            - model.static_attenuation_db
            + to_db(channel_gain(model)?)
            + category_chain_db(descriptor, categories)?,
    )
}

fn to_db(amplitude: f32) -> f32 {
    DB_PER_DECADE_OF_AMPLITUDE * amplitude.log10()
}

/// The share of input channel 0 that reaches the left and right speaker. Unequal shares are not
/// decided, so they are an error.
fn channel_gain(model: &SoundOutputModel) -> Result<f32, MixError> {
    let share = |speaker: &'static str| {
        model
            .channels
            .iter()
            .find(|channel| {
                channel.input_channel == FIRST_INPUT_CHANNEL && channel.speaker == speaker
            })
            .map(|channel| channel.percent)
            .ok_or_else(|| MixError::MissingChannel {
                model: model.form_key.clone(),
                speaker,
            })
    };
    let (left, right) = (share(LEFT_SPEAKER)?, share(RIGHT_SPEAKER)?);
    if left != right {
        return Err(MixError::UnequalChannelShares {
            model: model.form_key.clone(),
            left,
            right,
        });
    }
    if left < 0 {
        return Err(MixError::NegativeChannelShare {
            model: model.form_key.clone(),
            percent: left,
        });
    }
    Ok(left as f32 / PERCENT)
}

fn category_chain_db(
    descriptor: &SoundDescriptor,
    categories: &HashMap<String, SoundCategory>,
) -> Result<f32, MixError> {
    let mut key = descriptor
        .category
        .as_deref()
        .ok_or_else(|| MixError::NoCategory(descriptor.form_key.clone()))?;
    let mut visited = HashSet::new();
    let mut total = 0.0;
    loop {
        if !visited.insert(key) {
            return Err(MixError::CategoryCycle(key.to_string()));
        }
        let category = categories
            .get(key)
            .ok_or_else(|| MixError::UnknownCategory(key.to_string()))?;
        let multiplier = category
            .static_multiplier
            .ok_or_else(|| MixError::CategoryWithoutMultiplier(key.to_string()))?;
        if !multiplier.is_finite() || multiplier < 0.0 {
            return Err(MixError::InvalidMultiplier {
                category: key.to_string(),
                multiplier,
            });
        }
        total += to_db(multiplier);
        match &category.parent {
            Some(parent) => key = parent,
            None => return Ok(total),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::db::footsteps::OutputChannel;

    /// Replays fixed unit draws, so a test can pin the variance to either end.
    struct ScriptedUnits(Vec<f32>);

    impl AudioRng for ScriptedUnits {
        fn below(&mut self, _n: usize) -> usize {
            unreachable!("only unit draws are scripted")
        }

        fn unit(&mut self) -> f32 {
            self.0.remove(0)
        }
    }

    const MIDDLE: f32 = 0.5;
    const TOLERANCE: f32 = 1e-4;

    fn descriptor() -> SoundDescriptor {
        SoundDescriptor {
            form_key: "step".to_string(),
            editor_id: None,
            descriptor_type: 0x1EEF_540A,
            base_descriptor: None,
            freq_shift_pct: 0,
            freq_variance_pct: 0,
            priority: 0,
            db_variance: 0,
            static_attenuation_centidb: 0,
            looping_flags: 0,
            category: Some("leaf".to_string()),
            output_model: Some("model".to_string()),
            sound_files: vec![],
            children: vec![],
            conditions: vec![],
        }
    }

    fn channel(speaker: &str, percent: i32) -> OutputChannel {
        OutputChannel {
            input_channel: 0,
            speaker: speaker.to_string(),
            percent,
        }
    }

    fn model(attenuation_db: f32, channels: Vec<OutputChannel>) -> SoundOutputModel {
        SoundOutputModel {
            form_key: "model".to_string(),
            editor_id: None,
            output_type: 1,
            static_attenuation_db: attenuation_db,
            channels,
        }
    }

    fn unity_model() -> SoundOutputModel {
        model(0.0, vec![channel("L", 100), channel("R", 100)])
    }

    fn category(key: &str, multiplier: Option<f32>, parent: Option<&str>) -> SoundCategory {
        SoundCategory {
            form_key: key.to_string(),
            editor_id: None,
            parent: parent.map(ToString::to_string),
            static_multiplier: multiplier,
        }
    }

    fn categories(entries: Vec<SoundCategory>) -> HashMap<String, SoundCategory> {
        entries
            .into_iter()
            .map(|entry| (entry.form_key.clone(), entry))
            .collect()
    }

    fn unity_categories() -> HashMap<String, SoundCategory> {
        categories(vec![category("leaf", Some(1.0), None)])
    }

    fn draw(
        descriptor: &SoundDescriptor,
        model: &SoundOutputModel,
        categories: &HashMap<String, SoundCategory>,
        units: Vec<f32>,
    ) -> Result<LayerMix, MixError> {
        LayerMix::draw(descriptor, model, categories, &mut ScriptedUnits(units))
    }

    fn mix(descriptor: &SoundDescriptor, model: &SoundOutputModel) -> LayerMix {
        draw(descriptor, model, &unity_categories(), vec![MIDDLE, MIDDLE]).unwrap()
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < TOLERANCE,
            "{actual} is not {expected}"
        );
    }

    #[test]
    fn a_descriptor_without_adjustments_plays_unchanged() {
        let result = mix(&descriptor(), &unity_model());
        assert!((result.playback_rate - 1.0).abs() < 1e-12);
        assert_close(result.gain_db, 0.0);
    }

    #[test]
    fn frequency_shift_moves_the_rate_by_percent() {
        let shifted = SoundDescriptor {
            freq_shift_pct: -4,
            ..descriptor()
        };
        assert!((mix(&shifted, &unity_model()).playback_rate - 0.96).abs() < 1e-9);
    }

    #[test]
    fn frequency_variance_spans_shift_plus_minus_variance() {
        let varied = SoundDescriptor {
            freq_shift_pct: 5,
            freq_variance_pct: 10,
            ..descriptor()
        };
        let rate = |first_draw: f32| {
            draw(
                &varied,
                &unity_model(),
                &unity_categories(),
                vec![first_draw, MIDDLE],
            )
            .unwrap()
            .playback_rate
        };
        assert!((rate(0.0) - 0.95).abs() < 1e-9);
        assert!((rate(MIDDLE) - 1.05).abs() < 1e-9);
        assert!((rate(0.999_999) - 1.15).abs() < 1e-4);
    }

    #[test]
    fn static_attenuation_lowers_the_gain_in_hundredths_of_a_db() {
        let attenuated = SoundDescriptor {
            static_attenuation_centidb: 450,
            ..descriptor()
        };
        assert_close(mix(&attenuated, &unity_model()).gain_db, -4.5);
    }

    #[test]
    fn db_variance_spans_plus_minus_the_variance() {
        let varied = SoundDescriptor {
            db_variance: 3,
            ..descriptor()
        };
        let gain = |second_draw: f32| {
            draw(
                &varied,
                &unity_model(),
                &unity_categories(),
                vec![MIDDLE, second_draw],
            )
            .unwrap()
            .gain_db
        };
        assert_close(gain(0.0), -3.0);
        assert_close(gain(MIDDLE), 0.0);
        assert_close(gain(1.0), 3.0);
    }

    #[test]
    fn output_model_attenuation_lowers_the_gain() {
        let result = mix(
            &descriptor(),
            &model(1.76, vec![channel("L", 100), channel("R", 100)]),
        );
        assert_close(result.gain_db, -1.76);
    }

    #[test]
    fn equal_channel_shares_apply_as_a_percentage_gain() {
        let result = mix(
            &descriptor(),
            &model(0.0, vec![channel("L", 75), channel("R", 75)]),
        );
        assert_close(result.gain_db, 20.0 * 0.75_f32.log10());
    }

    #[test]
    fn only_input_channel_zero_counts() {
        let other = OutputChannel {
            input_channel: 1,
            speaker: "L".to_string(),
            percent: 10,
        };
        let result = mix(
            &descriptor(),
            &model(0.0, vec![channel("L", 50), channel("R", 50), other]),
        );
        assert_close(result.gain_db, 20.0 * 0.5_f32.log10());
    }

    #[test]
    fn unequal_channel_shares_are_an_error() {
        let result = draw(
            &descriptor(),
            &model(0.0, vec![channel("L", 75), channel("R", 50)]),
            &unity_categories(),
            vec![MIDDLE, MIDDLE],
        );
        assert!(matches!(
            result,
            Err(MixError::UnequalChannelShares {
                left: 75,
                right: 50,
                ..
            })
        ));
    }

    #[test]
    fn a_missing_speaker_share_is_an_error() {
        let result = draw(
            &descriptor(),
            &model(0.0, vec![channel("L", 75)]),
            &unity_categories(),
            vec![MIDDLE, MIDDLE],
        );
        assert!(matches!(
            result,
            Err(MixError::MissingChannel { speaker: "R", .. })
        ));
    }

    #[test]
    fn the_category_chain_sums_its_multipliers_in_db_up_to_the_root() {
        let chain = categories(vec![
            category("leaf", Some(0.95), Some("mid")),
            category("mid", Some(0.5), Some("master")),
            category("master", Some(0.85), None),
        ]);
        let result = draw(&descriptor(), &unity_model(), &chain, vec![MIDDLE, MIDDLE]).unwrap();
        assert_close(result.gain_db, 20.0 * (0.95_f32 * 0.5 * 0.85).log10());
    }

    #[test]
    fn every_term_adds_up_in_one_formula() {
        let full = SoundDescriptor {
            static_attenuation_centidb: 200,
            db_variance: 2,
            ..descriptor()
        };
        let chain = categories(vec![category("leaf", Some(0.5), None)]);
        let result = draw(
            &full,
            &model(1.0, vec![channel("L", 50), channel("R", 50)]),
            &chain,
            vec![MIDDLE, 1.0],
        )
        .unwrap();
        let expected = -2.0 + 2.0 - 1.0 + 20.0 * 0.5_f32.log10() + 20.0 * 0.5_f32.log10();
        assert_close(result.gain_db, expected);
    }

    #[test]
    fn category_data_that_cannot_be_used_is_an_error() {
        let cases = [
            (
                categories(vec![category("leaf", None, None)]),
                MixError::CategoryWithoutMultiplier("leaf".to_string()),
            ),
            (
                categories(vec![category("leaf", Some(1.0), Some("gone"))]),
                MixError::UnknownCategory("gone".to_string()),
            ),
            (
                categories(vec![
                    category("leaf", Some(1.0), Some("other")),
                    category("other", Some(1.0), Some("leaf")),
                ]),
                MixError::CategoryCycle("leaf".to_string()),
            ),
            (
                categories(vec![category("leaf", Some(-1.0), None)]),
                MixError::InvalidMultiplier {
                    category: "leaf".to_string(),
                    multiplier: -1.0,
                },
            ),
        ];
        for (chain, expected) in cases {
            let result = draw(&descriptor(), &unity_model(), &chain, vec![MIDDLE, MIDDLE]);
            assert_eq!(result, Err(expected));
        }
    }

    #[test]
    fn a_descriptor_without_a_category_is_an_error() {
        let orphan = SoundDescriptor {
            category: None,
            ..descriptor()
        };
        assert_eq!(
            draw(
                &orphan,
                &unity_model(),
                &unity_categories(),
                vec![MIDDLE, MIDDLE]
            ),
            Err(MixError::NoCategory("step".to_string()))
        );
    }

    #[test]
    fn a_rate_that_is_not_positive_is_an_error() {
        let silent = SoundDescriptor {
            freq_shift_pct: -100,
            ..descriptor()
        };
        assert!(matches!(
            draw(
                &silent,
                &unity_model(),
                &unity_categories(),
                vec![MIDDLE, MIDDLE]
            ),
            Err(MixError::InvalidPlaybackRate { .. })
        ));
    }

    #[test]
    fn only_the_looping_type_bits_mark_a_descriptor_as_looping() {
        let flags = |value: u32| {
            is_looping(&SoundDescriptor {
                looping_flags: value,
                ..descriptor()
            })
        };
        assert!(!flags(0));
        assert!(!flags(0x0100), "the bit every footstep layer stores");
        assert!(flags(0x0800), "loop");
        assert!(flags(0x1000), "envelope fast");
        assert!(flags(0x2000), "envelope slow");
        assert!(flags(0x0900));
    }
}
