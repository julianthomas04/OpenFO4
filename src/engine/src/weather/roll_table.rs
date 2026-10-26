use std::num::NonZeroUsize;

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

/// A weather of a [`WeatherRollTable`] with what the weather cycle reads about it. Only the table
/// makes them, so holding one proves that the weather is in the table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TableWeather {
    id: WeatherId,
    traits: WeatherRollTraits,
}

impl TableWeather {
    pub fn id(&self) -> WeatherId {
        self.id
    }

    pub fn traits(&self) -> &WeatherRollTraits {
        &self.traits
    }
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
    initial: TableWeather,
    shares: Vec<Share>,
    total_chance: NonZeroUsize,
}

/// The tickets of a roll that one entry of the climate owns.
#[derive(Debug, Clone, PartialEq)]
struct Share {
    chance: usize,
    weather: TableWeather,
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
        let resolve = |id: WeatherId| {
            weathers
                .get(id.index())
                .map(|&traits| TableWeather { id, traits })
                .ok_or(WeatherTableError::UnknownWeather { weather: id, count })
        };
        let initial = resolve(initial)?;
        let shares = entries
            .iter()
            .map(|entry| {
                Ok(Share {
                    chance: entry.chance as usize,
                    weather: resolve(entry.weather)?,
                })
            })
            .collect::<Result<Vec<_>, WeatherTableError>>()?;
        let total_chance = shares
            .iter()
            .try_fold(0_usize, |sum, share| sum.checked_add(share.chance))
            .ok_or(WeatherTableError::ChanceOverflow)?;
        let total_chance = NonZeroUsize::new(total_chance).ok_or(WeatherTableError::NoChance)?;
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
            shares,
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
        self.total_chance.get()
    }

    pub fn initial(&self) -> WeatherId {
        self.initial.id
    }

    /// The weather the cycle starts on.
    pub fn initial_weather(&self) -> TableWeather {
        self.initial
    }

    pub fn traits(&self, weather: WeatherId) -> Option<&WeatherRollTraits> {
        self.weathers.get(weather.index())
    }

    /// A weather of the table, or `None` for an id that is not in it.
    pub fn weather(&self, id: WeatherId) -> Option<TableWeather> {
        self.traits(id).map(|&traits| TableWeather { id, traits })
    }

    /// The weather that a ticket of a roll selects. The entries own consecutive tickets, as many
    /// as their chance, so each of the `total_chance` tickets picks one entry and a weather
    /// listed twice owns the tickets of both. A ticket beyond the total wraps around.
    pub fn pick(&self, ticket: usize) -> TableWeather {
        let mut remaining = ticket % self.total_chance;
        let mut picked = self.initial;
        for share in &self.shares {
            picked = share.weather;
            if remaining < share.chance {
                break;
            }
            remaining -= share.chance;
        }
        picked
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

    /// Weathers that tell each other apart by their transition byte, which is their index.
    fn distinct_weathers(count: u8) -> Vec<WeatherRollTraits> {
        (0..count)
            .map(|index| WeatherRollTraits {
                trans_delta_raw: index,
                ..traits()
            })
            .collect()
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

    /// A table of the weathers `0..chances.len()` with the given chances, in that order.
    fn table_of_chances(chances: &[u32]) -> WeatherRollTable {
        let entries = chances
            .iter()
            .enumerate()
            .map(|(weather, &chance)| entry(weather, chance))
            .collect();
        WeatherRollTable::new(
            0,
            entries,
            distinct_weathers(chances.len() as u8),
            WeatherId::new(0),
        )
        .unwrap()
    }

    fn picks(table: &WeatherRollTable, tickets: impl IntoIterator<Item = usize>) -> Vec<usize> {
        tickets
            .into_iter()
            .map(|ticket| table.pick(ticket).id().index())
            .collect()
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
        assert_eq!(table.weather(WeatherId::new(1)), None);
    }

    #[test]
    fn a_weather_of_the_table_comes_with_its_own_traits() {
        let table = WeatherRollTable::new(
            0,
            vec![entry(1, 1)],
            distinct_weathers(3),
            WeatherId::new(2),
        )
        .unwrap();
        let weather = table.weather(WeatherId::new(1)).unwrap();
        assert_eq!(weather.id(), WeatherId::new(1));
        assert_eq!(weather.traits().trans_delta_raw, 1);
        let initial = table.initial_weather();
        assert_eq!(initial.id(), WeatherId::new(2));
        assert_eq!(initial.traits().trans_delta_raw, 2);
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

    #[test]
    fn a_ticket_selects_the_entry_whose_chances_cover_it() {
        let table = table_of_chances(&[51, 13, 11, 6, 6, 6, 5, 4]);
        let tickets = [
            0, 50, 51, 63, 64, 74, 75, 80, 81, 86, 87, 92, 93, 97, 98, 101,
        ];
        assert_eq!(
            picks(&table, tickets),
            [0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7]
        );
    }

    #[test]
    fn a_picked_weather_comes_with_its_own_traits() {
        let table = table_of_chances(&[1, 1, 1]);
        for ticket in 0..3 {
            let weather = table.pick(ticket);
            assert_eq!(
                usize::from(weather.traits().trans_delta_raw),
                weather.id().index()
            );
        }
    }

    #[test]
    fn an_entry_without_a_chance_is_never_picked() {
        let table = table_of_chances(&[0, 5, 0, 5]);
        assert_eq!(picks(&table, 0..10), [1, 1, 1, 1, 1, 3, 3, 3, 3, 3]);
    }

    #[test]
    fn a_weather_listed_twice_is_picked_by_the_sum_of_its_chances() {
        let table = WeatherRollTable::new(
            0,
            vec![entry(0, 1), entry(1, 2), entry(0, 3)],
            distinct_weathers(2),
            WeatherId::new(0),
        )
        .unwrap();
        assert_eq!(picks(&table, 0..6), [0, 1, 1, 0, 0, 0]);
    }

    #[test]
    fn a_ticket_beyond_the_total_wraps_around() {
        let table = table_of_chances(&[3, 0, 2]);
        assert_eq!(picks(&table, 5..10), picks(&table, 0..5));
        assert_eq!(
            table.pick(usize::MAX),
            table.pick(usize::MAX % table.total_chance())
        );
    }
}
