use crate::assets::db::schema::WEATHER_LIBRARY_TABLES_FIX;
use crate::rendering::lighting::exterior_weather::ExteriorWeatherError;

#[derive(Debug, thiserror::Error)]
pub enum WeatherLibraryError {
    #[error(
        "weather library columns missing (database predates the weather library conversion); fix: run the installer with `{WEATHER_LIBRARY_TABLES_FIX}`"
    )]
    SchemaOutdated,
    #[error(transparent)]
    Lighting(#[from] ExteriorWeatherError),
    #[error("climate {climate} has no weather without a global gate")]
    NoUngatedWeather { climate: String },
    #[error("climate {climate} has no volatility")]
    VolatilityMissing { climate: String },
    #[error("climate {climate} volatility {value} does not fit a byte")]
    VolatilityOutOfRange { climate: String, value: i64 },
    #[error("weather {weather} has no {field} data")]
    MissingField {
        weather: String,
        field: &'static str,
    },
    #[error("weather {weather} {field} JSON is malformed: {source}")]
    Malformed {
        weather: String,
        field: &'static str,
        source: serde_json::Error,
    },
    #[error("weather {weather} {field} has {found} entries, expected {expected}")]
    WrongCount {
        weather: String,
        field: &'static str,
        expected: usize,
        found: usize,
    },
    #[error("weather {weather} cloud layer {layer} has no {slot} slot in its {entry}")]
    MissingCloudSlot {
        weather: String,
        layer: usize,
        entry: &'static str,
        slot: &'static str,
    },
    #[error("weather {weather} {field} has unknown flag bits {bits:#x}")]
    UnknownFlagBits {
        weather: String,
        field: &'static str,
        bits: u32,
    },
    #[error("weather {weather} names particle geometry {particle_geometry} which does not exist")]
    ParticleGeometryNotFound {
        weather: String,
        particle_geometry: String,
    },
    #[error("weather {weather} particle geometry {particle_geometry} has no {field} data")]
    ParticleGeometryMissingField {
        weather: String,
        particle_geometry: String,
        field: &'static str,
    },
    #[error(
        "weather {weather} particle geometry {particle_geometry} has unknown precipitation type {kind}"
    )]
    UnknownPrecipitationType {
        weather: String,
        particle_geometry: String,
        kind: u8,
    },
    #[error("database query failed: {0}")]
    Query(#[from] rusqlite::Error),
}

pub(super) fn parse_json<T: serde::de::DeserializeOwned>(
    weather: &str,
    field: &'static str,
    json: &str,
) -> Result<T, WeatherLibraryError> {
    serde_json::from_str(json).map_err(|source| WeatherLibraryError::Malformed {
        weather: weather.to_string(),
        field,
        source,
    })
}

pub(super) fn require_field<'a>(
    weather: &str,
    field: &'static str,
    json: Option<&'a str>,
) -> Result<&'a str, WeatherLibraryError> {
    json.ok_or_else(|| WeatherLibraryError::MissingField {
        weather: weather.to_string(),
        field,
    })
}
