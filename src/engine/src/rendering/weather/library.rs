use std::collections::HashMap;

use super::definition::WeatherDef;
use super::error::WeatherLibraryError;
use crate::assets::db::DbClient;
use crate::rendering::lighting::exterior_weather::{
    ClimateInfo, ClimateWeather, ExteriorWeatherError, load_worldspace_climate, most_likely_index,
};

/// Index of a weather in its [`WeatherLibrary`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WeatherId(usize);

impl WeatherId {
    pub const fn index(self) -> usize {
        self.0
    }
}

/// One weather a climate can pick and its relative chance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClimateEntry {
    pub weather: WeatherId,
    pub chance: u32,
}

/// The climate of a worldspace with every weather it can pick.
#[derive(Debug, Clone, PartialEq)]
pub struct WeatherLibrary {
    climate_volatility: u8,
    entries: Vec<ClimateEntry>,
    weathers: Vec<WeatherDef>,
    initial: WeatherId,
}

impl WeatherLibrary {
    pub fn load(db: &DbClient, worldspace: &str) -> Result<Self, WeatherLibraryError> {
        let schema = db.schema();
        if !schema.weather_columns_present {
            return Err(ExteriorWeatherError::ColumnsMissing.into());
        }
        if !schema.weather_library_columns_present {
            return Err(WeatherLibraryError::SchemaOutdated);
        }
        let climate = load_worldspace_climate(db, worldspace)?;
        let volatility = load_volatility(db, &climate.name)?;
        Self::build(&climate, volatility, |key| {
            WeatherDef::load(db, key, &climate)
        })
    }

    /// The climate's raw volatility byte: how quickly the climate changes its weather.
    pub fn climate_volatility(&self) -> u8 {
        self.climate_volatility
    }

    pub fn entries(&self) -> &[ClimateEntry] {
        &self.entries
    }

    pub fn weathers(&self) -> &[WeatherDef] {
        &self.weathers
    }

    pub fn weather(&self, id: WeatherId) -> &WeatherDef {
        &self.weathers[id.index()]
    }

    /// The most likely weather of the climate, the one lighting starts from.
    pub fn initial_weather(&self) -> WeatherId {
        self.initial
    }

    /// Entries gated by a global are dropped: global values are not evaluated yet.
    fn build(
        climate: &ClimateInfo,
        volatility: u8,
        mut load_weather: impl FnMut(&str) -> Result<WeatherDef, WeatherLibraryError>,
    ) -> Result<Self, WeatherLibraryError> {
        let ungated = ungated_entries(climate);
        let mut weathers: Vec<WeatherDef> = Vec::new();
        let mut ids: HashMap<&str, WeatherId> = HashMap::new();
        let mut entries = Vec::with_capacity(ungated.len());
        for entry in ungated {
            let id = if let Some(id) = ids.get(entry.weather.as_str()) {
                *id
            } else {
                let id = WeatherId(weathers.len());
                weathers.push(load_weather(&entry.weather)?);
                ids.insert(&entry.weather, id);
                id
            };
            entries.push(ClimateEntry {
                weather: id,
                chance: entry.chance,
            });
        }
        let initial = most_likely_index(&entries, |entry| entry.chance)
            .map(|index| entries[index].weather)
            .ok_or_else(|| WeatherLibraryError::NoUngatedWeather {
                climate: climate.name.clone(),
            })?;
        Ok(Self {
            climate_volatility: volatility,
            entries,
            weathers,
            initial,
        })
    }
}

fn ungated_entries(climate: &ClimateInfo) -> Vec<&ClimateWeather> {
    let (ungated, gated): (Vec<_>, Vec<_>) = climate
        .entries
        .iter()
        .partition(|entry| entry.global.is_none());
    if !gated.is_empty() {
        let gated: Vec<String> = gated
            .iter()
            .map(|entry| {
                format!(
                    "{} (global {})",
                    entry.weather,
                    entry.global.as_deref().unwrap_or_default()
                )
            })
            .collect();
        tracing::warn!(
            "climate {} has weathers gated by a global, which is not supported and ignored: {}",
            climate.name,
            gated.join(", ")
        );
    }
    ungated
}

fn load_volatility(db: &DbClient, climate: &str) -> Result<u8, WeatherLibraryError> {
    let value = db
        .get_climate_volatility(climate)
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => ExteriorWeatherError::ClimateNotFound {
                climate: climate.to_string(),
            }
            .into(),
            other => WeatherLibraryError::from(other),
        })?
        .ok_or_else(|| WeatherLibraryError::VolatilityMissing {
            climate: climate.to_string(),
        })?;
    checked_volatility(climate, value)
}

