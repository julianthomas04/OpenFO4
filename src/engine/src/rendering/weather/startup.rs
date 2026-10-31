//! What the weather of a world starts from: the weathers of its climate and the table its weather
//! cycle rolls them from.

use std::env::VarError;

use super::error::WeatherLibraryError;
use super::library::WeatherLibrary;
use crate::assets::db::DbClient;
use crate::weather::{WeatherRollTable, WeatherTableError};

/// Pins the seed the weather cycle rolls from, to roll the same weather again.
pub const WEATHER_SEED_VAR: &str = "OPENFO4_WEATHER_SEED";

/// Why a world starts without a weather cycle.
#[derive(Debug, thiserror::Error)]
pub enum StartupWeatherError {
    #[error("the weathers of the climate could not be loaded: {0}")]
    Library(#[from] WeatherLibraryError),
    #[error("the weathers of the climate cannot be rolled: {0}")]
    RollTable(#[from] WeatherTableError),
}

#[derive(Debug)]
pub struct StartupWeather {
    pub library: WeatherLibrary,
    /// The weathers of the library as the weather cycle rolls them.
    pub table: WeatherRollTable,
}

impl StartupWeather {
    pub fn load(db: &DbClient, worldspace: &str) -> Result<Self, StartupWeatherError> {
        Self::from_library(WeatherLibrary::load(db, worldspace)?)
    }

    pub fn from_library(library: WeatherLibrary) -> Result<Self, StartupWeatherError> {
        let table = library.roll_table()?;
        Ok(Self { library, table })
    }

    /// Boot-time entry point: a failure is logged and the world keeps its single static weather
    /// and no weather cycle.
    pub fn load_or_report(db_path: &str, worldspace: &str) -> Option<Self> {
        let db = match DbClient::new(db_path) {
            Ok(db) => db,
            Err(error) => {
                tracing::error!(
                    "weather cycle not started, database open failed: {error:#}; exteriors keep their single weather"
                );
                return None;
            }
        };
        match Self::load(&db, worldspace) {
            Ok(weather) => {
                tracing::info!(
                    "weather cycle for {worldspace}: {} weathers, climate volatility {}, starting on {}",
                    weather.library.weathers().len(),
                    weather.library.climate_volatility(),
                    weather
                        .library
                        .initial_weather()
                        .editor_id
                        .as_deref()
                        .unwrap_or("<no editor id>"),
                );
                Some(weather)
            }
            Err(error) => {
                tracing::error!(
                    "weather cycle not started: {error}; exteriors keep their single weather"
                );
                None
            }
        }
    }
}

/// The seed the weather cycle rolls from: the value of [`WEATHER_SEED_VAR`] (`pinned`) when it is
/// a seed, else `fresh`, which differs between runs. A pin that is no seed is reported.
pub fn cycle_seed(pinned: Result<String, VarError>, fresh: u64) -> u64 {
    let ignored = |reason: String| {
        tracing::warn!("{WEATHER_SEED_VAR} is ignored, {reason}; the weather rolls from {fresh}");
        fresh
    };
    match pinned {
        Ok(raw) => raw
            .trim()
            .parse()
            .unwrap_or_else(|error| ignored(format!("{raw:?} is not a whole number: {error}"))),
        Err(VarError::NotPresent) => fresh,
        Err(error @ VarError::NotUnicode(_)) => ignored(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;
    use crate::audio::test_support::SharedBuffer;
    use crate::rendering::weather::definition::fixtures::weather_def;
    use crate::rendering::weather::library::fixtures::library_of;

    #[test]
    fn the_cycle_rolls_the_weathers_of_the_library_it_starts_from() {
        let startup =
            StartupWeather::from_library(library_of(vec![weather_def("A"), weather_def("B")]))
                .unwrap();
        assert_eq!(startup.table.initial(), startup.library.initial());
        assert_eq!(startup.table.entries(), startup.library.entries());
    }

    #[test]
    fn without_a_pin_the_cycle_rolls_from_the_fresh_seed() {
        assert_eq!(cycle_seed(Err(VarError::NotPresent), 7), 7);
    }

    #[test]
    fn a_pinned_seed_is_the_one_the_cycle_rolls_from_whatever_the_fresh_one() {
        for (pin, seed) in [
            ("42", 42),
            (" 42\n", 42),
            ("0", 0),
            ("18446744073709551615", u64::MAX),
        ] {
            assert_eq!(cycle_seed(Ok(pin.to_owned()), 7), seed, "{pin:?}");
        }
    }

    #[test]
    fn a_pin_that_is_no_seed_is_reported_and_leaves_the_fresh_seed() {
        let unusable = Ok("0xFF".to_owned());
        for pin in [
            unusable,
            Ok(String::new()),
            Ok("-1".to_owned()),
            Ok("1.5".to_owned()),
            Ok("18446744073709551616".to_owned()),
            Err(VarError::NotUnicode(OsString::from("12"))),
        ] {
            let log = SharedBuffer::default();
            let guard = log.capture("warn");
            let seed = cycle_seed(pin.clone(), 7);
            drop(guard);
            let logged = log.drain();
            assert_eq!(seed, 7, "{pin:?}");
            assert!(
                logged.contains(WEATHER_SEED_VAR) && logged.contains("rolls from 7"),
                "{pin:?}: {logged}"
            );
        }
    }

    #[test]
    fn a_usable_pin_or_none_at_all_is_not_reported() {
        let log = SharedBuffer::default();
        let guard = log.capture("warn");
        cycle_seed(Ok("5".to_owned()), 7);
        cycle_seed(Err(VarError::NotPresent), 7);
        drop(guard);
        assert_eq!(log.drain(), "");
    }
}
