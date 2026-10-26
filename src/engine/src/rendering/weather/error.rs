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
    #[error("climate {climate} has no volatility")]
    VolatilityMissing { climate: String },
    #[error("climate {climate} volatility {value} does not fit a byte")]
    VolatilityOutOfRange { climate: String, value: i64 },
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
    #[error("weather {weather} cloud layer {layer} has {present} but no {absent}")]
    UnpairedCloudValues {
        weather: String,
        layer: usize,
        present: &'static str,
        absent: &'static str,
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
