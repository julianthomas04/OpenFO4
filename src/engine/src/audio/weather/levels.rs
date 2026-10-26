//! How loud each looping weather sound should be. Pure: the sounds of the weathers, the weathers
//! that show and where the player is are inputs.

use std::collections::BTreeMap;

use super::error::WeatherAudioError;
use super::input::{WeatherSide, unit};
use super::table::WeatherSoundTable;
use crate::weather::SoundKind;

const DB_PER_DECADE_OF_AMPLITUDE: f32 = 20.0;

/// A looping sound and the part of its steady level it should play at.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopLevel {
    pub descriptor: String,
    /// In `(0, 1]`.
    pub level: f32,
}

/// The looping sounds to hear and how loud, by descriptor and without the silent ones. A default
/// sound follows the weight of its weather, a precipitation sound also the precipitation that
/// shows and a wind sound also the wind speed; thunder is no loop. A descriptor several sounds name
/// takes the sum of their levels, at most 1: the bed two weathers share keeps its level through
/// the transition between them. Indoors the weather is not heard, so no loop is wanted.
pub fn loop_levels(
    table: &WeatherSoundTable,
    sides: &[WeatherSide],
    outdoor: bool,
) -> Result<Vec<LoopLevel>, WeatherAudioError> {
    if !outdoor {
        return Ok(Vec::new());
    }
    let mut levels: BTreeMap<&str, f32> = BTreeMap::new();
    for side in sides {
        for sound in table.sounds(side.weather)? {
            if let Some(share) = share_of(sound.kind, side) {
                *levels.entry(&sound.descriptor).or_default() += unit(side.weight) * share;
            }
        }
    }
    Ok(levels
        .into_iter()
        .filter(|&(_, level)| level > 0.0)
        .map(|(descriptor, level)| LoopLevel {
            descriptor: descriptor.to_string(),
            level: level.min(1.0),
        })
        .collect())
}

/// The gain of `level` against the steady level, `0` at 1 and minus infinity at 0.
pub fn level_db(level: f32) -> f32 {
    DB_PER_DECADE_OF_AMPLITUDE * level.log10()
}

