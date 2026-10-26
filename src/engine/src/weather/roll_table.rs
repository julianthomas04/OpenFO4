use thiserror::Error;

use super::id::{ClimateEntry, WeatherId};
use super::lightning::LightningParams;

/// What the weather cycle reads about one weather.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeatherRollTraits {
    /// The record's unscaled transition byte; how long a transition takes is the cycle's call.
    pub trans_delta_raw: u8,
    /// How much faster the climate rolls while this weather is current; 1.0 when the weather has
    /// no multiplier.
    pub volatility_mult: f32,
    pub lightning: LightningParams,
    pub has_thunder_sound: bool,
}

#[derive(Debug, Error, PartialEq)]
pub enum WeatherTableError {
    #[error("the climate lists no weather")]
    NoEntries,
    #[error("{weather:?} is not one of the {count} weathers of the table")]
    UnknownWeather { weather: WeatherId, count: usize },
    #[error("no weather of the climate has a chance above zero")]
    NoChance,
    #[error("the chances of the climate add up to more than a draw can cover")]
    ChanceOverflow,
    #[error(
        "{weather:?} has the volatility multiplier {value}, which is not a positive finite number"
    )]
    InvalidVolatilityMult { weather: WeatherId, value: f32 },
}

/// The weathers of a climate with the chance of each and the values the weather cycle needs to
/// roll and blend between them. It is checked on construction: every entry names a weather of the
/// table, some entry has a chance and every multiplier is usable.
#[derive(Debug, Clone, PartialEq)]
pub struct WeatherRollTable {
    climate_volatility: u8,
    entries: Vec<ClimateEntry>,
    weathers: Vec<WeatherRollTraits>,
    initial: WeatherId,
    total_chance: usize,
}

impl WeatherRollTable {
    /// `weathers` is indexed by [`WeatherId::index`]. `climate_volatility` is the climate's raw
    /// volatility byte and `initial` the weather the cycle starts on.
    pub fn new(
        climate_volatility: u8,
        entries: Vec<ClimateEntry>,
        weathers: Vec<WeatherRollTraits>,
        initial: WeatherId,
    ) -> Result<Self, WeatherTableError> {
        if entries.is_empty() {
            return Err(WeatherTableError::NoEntries);
        }
        let count = weathers.len();
        let known = |weather: WeatherId| {
            if weather.index() < count {
                Ok(())
            } else {
                Err(WeatherTableError::UnknownWeather { weather, count })
            }
        };
        known(initial)?;
        for entry in &entries {
            known(entry.weather)?;
        }
        let total_chance = entries
            .iter()
            .try_fold(0_usize, |sum, entry| sum.checked_add(entry.chance as usize))
            .ok_or(WeatherTableError::ChanceOverflow)?;
        if total_chance == 0 {
            return Err(WeatherTableError::NoChance);
        }
        for (index, traits) in weathers.iter().enumerate() {
            let value = traits.volatility_mult;
            if !(value.is_finite() && value > 0.0) {
                return Err(WeatherTableError::InvalidVolatilityMult {
                    weather: WeatherId::new(index),
                    value,
                });
            }
        }
        Ok(Self {
            climate_volatility,
            entries,
            weathers,
            initial,
            total_chance,
        })
    }

    pub fn climate_volatility(&self) -> u8 {
        self.climate_volatility
    }

    pub fn entries(&self) -> &[ClimateEntry] {
        &self.entries
    }

    /// The sum of the chances of the entries; above zero.
    pub fn total_chance(&self) -> usize {
        self.total_chance
    }

    pub fn initial(&self) -> WeatherId {
        self.initial
    }

    pub fn traits(&self, weather: WeatherId) -> Option<&WeatherRollTraits> {
        self.weathers.get(weather.index())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::weather::FadeRange;

    fn traits() -> WeatherRollTraits {
        WeatherRollTraits {
            trans_delta_raw: 25,
            volatility_mult: 1.0,
            lightning: LightningParams {
                fade: FadeRange {
                    begin_fade_in: 0.0,
                    end_fade_out: 1.0,
                },
                frequency: 1.0,
                color_srgb: [255, 255, 255],
            },
            has_thunder_sound: false,
        }
    }

    fn entry(weather: usize, chance: u32) -> ClimateEntry {
        ClimateEntry {
            weather: WeatherId::new(weather),
            chance,
        }
    }

    fn table(
        entries: Vec<ClimateEntry>,
        weathers: usize,
    ) -> Result<WeatherRollTable, WeatherTableError> {
        WeatherRollTable::new(175, entries, vec![traits(); weathers], WeatherId::new(0))
    }

    #[test]
    fn a_valid_table_exposes_what_it_was_built_from() {
        let table = table(vec![entry(0, 51), entry(1, 11), entry(0, 40)], 2).unwrap();
        assert_eq!(table.climate_volatility(), 175);
        assert_eq!(table.entries().len(), 3);
        assert_eq!(table.total_chance(), 102);
        assert_eq!(table.initial(), WeatherId::new(0));
        assert_eq!(table.traits(WeatherId::new(1)), Some(&traits()));
    }

    #[test]
    fn a_weather_outside_the_table_has_no_traits() {
        let table = table(vec![entry(0, 1)], 1).unwrap();
        assert_eq!(table.traits(WeatherId::new(1)), None);
    }

    #[test]
    fn a_climate_without_entries_is_rejected() {
        assert_eq!(table(vec![], 1), Err(WeatherTableError::NoEntries));
    }

    #[test]
    fn an_entry_or_initial_weather_outside_the_table_is_rejected() {
        assert_eq!(
            table(vec![entry(0, 1), entry(2, 1)], 2),
            Err(WeatherTableError::UnknownWeather {
                weather: WeatherId::new(2),
                count: 2
            })
        );
        assert_eq!(
            WeatherRollTable::new(0, vec![entry(0, 1)], vec![traits()], WeatherId::new(1)),
            Err(WeatherTableError::UnknownWeather {
                weather: WeatherId::new(1),
                count: 1
            })
        );
    }

    #[test]
    fn a_climate_where_nothing_has_a_chance_is_rejected() {
        assert_eq!(
            table(vec![entry(0, 0), entry(1, 0)], 2),
            Err(WeatherTableError::NoChance)
        );
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn chances_are_summed_without_wrapping_at_32_bits() {
        let table = table(vec![entry(0, u32::MAX), entry(0, u32::MAX)], 1).unwrap();
        assert_eq!(table.total_chance(), 2 * u32::MAX as usize);
    }

    #[cfg(target_pointer_width = "32")]
    #[test]
    fn chances_that_overflow_the_total_are_rejected() {
        let result = table(vec![entry(0, u32::MAX), entry(0, u32::MAX)], 1);
        assert_eq!(result, Err(WeatherTableError::ChanceOverflow));
    }

    #[test]
    fn a_volatility_multiplier_that_is_not_a_positive_number_is_rejected() {
        for value in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let mut weathers = vec![traits(); 2];
            weathers[1].volatility_mult = value;
            let result = WeatherRollTable::new(0, vec![entry(0, 1)], weathers, WeatherId::new(0));
            assert!(
                matches!(
                    result,
                    Err(WeatherTableError::InvalidVolatilityMult { weather, .. })
                        if weather == WeatherId::new(1)
                ),
                "{value}"
            );
        }
    }
}