fn checked_volatility(climate: &str, value: i64) -> Result<u8, WeatherLibraryError> {
    u8::try_from(value).map_err(|_| WeatherLibraryError::VolatilityOutOfRange {
        climate: climate.to_string(),
        value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::lighting::exterior_weather::fixtures::climate_with_entries;
    use crate::rendering::weather::definition::fixtures::weather_def;

    #[allow(
        clippy::unnecessary_wraps,
        reason = "it stands in for the database loader"
    )]
    fn load_fixture(key: &str) -> Result<WeatherDef, WeatherLibraryError> {
        Ok(weather_def(key))
    }

    fn build(entries: &str) -> Result<WeatherLibrary, WeatherLibraryError> {
        WeatherLibrary::build(&climate_with_entries(entries), 30, load_fixture)
    }

    const TWO_WEATHERS: &str = r#"[{"Weather":"A","Chance":10},{"Weather":"B","Chance":40}]"#;

    #[test]
    fn every_entry_gets_its_weather_and_chance() {
        let library = build(TWO_WEATHERS).unwrap();
        assert_eq!(library.climate_volatility(), 30);
        let chances: Vec<u32> = library.entries().iter().map(|entry| entry.chance).collect();
        assert_eq!(chances, [10, 40]);
        let keys: Vec<&str> = library
            .entries()
            .iter()
            .map(|entry| library.weather(entry.weather).form_key.as_str())
            .collect();
        assert_eq!(keys, ["A", "B"]);
    }

    #[test]
    fn the_initial_weather_is_the_most_likely_entry() {
        let library = build(TWO_WEATHERS).unwrap();
        assert_eq!(library.weather(library.initial_weather()).form_key, "B");
    }

    #[test]
    fn among_equal_chances_the_first_listed_weather_is_initial() {
        let library = build(r#"[{"Weather":"A","Chance":5},{"Weather":"B","Chance":5}]"#).unwrap();
        assert_eq!(library.weather(library.initial_weather()).form_key, "A");
    }

    #[test]
    fn a_weather_listed_twice_is_loaded_once_and_shared_by_both_entries() {
        let mut loads = Vec::new();
        let library = WeatherLibrary::build(
            &climate_with_entries(
                r#"[{"Weather":"A","Chance":1},{"Weather":"B","Chance":2},{"Weather":"A","Chance":3}]"#,
            ),
            0,
            |key| {
                loads.push(key.to_owned());
                load_fixture(key)
            },
        )
        .unwrap();
        assert_eq!(loads, ["A", "B"]);
        assert_eq!(library.weathers().len(), 2);
        assert_eq!(library.entries().len(), 3);
        assert_eq!(library.entries()[0].weather, library.entries()[2].weather);
        assert_eq!(library.weather(library.initial_weather()).form_key, "A");
    }

    #[test]
    fn entries_gated_by_a_global_are_ignored_and_never_loaded() {
        let mut loads = Vec::new();
        let library = WeatherLibrary::build(
            &climate_with_entries(
                r#"[{"Weather":"A","Chance":50,"Global":"00FF:Fallout4.esm"},{"Weather":"B","Chance":10,"Global":null},{"Weather":"C","Chance":20}]"#,
            ),
            0,
            |key| {
                loads.push(key.to_owned());
                load_fixture(key)
            },
        )
        .unwrap();
        assert_eq!(loads, ["B", "C"]);
        assert_eq!(library.entries().len(), 2);
        assert_eq!(library.weather(library.initial_weather()).form_key, "C");
    }

    #[test]
    fn a_climate_with_only_gated_entries_is_an_error_naming_it() {
        let error =
            build(r#"[{"Weather":"A","Chance":50,"Global":"00FF:Fallout4.esm"}]"#).unwrap_err();
        assert!(
            matches!(&error, WeatherLibraryError::NoUngatedWeather { climate } if climate == "DefaultClimate"),
            "{error:?}"
        );
    }

    #[test]
    fn a_weather_that_fails_to_load_fails_the_library_with_its_error() {
        let error = WeatherLibrary::build(&climate_with_entries(TWO_WEATHERS), 0, |key| {
            if key == "B" {
                Err(WeatherLibraryError::MissingField {
                    weather: key.to_owned(),
                    field: "data",
                })
            } else {
                load_fixture(key)
            }
        })
        .unwrap_err();
        assert!(
            matches!(&error, WeatherLibraryError::MissingField { weather, .. } if weather == "B"),
            "{error:?}"
        );
    }

    #[test]
    fn volatility_is_a_raw_byte() {
        assert_eq!(checked_volatility("C", 0).unwrap(), 0);
        assert_eq!(checked_volatility("C", 175).unwrap(), 175);
        assert_eq!(checked_volatility("C", 255).unwrap(), 255);
        for value in [-1, 256, i64::MAX] {
            assert!(
                matches!(
                    checked_volatility("C", value),
                    Err(WeatherLibraryError::VolatilityOutOfRange { value: v, .. }) if v == value
                ),
                "{value}"
            );
        }
    }
}