/// How much of its weight a sound of `kind` has on `side`; `None` for the sounds that are no loops.
fn share_of(kind: SoundKind, side: &WeatherSide) -> Option<f32> {
    match kind {
        SoundKind::Default => Some(1.0),
        SoundKind::Precipitation => Some(unit(side.precipitation)),
        SoundKind::Wind => Some(unit(side.wind)),
        SoundKind::Thunder => None,
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the expected levels are exactly representable"
)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::weather::{FadeRange, TransitionSide, WeatherId, WeatherSound};

    const CLEAR: WeatherId = WeatherId::new(0);
    const RAIN: WeatherId = WeatherId::new(1);
    const STORM: WeatherId = WeatherId::new(2);
    const NEVER_FADES: FadeRange = FadeRange {
        begin_fade_in: 0.0,
        end_fade_out: 1.0,
    };

    fn sound(kind: SoundKind, descriptor: &str) -> WeatherSound {
        WeatherSound {
            kind,
            descriptor: descriptor.to_string(),
        }
    }

    /// Clear has a bed, rain shares it and adds rain, gusts and thunder, the storm has a bed of
    /// its own.
    fn table() -> WeatherSoundTable {
        WeatherSoundTable::new(vec![
            vec![sound(SoundKind::Default, "bed")],
            vec![
                sound(SoundKind::Default, "bed"),
                sound(SoundKind::Precipitation, "rain"),
                sound(SoundKind::Wind, "gusts"),
                sound(SoundKind::Thunder, "thunder"),
            ],
            vec![sound(SoundKind::Default, "storm bed")],
        ])
    }

    fn side(weather: WeatherId, weight: f32, precipitation: f32, wind: f32) -> WeatherSide {
        WeatherSide {
            weather,
            weight,
            precipitation,
            wind,
        }
    }

    fn settled(weather: WeatherId) -> WeatherSide {
        side(weather, 1.0, 1.0, 1.0)
    }

    fn levels(sides: &[WeatherSide]) -> Vec<(String, f32)> {
        loop_levels(&table(), sides, true)
            .unwrap()
            .into_iter()
            .map(|loop_level| (loop_level.descriptor, loop_level.level))
            .collect()
    }

    fn level_of(sides: &[WeatherSide], descriptor: &str) -> f32 {
        levels(sides)
            .into_iter()
            .find(|(name, _)| name == descriptor)
            .map_or(0.0, |(_, level)| level)
    }

    #[test]
    fn a_settled_weather_plays_its_beds_at_full_level_and_no_thunder() {
        assert_eq!(
            levels(&[settled(RAIN)]),
            [
                ("bed".to_string(), 1.0),
                ("gusts".to_string(), 1.0),
                ("rain".to_string(), 1.0),
            ]
        );
    }

    #[test]
    fn a_default_sound_follows_the_weight_of_its_weather_whatever_the_precipitation_and_wind() {
        assert_eq!(level_of(&[side(CLEAR, 0.4, 0.0, 0.0)], "bed"), 0.4);
    }

    #[test]
    fn a_precipitation_sound_follows_the_weight_times_the_precipitation() {
        assert_eq!(level_of(&[side(RAIN, 0.5, 0.5, 1.0)], "rain"), 0.25);
        assert_eq!(level_of(&[side(RAIN, 1.0, 0.0, 1.0)], "rain"), 0.0);
    }

    #[test]
    fn a_wind_sound_follows_the_weight_times_the_wind_speed() {
        assert_eq!(level_of(&[side(RAIN, 0.5, 1.0, 0.5)], "gusts"), 0.25);
        assert_eq!(level_of(&[side(RAIN, 1.0, 1.0, 0.0)], "gusts"), 0.0);
    }

    #[test]
    fn a_sound_with_no_level_is_left_out_and_so_is_one_that_is_not_a_number() {
        assert_eq!(
            levels(&[side(RAIN, 1.0, 0.0, 0.0)]),
            [("bed".to_string(), 1.0)]
        );
        assert_eq!(levels(&[side(RAIN, f32::NAN, 1.0, 1.0)]), []);
        assert_eq!(
            levels(&[side(RAIN, 1.0, f32::NAN, f32::NAN)]),
            [("bed".to_string(), 1.0)]
        );
    }

    #[test]
    fn indoors_no_loop_is_wanted() {
        assert_eq!(loop_levels(&table(), &[settled(RAIN)], false).unwrap(), []);
    }

    #[test]
    fn a_weather_that_shows_nothing_wants_no_loop() {
        assert_eq!(levels(&[]), []);
    }

    #[test]
    fn the_beds_of_two_weathers_cross_fade_with_the_transition() {
        let incoming = side(STORM, 0.25, 1.0, 1.0);
        let outgoing = side(CLEAR, 0.75, 1.0, 1.0);

        assert_eq!(
            levels(&[incoming, outgoing]),
            [("bed".to_string(), 0.75), ("storm bed".to_string(), 0.25)]
        );
    }

    #[test]
    fn a_bed_both_weathers_share_keeps_its_level_through_the_whole_transition() {
        for step in 0..=20 {
            let progress = step as f32 / 20.0;
            let sides = [
                WeatherSide::new(
                    RAIN,
                    TransitionSide::Incoming { progress },
                    NEVER_FADES,
                    0.5,
                ),
                WeatherSide::new(
                    CLEAR,
                    TransitionSide::Outgoing { progress },
                    NEVER_FADES,
                    0.5,
                ),
            ];

            let level = level_of(&sides, "bed");

            assert!((level - 1.0).abs() < 1e-6, "{level} at {progress}");
        }
    }

    #[test]
    fn levels_of_one_descriptor_add_up_and_stop_at_one() {
        let two_parts = [side(CLEAR, 0.3, 1.0, 1.0), side(RAIN, 0.2, 1.0, 1.0)];
        let too_much = [side(CLEAR, 0.8, 1.0, 1.0), side(RAIN, 0.8, 1.0, 1.0)];

        assert_eq!(level_of(&two_parts, "bed"), 0.5);
        assert_eq!(level_of(&too_much, "bed"), 1.0);
    }

    #[test]
    fn a_weather_the_table_does_not_know_is_an_error() {
        let error = loop_levels(&table(), &[settled(WeatherId::new(9))], true).unwrap_err();

        assert!(
            matches!(error, WeatherAudioError::UnknownWeather(9)),
            "{error:?}"
        );
    }

    #[test]
    fn the_gain_of_a_level_is_zero_at_full_level_and_falls_by_six_db_per_halving() {
        assert_eq!(level_db(1.0), 0.0);
        assert!((level_db(0.5) + 6.0206).abs() < 1e-3);
        assert!((level_db(0.1) + 20.0).abs() < 1e-3);
        assert_eq!(level_db(0.0), f32::NEG_INFINITY);
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
        fn every_listed_level_is_above_nothing_and_at_most_full(
            weights in proptest::collection::vec(any_fraction(), 1..4),
            precipitation in any_fraction(),
            wind in any_fraction(),
        ) {
            let sides: Vec<WeatherSide> = weights
                .into_iter()
                .zip([CLEAR, RAIN, STORM])
                .map(|(weight, weather)| side(weather, weight, precipitation, wind))
                .collect();

            for (descriptor, level) in levels(&sides) {
                prop_assert!(level > 0.0 && level <= 1.0, "{descriptor} at {level}");
            }
        }
    }
}
