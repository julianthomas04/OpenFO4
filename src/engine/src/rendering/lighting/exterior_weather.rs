use std::collections::HashMap;

use glam::Vec3;
use serde::Deserialize;

use super::ambient::AmbientCube;
use super::cell_lighting::{RawColor, bethesda_faces_to_engine_order};
use super::fog::{HeightFogError, RawHeightFog};
use super::time_of_day::{
    ClimateMoon, ClimateMoonError, ClimateSky, ClimateSkyError, ClimateSunTextures, ClimateTimes,
    ClimateTimesError, TimeSlot,
};
pub use super::weather_slots::{ExteriorWeather, ExteriorWeatherSlots, ImageSpaceHdr};
use super::weather_slots::{FogColors, FogRange, SlotValues, WeatherFog};
use crate::assets::db::DbClient;
use crate::assets::db::weather::WeatherRow;
use crate::rendering::post::depth_of_field::ImageSpaceDepthOfField;
use crate::rendering::world_streamer::SCALE;

/// The database stores fog distances for these two slots only.
const FOG_DAY_SLOT: TimeSlot = TimeSlot::Day;
const FOG_NIGHT_SLOT: TimeSlot = TimeSlot::Night;

#[derive(Debug, thiserror::Error)]
pub enum ExteriorWeatherError {
    #[error("weather columns missing (database predates weather conversion)")]
    ColumnsMissing,
    #[error("worldspace {worldspace} not found")]
    WorldspaceNotFound { worldspace: String },
    #[error("worldspace {worldspace} has no climate")]
    WorldspaceWithoutClimate { worldspace: String },
    #[error("climate {climate} not found")]
    ClimateNotFound { climate: String },
    #[error("climate {climate} has no data")]
    ClimateWithoutData { climate: String },
    #[error("climate {climate} lists no weathers")]
    EmptyWeatherList { climate: String },
    #[error("climate {climate} has no weather without a global gate")]
    NoUngatedWeather { climate: String },
    #[error("climate {climate} has unusable sunrise and sunset times: {source}")]
    ClimateTimes {
        climate: String,
        source: ClimateTimesError,
    },
    #[error("climate {climate} has unusable sky data: {source}")]
    ClimateSky {
        climate: String,
        source: ClimateSkyError,
    },
    #[error("climate {climate} has an unusable moon: {source}")]
    ClimateMoon {
        climate: String,
        source: ClimateMoonError,
    },
    #[error("weather {weather} not found")]
    WeatherNotFound { weather: String },
    #[error("weather {weather} has no {field} data")]
    MissingField {
        weather: String,
        field: &'static str,
    },
    #[error("weather {weather} has no fog data (database predates fog conversion)")]
    FogMissing { weather: String },
    #[error(
        "weather {weather} {slot} fog range is not increasing (near {near}, far {far} Bethesda units)"
    )]
    InvertedFogRange {
        weather: String,
        slot: &'static str,
        near: f32,
        far: f32,
    },
    #[error("weather {weather} {slot} fog: {source}")]
    HeightFog {
        weather: String,
        slot: &'static str,
        source: HeightFogError,
    },
    #[error("weather {weather} has no {slot} slot for {entry}")]
    MissingTimeSlot {
        weather: String,
        entry: String,
        slot: &'static str,
    },
    #[error("image space {image_space} not found")]
    ImageSpaceNotFound { image_space: String },
    #[error("image space {image_space} has no data")]
    ImageSpaceWithoutData { image_space: String },
    #[error("weather {weather} has malformed {field} JSON: {source}")]
    Malformed {
        weather: String,
        field: &'static str,
        source: serde_json::Error,
    },
    #[error("climate {climate} has malformed data JSON: {source}")]
    ClimateMalformed {
        climate: String,
        source: serde_json::Error,
    },
    #[error("image space {image_space} has malformed data JSON: {source}")]
    ImageSpaceMalformed {
        image_space: String,
        source: serde_json::Error,
    },
    #[error("database query failed: {0}")]
    Query(#[from] rusqlite::Error),
}

#[derive(Debug, Deserialize)]
struct ClimateData {
    #[serde(rename = "Weathers")]
    weathers: Vec<ClimateWeather>,
}

/// One weather a climate can pick, with the global that gates it when it has one.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct ClimateWeather {
    #[serde(rename = "Weather")]
    pub(crate) weather: String,
    #[serde(rename = "Chance")]
    pub(crate) chance: u32,
    #[serde(rename = "Global")]
    global: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawAmbientColors {
    x_plus: RawColor,
    x_minus: RawColor,
    y_plus: RawColor,
    y_minus: RawColor,
    z_plus: RawColor,
    z_minus: RawColor,
    specular: RawColor,
    scale: f32,
}

/// One slot of the fog JSON. Databases from before the height-fog export have no height
/// fields; their fog stays the same at every height.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawFog {
    near: f32,
    far: f32,
    power: f32,
    max: f32,
    near_height_mid: Option<f32>,
    near_height_range: Option<f32>,
    far_height_mid: Option<f32>,
    far_height_range: Option<f32>,
    high_density_scale: Option<f32>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct ImageSpaceData {
    #[serde(rename = "DepthOfField")]
    depth_of_field: Option<ImageSpaceDepthOfField>,
    #[serde(rename = "Hdr")]
    hdr: ImageSpaceHdr,
}

type ColorGroups = HashMap<String, HashMap<String, RawColor>>;
type FogSlots = HashMap<String, RawFog>;
type DirectionalAmbient = HashMap<String, RawAmbientColors>;

/// The sunrise and sunset times and the moon of the climate a weather belongs to.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClimateInfo {
    pub(crate) name: String,
    /// The weathers the climate can pick, in its order. Entries gated by a global are left out:
    /// global values are not evaluated yet.
    entries: Vec<ClimateWeather>,
    /// The index in `entries` of the most likely weather, the one lighting and the weather cycle
    /// start from.
    initial: usize,
    times: ClimateTimes,
    sky: ClimateSky,
    moon: Option<ClimateMoon>,
    /// Not in the climate JSON: [`load_climate`] reads the columns.
    sun_textures: ClimateSunTextures,
}

impl ClimateInfo {
    pub(crate) fn entries(&self) -> &[ClimateWeather] {
        &self.entries
    }

    /// The index in [`Self::entries`] of the weather lighting and the weather cycle start from.
    pub(crate) fn initial_index(&self) -> usize {
        self.initial
    }

    fn initial_weather(&self) -> &str {
        &self.entries[self.initial].weather
    }
}

impl ExteriorWeatherSlots {
    /// Boot-time entry point: a failure is logged and exteriors keep the
    /// built-in defaults.
    pub fn load_or_report(db_path: &str, worldspace: &str) -> Option<Self> {
        let db = match DbClient::new(db_path) {
            Ok(db) => db,
            Err(error) => {
                tracing::error!(
                    "exterior weather not loaded, database open failed: {error:#}; exteriors use default lighting"
                );
                return None;
            }
        };
        match Self::load(&db, worldspace) {
            Ok(slots) => {
                let initial = slots.initial();
                tracing::info!(
                    "exterior weather {} ({}) for {worldspace}, all {} time slots, climate {:?}, moon {:?}; initial slot: sun light {:?}, sun disc {:?}, hdr {:?}, depth of field {:?}, sky upper {:?}, sky lower {:?}, horizon {:?}, stars {:?}, ambient cube {:?}, ambient specular {:?}, fog source {}",
                    slots.editor_id().unwrap_or("<no editor id>"),
                    slots.form_key(),
                    TimeSlot::COUNT,
                    slots.climate_times(),
                    slots.moon(),
                    initial.sun_light,
                    initial.sun_disc,
                    initial.hdr,
                    initial.depth_of_field,
                    initial.sky_upper,
                    initial.sky_lower,
                    initial.horizon,
                    initial.stars,
                    initial.ambient_cube.0,
                    initial.ambient_specular,
                    initial
                        .fog
                        .map_or_else(|| "default".to_string(), |fog| format!("weather {fog:?}")),
                );
                Some(slots)
            }
            Err(error) => {
                tracing::error!(
                    "exterior weather not loaded: {error}; exteriors use default lighting"
                );
                None
            }
        }
    }

    pub fn load(db: &DbClient, worldspace: &str) -> Result<Self, ExteriorWeatherError> {
        if !db.schema().weather_columns_present {
            return Err(ExteriorWeatherError::ColumnsMissing);
        }
        let climate = load_worldspace_climate(db, worldspace)?;
        Self::load_weather(db, climate.initial_weather(), &climate)
    }

    /// One weather of `climate` with all its slots.
    pub(crate) fn load_weather(
        db: &DbClient,
        weather_key: &str,
        climate: &ClimateInfo,
    ) -> Result<Self, ExteriorWeatherError> {
        let row = db.get_weather(weather_key).map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => ExteriorWeatherError::WeatherNotFound {
                weather: weather_key.to_string(),
            },
            other => other.into(),
        })?;
        let image_spaces = load_image_spaces(db, &row)?;
        Self::from_row(row, &image_spaces, climate)
    }

    fn from_row(
        row: WeatherRow,
        image_spaces: &[ImageSpaceData; TimeSlot::COUNT],
        climate: &ClimateInfo,
    ) -> Result<Self, ExteriorWeatherError> {
        let WeatherRow {
            form_key,
            editor_id,
            colors,
            directional_ambient,
            image_spaces: _,
            fog,
        } = row;
        let colors: ColorGroups = parse_field(&form_key, "colors", colors.as_deref())?;
        let ambient: DirectionalAmbient = parse_field(
            &form_key,
            "directional ambient",
            directional_ambient.as_deref(),
        )?;
        let slots = TimeSlot::try_map_all(|slot| {
            load_slot(
                &form_key,
                slot,
                &colors,
                &ambient,
                image_spaces[slot.index()],
            )
        })?;
        let fog = match load_fog(&form_key, fog, &colors) {
            Ok(fog) => Some(fog),
            Err(error @ ExteriorWeatherError::FogMissing { .. }) => {
                tracing::warn!("{error}; exterior fog keeps its defaults");
                None
            }
            Err(error) => return Err(error),
        };
        Ok(Self::new(
            &form_key,
            editor_id.as_deref(),
            slots,
            fog,
            climate.times,
            climate.sky.clone(),
            climate.moon,
        )
        .with_sun_textures(climate.sun_textures.clone()))
    }
}

fn load_slot(
    weather: &str,
    slot: TimeSlot,
    colors: &ColorGroups,
    ambient: &DirectionalAmbient,
    image_space: ImageSpaceData,
) -> Result<SlotValues, ExteriorWeatherError> {
    let group = |name: &str| color_in_slot(weather, colors, name, slot);
    let dalc = ambient
        .get(slot.json_key())
        .ok_or_else(|| missing_slot(weather, "directional ambient", slot))?;
    let ambient_cube = AmbientCube(
        bethesda_faces_to_engine_order([
            dalc.x_plus,
            dalc.x_minus,
            dalc.y_plus,
            dalc.y_minus,
            dalc.z_plus,
            dalc.z_minus,
        ])
        .map(|face| face.linear() * dalc.scale),
    );
    Ok(SlotValues {
        sun_light: group("Sunlight")?,
        sun_disc: group("Sun")?,
        sun_glare: group("SunGlare")?,
        sky_upper: group("SkyUpper")?,
        sky_lower: group("SkyLower")?,
        horizon: group("Horizon")?,
        stars: group("Stars")?,
        ambient_cube,
        ambient_specular: dalc.specular.linear(),
        hdr: image_space.hdr,
        depth_of_field: image_space.depth_of_field,
    })
}

fn color_in_slot(
    weather: &str,
    colors: &ColorGroups,
    group: &str,
    slot: TimeSlot,
) -> Result<Vec3, ExteriorWeatherError> {
    colors
        .get(group)
        .and_then(|slots| slots.get(slot.json_key()))
        .copied()
        .map(RawColor::linear)
        .ok_or_else(|| missing_slot(weather, group, slot))
}

fn load_fog(
    weather: &str,
    json: Option<String>,
    colors: &ColorGroups,
) -> Result<WeatherFog, ExteriorWeatherError> {
    let json = json.ok_or_else(|| ExteriorWeatherError::FogMissing {
        weather: weather.to_string(),
    })?;
    let ranges: FogSlots = parse_json(weather, "fog", &json)?;
    let range_of = |slot: TimeSlot| -> Result<Option<FogRange>, ExteriorWeatherError> {
        let raw = ranges
            .get(slot.json_key())
            .ok_or_else(|| missing_slot(weather, "fog", slot))?;
        fog_range(weather, slot, raw)
    };
    let colors = TimeSlot::try_map_all(|slot| {
        let near = color_in_slot(weather, colors, "FogNear", slot)?;
        let far = color_in_slot(weather, colors, "FogFar", slot)?;
        Ok::<_, ExteriorWeatherError>(FogColors {
            near,
            far,
            near_high: high_color_in_slot(weather, colors, "FogNearHigh", slot, near)?,
            far_high: high_color_in_slot(weather, colors, "FogFarHigh", slot, far)?,
        })
    })?;
    Ok(WeatherFog {
        day: range_of(FOG_DAY_SLOT)?,
        night: range_of(FOG_NIGHT_SLOT)?,
        colors,
    })
}

/// A height-fog colour group is absent from databases that predate its export; the fog then
/// keeps its `low` colour at every height.
fn high_color_in_slot(
    weather: &str,
    colors: &ColorGroups,
    group: &str,
    slot: TimeSlot,
    low: Vec3,
) -> Result<Vec3, ExteriorWeatherError> {
    if colors.contains_key(group) {
        color_in_slot(weather, colors, group, slot)
    } else {
        Ok(low)
    }
}

/// `None` for the zero range that marks a slot without fog.
fn fog_range(
    weather: &str,
    slot: TimeSlot,
    raw: &RawFog,
) -> Result<Option<FogRange>, ExteriorWeatherError> {
    if raw.near == 0.0 && raw.far == 0.0 {
        return Ok(None);
    }
    if !(raw.far > raw.near) {
        return Err(ExteriorWeatherError::InvertedFogRange {
            weather: weather.to_string(),
            slot: slot.json_key(),
            near: raw.near,
            far: raw.far,
        });
    }
    let height = RawHeightFog {
        near_mid: raw.near_height_mid,
        near_range: raw.near_height_range,
        far_mid: raw.far_height_mid,
        far_range: raw.far_height_range,
        high_density_scale: raw.high_density_scale,
    }
    .bands()
    .map_err(|source| ExteriorWeatherError::HeightFog {
        weather: weather.to_string(),
        slot: slot.json_key(),
        source,
    })?;
    Ok(Some(FogRange {
        near: raw.near * SCALE,
        far: raw.far * SCALE,
        power: raw.power,
        max: raw.max,
        height,
    }))
}

fn load_image_spaces(
    db: &DbClient,
    row: &WeatherRow,
) -> Result<[ImageSpaceData; TimeSlot::COUNT], ExteriorWeatherError> {
    let names: HashMap<String, Option<String>> =
        parse_field(&row.form_key, "image spaces", row.image_spaces.as_deref())?;
    TimeSlot::try_map_all(|slot| {
        let image_space = names
            .get(slot.json_key())
            .cloned()
            .flatten()
            .ok_or_else(|| missing_slot(&row.form_key, "image space", slot))?;
        load_image_space(db, &image_space)
    })
}

fn load_image_space(
    db: &DbClient,
    image_space: &str,
) -> Result<ImageSpaceData, ExteriorWeatherError> {
    let data = db
        .get_image_space_data(image_space)
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => ExteriorWeatherError::ImageSpaceNotFound {
                image_space: image_space.to_string(),
            },
            other => other.into(),
        })?
        .ok_or_else(|| ExteriorWeatherError::ImageSpaceWithoutData {
            image_space: image_space.to_string(),
        })?;
    serde_json::from_str(&data).map_err(|source| ExteriorWeatherError::ImageSpaceMalformed {
        image_space: image_space.to_string(),
        source,
    })
}

/// The climate of `worldspace`, with a warning when it predates the moon export.
pub(crate) fn load_worldspace_climate(
    db: &DbClient,
    worldspace: &str,
) -> Result<ClimateInfo, ExteriorWeatherError> {
    let climate_name = db
        .get_worldspace_climate(worldspace)
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => ExteriorWeatherError::WorldspaceNotFound {
                worldspace: worldspace.to_string(),
            },
            other => other.into(),
        })?
        .ok_or_else(|| ExteriorWeatherError::WorldspaceWithoutClimate {
            worldspace: worldspace.to_string(),
        })?;
    let climate = load_climate(db, &climate_name)?;
    if climate.sky.moons.is_none() {
        tracing::warn!(
            "climate {climate_name} stores no moon flags (database predates the moon export); the sky shows no moon"
        );
    }
    Ok(climate)
}

fn load_climate(db: &DbClient, climate: &str) -> Result<ClimateInfo, ExteriorWeatherError> {
    let data = db
        .get_climate_data(climate)
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => ExteriorWeatherError::ClimateNotFound {
                climate: climate.to_string(),
            },
            other => other.into(),
        })?
        .ok_or_else(|| ExteriorWeatherError::ClimateWithoutData {
            climate: climate.to_string(),
        })?;
    let mut info = parse_climate(climate, &data)?;
    let (sun, glare) = db.get_climate_sun_textures(climate)?;
    info.sun_textures = ClimateSunTextures::new(sun.as_deref(), glare.as_deref());
    Ok(info)
}

fn parse_climate(climate: &str, json: &str) -> Result<ClimateInfo, ExteriorWeatherError> {
    let data: ClimateData =
        serde_json::from_str(json).map_err(|source| ExteriorWeatherError::ClimateMalformed {
            climate: climate.to_string(),
            source,
        })?;
    let sky = ClimateSky::parse(json).map_err(|source| ExteriorWeatherError::ClimateSky {
        climate: climate.to_string(),
        source,
    })?;
    let (entries, initial) = pickable_entries(climate, data.weathers)?;
    Ok(ClimateInfo {
        name: climate.to_string(),
        entries,
        initial,
        times: ClimateTimes::parse(json).map_err(|source| ExteriorWeatherError::ClimateTimes {
            climate: climate.to_string(),
            source,
        })?,
        moon: ClimateMoon::from_climate(&sky).map_err(|source| {
            ExteriorWeatherError::ClimateMoon {
                climate: climate.to_string(),
                source,
            }
        })?,
        sky,
        sun_textures: ClimateSunTextures::default(),
    })
}

/// The entries a climate can pick, without the ones gated by a global (not evaluated yet, so
/// they are ignored with a warning), and the index of the most likely of them: the highest
/// chance, the first listed among equals.
fn pickable_entries(
    climate: &str,
    weathers: Vec<ClimateWeather>,
) -> Result<(Vec<ClimateWeather>, usize), ExteriorWeatherError> {
    if weathers.is_empty() {
        return Err(ExteriorWeatherError::EmptyWeatherList {
            climate: climate.to_string(),
        });
    }
    let (entries, gated): (Vec<_>, Vec<_>) = weathers
        .into_iter()
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
            "climate {climate} has weathers gated by a global, which is not supported and ignored: {}",
            gated.join(", ")
        );
    }
    let initial = entries
        .iter()
        .enumerate()
        .rev()
        .max_by_key(|(_, entry)| entry.chance)
        .map(|(index, _)| index)
        .ok_or_else(|| ExteriorWeatherError::NoUngatedWeather {
            climate: climate.to_string(),
        })?;
    Ok((entries, initial))
}

/// A JSON column of a weather that must be there.
pub(crate) fn parse_field<T: serde::de::DeserializeOwned>(
    weather: &str,
    field: &'static str,
    json: Option<&str>,
) -> Result<T, ExteriorWeatherError> {
    let json = json.ok_or_else(|| ExteriorWeatherError::MissingField {
        weather: weather.to_string(),
        field,
    })?;
    parse_json(weather, field, json)
}

/// A JSON column of a weather.
pub(crate) fn parse_json<T: serde::de::DeserializeOwned>(
    weather: &str,
    field: &'static str,
    json: &str,
) -> Result<T, ExteriorWeatherError> {
    serde_json::from_str(json).map_err(|source| ExteriorWeatherError::Malformed {
        weather: weather.to_string(),
        field,
        source,
    })
}

fn missing_slot(weather: &str, entry: &str, slot: TimeSlot) -> ExteriorWeatherError {
    ExteriorWeatherError::MissingTimeSlot {
        weather: weather.to_string(),
        entry: entry.to_string(),
        slot: slot.json_key(),
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use serde_json::{Value, json};

    use super::*;

    pub(crate) const FORM_KEY: &str = "0012AB:Fallout4.esm";
    pub(super) const DEFAULT_CLIMATE_JSON: &str = r#"{"Weathers":[{"Weather":"0012AB:Fallout4.esm","Chance":51}],"SunriseBegin":"05:00:00","SunriseEnd":"09:00:00","SunsetBegin":"17:00:00","SunsetEnd":"21:00:00","MoonPhaseLength":4,"Moons":{"Masser":true,"Secunda":false},"ModelPath":"Sky\\Stars.nif"}"#;

    pub(in crate::rendering::lighting) fn rgb(r: u8, g: u8, b: u8) -> Value {
        json!({"R": r, "G": g, "B": b})
    }

    /// One JSON object with an entry per slot, each from `value`.
    pub(in crate::rendering::lighting) fn per_slot(value: impl Fn(TimeSlot) -> Value) -> Value {
        Value::Object(
            TimeSlot::ALL
                .into_iter()
                .map(|slot| (slot.json_key().to_owned(), value(slot)))
                .collect(),
        )
    }

    pub(super) fn same_in_every_slot(value: &Value) -> Value {
        per_slot(|_| value.clone())
    }

    pub(super) fn colors() -> Value {
        let group = |value: Value| same_in_every_slot(&value);
        json!({
            "Sunlight": group(rgb(255, 0, 0)),
            "Sun": group(rgb(0, 255, 0)),
            "SunGlare": group(rgb(255, 128, 0)),
            "SkyUpper": group(rgb(0, 0, 255)),
            "SkyLower": group(rgb(255, 255, 0)),
            "Horizon": group(rgb(0, 255, 255)),
            "Stars": group(rgb(255, 0, 255)),
            "FogNear": group(rgb(0, 0, 0)),
            "FogFar": group(rgb(255, 255, 255)),
        })
    }

    pub(super) fn ambient_slot(scale: f32) -> Value {
        let face = |r: u8| rgb(r, r, r);
        json!({
            "XPlus": face(0), "XMinus": face(0),
            "YPlus": face(255), "YMinus": face(0),
            "ZPlus": face(0), "ZMinus": face(0),
            "Specular": rgb(255, 255, 255),
            "Scale": scale,
        })
    }

    pub(super) fn directional_ambient(scale: f32) -> Value {
        same_in_every_slot(&ambient_slot(scale))
    }

    pub(super) fn fog_slot(near: f32, far: f32) -> Value {
        json!({"Near": near, "Far": far, "Power": 1.5, "Max": 0.75})
    }

    pub(super) fn fog(near: f32, far: f32) -> Value {
        json!({"Day": fog_slot(near, far), "Night": fog_slot(near, far)})
    }

    pub(super) fn row() -> WeatherRow {
        WeatherRow {
            form_key: FORM_KEY.to_owned(),
            editor_id: Some("CommonwealthClear".to_owned()),
            colors: Some(colors().to_string()),
            directional_ambient: Some(directional_ambient(1.0).to_string()),
            image_spaces: Some(same_in_every_slot(&Value::Null).to_string()),
            fog: Some(fog(100.0, 1000.0).to_string()),
        }
    }

    pub(super) fn neutral_image_space() -> ImageSpaceData {
        ImageSpaceData {
            hdr: ImageSpaceHdr::default(),
            depth_of_field: None,
        }
    }

    pub(crate) fn climate() -> ClimateInfo {
        parse_climate("DefaultClimate", DEFAULT_CLIMATE_JSON).unwrap()
    }

    /// The default climate with the weather list `entries_json` (a JSON array).
    pub(crate) fn climate_with_entries(entries_json: &str) -> ClimateInfo {
        let json = format!(
            r#"{{"Weathers":{entries_json},"SunriseBegin":"05:00:00","SunriseEnd":"09:00:00","SunsetBegin":"17:00:00","SunsetEnd":"21:00:00"}}"#
        );
        parse_climate("DefaultClimate", &json).unwrap()
    }

    /// The default weather's slots under another form key.
    pub(crate) fn slots_for(form_key: &str, editor_id: &str) -> ExteriorWeatherSlots {
        let mut row = row();
        form_key.clone_into(&mut row.form_key);
        row.editor_id = Some(editor_id.to_owned());
        load(row).unwrap()
    }

    /// The default weather's slots under another form key, with the upper sky colour that
    /// `sky_upper` gives for each time slot.
    pub(crate) fn slots_with_sky(
        form_key: &str,
        editor_id: &str,
        sky_upper: impl Fn(TimeSlot) -> (u8, u8, u8),
    ) -> ExteriorWeatherSlots {
        let mut colors = colors();
        colors["SkyUpper"] = per_slot(|slot| {
            let (r, g, b) = sky_upper(slot);
            rgb(r, g, b)
        });
        let mut row = row();
        form_key.clone_into(&mut row.form_key);
        row.editor_id = Some(editor_id.to_owned());
        row.colors = Some(colors.to_string());
        load(row).unwrap()
    }

    pub(super) fn slots_with(
        row: WeatherRow,
        image_spaces: &[ImageSpaceData; TimeSlot::COUNT],
    ) -> Result<ExteriorWeatherSlots, ExteriorWeatherError> {
        ExteriorWeatherSlots::from_row(row, image_spaces, &climate())
    }

    pub(super) fn load(row: WeatherRow) -> Result<ExteriorWeatherSlots, ExteriorWeatherError> {
        slots_with(row, &[neutral_image_space(); TimeSlot::COUNT])
    }

    /// The same weather in a climate that shows no moon.
    pub(in crate::rendering::lighting) fn without_moon(
        slots: ExteriorWeatherSlots,
    ) -> ExteriorWeatherSlots {
        slots.without_moon()
    }

    /// A weather loaded from the default row after `edit_colors` changed its colour groups and
    /// with the image space values `hdr` gives per slot.
    pub(in crate::rendering::lighting) fn weather_slots(
        edit_colors: impl FnOnce(&mut Value),
        hdr: impl Fn(TimeSlot) -> ImageSpaceHdr,
    ) -> ExteriorWeatherSlots {
        let mut colors = colors();
        edit_colors(&mut colors);
        let mut row = row();
        row.colors = Some(colors.to_string());
        let image_spaces = TimeSlot::try_map_all(|slot| {
            Ok::<_, ExteriorWeatherError>(ImageSpaceData {
                hdr: hdr(slot),
                depth_of_field: None,
            })
        })
        .unwrap();
        ExteriorWeatherSlots::from_row(row, &image_spaces, &climate()).unwrap()
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact sentinels and copied values"
)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use TimeSlot::{Day, EarlySunrise, LateSunrise, Night, Sunrise};

    type Corrupt = fn(&mut WeatherRow, String);

    use super::super::fog::test_support::assert_band;
    use super::super::fog::{Fog, FogParams, HeightBand, HeightFog};
    use super::super::time_of_day::SlotBlend;
    use super::fixtures::*;

    fn at(slot: TimeSlot) -> SlotBlend {
        SlotBlend {
            from: slot,
            to: slot,
            t: 0.0,
        }
    }

    fn between(from: TimeSlot, to: TimeSlot, t: f32) -> SlotBlend {
        SlotBlend { from, to, t }
    }

    fn day_weather(row: WeatherRow) -> ExteriorWeather {
        load(row).unwrap().sample(at(Day))
    }

    #[test]
    fn a_complete_row_yields_linear_colours_for_every_group() {
        let weather = day_weather(row());
        assert_eq!(weather.sun_light, Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(weather.sun_disc, Vec3::new(0.0, 1.0, 0.0));
        assert_eq!(weather.sun_glare.x, 1.0);
        assert_eq!(weather.sun_glare.z, 0.0);
        assert_eq!(weather.sky_upper, Vec3::new(0.0, 0.0, 1.0));
        assert_eq!(weather.sky_lower, Vec3::new(1.0, 1.0, 0.0));
        assert_eq!(weather.horizon, Vec3::new(0.0, 1.0, 1.0));
        assert_eq!(weather.stars, Vec3::new(1.0, 0.0, 1.0));
        assert_eq!(&*weather.form_key, FORM_KEY);
        assert_eq!(weather.editor_id.as_deref(), Some("CommonwealthClear"));
    }

    #[test]
    fn weather_fog_is_scaled_into_engine_units() {
        let weather = day_weather(row());
        let Some(Fog::Enabled(fog)) = weather.fog else {
            panic!("fog should be enabled: {:?}", weather.fog);
        };
        assert!((fog.near - 10.0).abs() < 1e-4);
        assert!((fog.far - 100.0).abs() < 1e-3);
        assert_eq!((fog.power, fog.max), (1.5, 0.75));
        assert_eq!(fog.near_color, Vec3::ZERO);
        assert_eq!(fog.far_color, Vec3::ONE);
    }

    #[test]
    fn the_ambient_cube_follows_engine_axis_order_and_the_weather_scale() {
        let mut row = row();
        row.directional_ambient = Some(directional_ambient(0.5).to_string());
        let cube = day_weather(row).ambient_cube.0;
        // Bethesda +Y (forward) is the engine -Z face, the last one.
        let nonzero: Vec<usize> = (0..6).filter(|i| cube[*i] != Vec3::ZERO).collect();
        assert_eq!(nonzero, vec![5]);
        let unscaled = day_weather(self::row()).ambient_cube.0[5];
        assert!(cube[5].x < unscaled.x);
    }

    #[test]
    fn a_row_without_fog_data_still_loads_with_default_fog_at_every_moment() {
        let mut row = row();
        row.fog = None;
        let slots = load(row).unwrap();
        for blend in [at(Day), at(Night), between(Night, Day, 0.5)] {
            assert_eq!(slots.sample(blend).fog, None);
        }
    }

    #[test]
    fn a_zero_fog_range_in_both_slots_means_the_weather_has_no_fog() {
        let mut row = row();
        row.fog = Some(fog(0.0, 0.0).to_string());
        let slots = load(row).unwrap();
        for blend in [at(Day), at(Night), between(Night, Day, 0.5)] {
            assert_eq!(slots.sample(blend).fog, Some(Fog::Disabled));
        }
    }

    #[test]
    fn a_fog_range_that_does_not_increase_fails_the_weather_naming_the_slot() {
        for (near, far) in [(1000.0, 100.0), (500.0, 500.0)] {
            for broken in ["Day", "Night"] {
                let mut fog = fog(100.0, 1000.0);
                fog[broken] = fog_slot(near, far);
                let mut row = row();
                row.fog = Some(fog.to_string());
                let result = load(row);
                assert!(
                    matches!(&result, Err(ExteriorWeatherError::InvertedFogRange { slot, .. }) if *slot == broken),
                    "{near}..{far} in {broken}: {result:?}"
                );
            }
        }
    }

    #[test]
    fn malformed_json_fails_the_whole_weather_naming_the_broken_field() {
        let cases: [(&str, Corrupt); 3] = [
            ("colors", |row, text| row.colors = Some(text)),
            ("directional ambient", |row, text| {
                row.directional_ambient = Some(text);
            }),
            ("fog", |row, text| row.fog = Some(text)),
        ];
        for (field, corrupt) in cases {
            let mut row = row();
            corrupt(&mut row, "{ not json".to_owned());
            let result = load(row);
            assert!(
                matches!(&result, Err(ExteriorWeatherError::Malformed { weather, field: f, .. }) if weather == FORM_KEY && *f == field),
                "{field}: {result:?}"
            );
        }
    }

    #[test]
    fn a_missing_colour_or_ambient_column_is_reported_with_the_weather() {
        let mut no_colors = row();
        no_colors.colors = None;
        assert!(matches!(
            load(no_colors),
            Err(ExteriorWeatherError::MissingField { weather, field: "colors" })
                if weather == FORM_KEY
        ));
        let mut no_ambient = row();
        no_ambient.directional_ambient = None;
        assert!(matches!(
            load(no_ambient),
            Err(ExteriorWeatherError::MissingField { weather, field: "directional ambient" })
                if weather == FORM_KEY
        ));
    }

    #[test]
    fn a_colour_group_missing_one_slot_names_the_group_and_the_slot() {
        let mut broken = colors();
        broken["Sunlight"]
            .as_object_mut()
            .unwrap()
            .remove("LateSunset");
        let mut row = row();
        row.colors = Some(broken.to_string());
        let error = load(row).unwrap_err();
        assert!(
            matches!(&error, ExteriorWeatherError::MissingTimeSlot { entry, slot, .. } if entry == "Sunlight" && *slot == "LateSunset"),
            "{error:?}"
        );
    }

    #[test]
    fn a_missing_colour_group_is_a_missing_slot_error() {
        let mut broken = colors();
        broken.as_object_mut().unwrap().remove("Horizon");
        let mut row = row();
        row.colors = Some(broken.to_string());
        assert!(matches!(
            load(row),
            Err(ExteriorWeatherError::MissingTimeSlot { entry, .. }) if entry == "Horizon"
        ));
    }

    #[test]
    fn a_missing_stars_group_is_a_missing_slot_error() {
        let mut broken = colors();
        broken.as_object_mut().unwrap().remove("Stars");
        let mut row = row();
        row.colors = Some(broken.to_string());
        assert!(matches!(
            load(row),
            Err(ExteriorWeatherError::MissingTimeSlot { entry, .. }) if entry == "Stars"
        ));
    }

    #[test]
    fn a_missing_ambient_slot_names_the_slot() {
        let mut broken = directional_ambient(1.0);
        broken.as_object_mut().unwrap().remove("Night");
        let mut row = row();
        row.directional_ambient = Some(broken.to_string());
        let error = load(row).unwrap_err();
        assert!(
            matches!(&error, ExteriorWeatherError::MissingTimeSlot { entry, slot, .. } if entry == "directional ambient" && *slot == "Night"),
            "{error:?}"
        );
    }

    #[test]
    fn a_fog_slot_missing_from_the_distances_names_the_slot() {
        let mut row = row();
        row.fog = Some(json!({"Day": fog_slot(100.0, 1000.0)}).to_string());
        let error = load(row).unwrap_err();
        assert!(
            matches!(&error, ExteriorWeatherError::MissingTimeSlot { entry, slot, .. } if entry == "fog" && *slot == "Night"),
            "{error:?}"
        );
    }

    #[test]
    fn fog_colours_are_required_in_every_slot_when_fog_data_exists() {
        let mut broken = colors();
        broken["FogFar"].as_object_mut().unwrap().remove("Sunrise");
        let mut row = row();
        row.colors = Some(broken.to_string());
        let error = load(row).unwrap_err();
        assert!(
            matches!(&error, ExteriorWeatherError::MissingTimeSlot { entry, slot, .. } if entry == "FogFar" && *slot == "Sunrise"),
            "{error:?}"
        );
    }

    /// Sunlight's red channel counts up with the slot, so each slot is told apart.
    fn sunlight_by_slot() -> WeatherRow {
        let mut colors = colors();
        colors["Sunlight"] = per_slot(|slot| rgb(slot.index() as u8 * 30, 0, 0));
        let mut row = row();
        row.colors = Some(colors.to_string());
        row
    }

    fn sunlight_red(slot: TimeSlot) -> f32 {
        RawColor::linear(serde_json::from_value(rgb(slot.index() as u8 * 30, 0, 0)).unwrap()).x
    }

    #[test]
    fn a_blend_that_starts_a_slot_takes_that_slots_values() {
        let slots = load(sunlight_by_slot()).unwrap();
        for slot in TimeSlot::ALL {
            let weather = slots.sample(between(slot, Day, 0.0));
            assert_eq!(weather.sun_light.x, sunlight_red(slot), "{slot:?}");
        }
    }

    #[test]
    fn a_blend_that_finishes_a_slot_takes_that_slots_values() {
        let slots = load(sunlight_by_slot()).unwrap();
        for slot in TimeSlot::ALL {
            let weather = slots.sample(between(Day, slot, 1.0));
            assert_eq!(weather.sun_light.x, sunlight_red(slot), "{slot:?}");
        }
    }

    #[test]
    fn a_blend_mixes_exactly_its_two_slots_in_linear_space() {
        let slots = load(sunlight_by_slot()).unwrap();
        let weather = slots.sample(between(LateSunrise, Sunrise, 0.25));
        let expected = sunlight_red(LateSunrise) * 0.75 + sunlight_red(Sunrise) * 0.25;
        assert!((weather.sun_light.x - expected).abs() < 1e-6);
    }

    #[test]
    fn halfway_between_black_and_white_is_linear_one_half_not_the_encoded_mid_grey() {
        let mut colors = colors();
        colors["Sun"] = per_slot(|slot| {
            if slot == Night {
                rgb(0, 0, 0)
            } else {
                rgb(255, 255, 255)
            }
        });
        let mut row = row();
        row.colors = Some(colors.to_string());
        let weather = load(row).unwrap().sample(between(Night, Day, 0.5));
        assert!((weather.sun_disc.x - 0.5).abs() < 1e-6);
    }

    #[test]
    fn every_colour_group_is_blended() {
        let mut colors = colors();
        for group in [
            "Sunlight", "Sun", "SunGlare", "SkyUpper", "SkyLower", "Horizon", "Stars",
        ] {
            colors[group] = per_slot(|slot| {
                if slot == Night {
                    rgb(0, 0, 0)
                } else {
                    rgb(255, 255, 255)
                }
            });
        }
        let mut row = row();
        row.colors = Some(colors.to_string());
        let weather = load(row).unwrap().sample(between(Night, Day, 0.5));
        for (name, colour) in [
            ("sun light", weather.sun_light),
            ("sun disc", weather.sun_disc),
            ("sun glare", weather.sun_glare),
            ("sky upper", weather.sky_upper),
            ("sky lower", weather.sky_lower),
            ("horizon", weather.horizon),
            ("stars", weather.stars),
        ] {
            assert!(
                colour.abs_diff_eq(Vec3::splat(0.5), 1e-6),
                "{name}: {colour}"
            );
        }
    }

    #[test]
    fn the_ambient_cube_and_specular_are_blended_per_slot() {
        let mut ambient = directional_ambient(1.0);
        ambient["Night"] = ambient_slot(0.0);
        let mut row = row();
        row.directional_ambient = Some(ambient.to_string());
        let slots = load(row).unwrap();
        let day = slots.sample(at(Day));
        let night = slots.sample(at(Night));
        let halfway = slots.sample(between(Night, Day, 0.5));
        assert!(night.ambient_cube.0.iter().all(|face| *face == Vec3::ZERO));
        for face in 0..6 {
            assert!(
                halfway.ambient_cube.0[face].abs_diff_eq(day.ambient_cube.0[face] * 0.5, 1e-6),
                "face {face}"
            );
        }
    }

    #[test]
    fn the_image_space_light_scales_and_exposure_limits_are_blended() {
        let mut image_spaces = [neutral_image_space(); TimeSlot::COUNT];
        image_spaces[Night.index()].hdr = ImageSpaceHdr {
            eye_adapt_speed: 1.0,
            sunlight_scale: 0.0,
            sky_scale: 0.5,
            middle_gray: 0.1,
            auto_exposure_min: 0.2,
            auto_exposure_max: 2.0,
        };
        let slots = slots_with(row(), &image_spaces).unwrap();
        let hdr = slots.sample(between(Night, Day, 0.5)).hdr;
        let day = ImageSpaceHdr::default();
        assert!((hdr.sunlight_scale - 0.5).abs() < 1e-6);
        assert!((hdr.sky_scale - 0.75).abs() < 1e-6);
        assert!((hdr.eye_adapt_speed - f32::midpoint(1.0, day.eye_adapt_speed)).abs() < 1e-6);
        assert!((hdr.middle_gray - f32::midpoint(0.1, day.middle_gray)).abs() < 1e-6);
        assert!((hdr.auto_exposure_min - f32::midpoint(0.2, day.auto_exposure_min)).abs() < 1e-6);
        assert!((hdr.auto_exposure_max - f32::midpoint(2.0, day.auto_exposure_max)).abs() < 1e-6);
    }

    fn depth_of_field(strength: f32, mode: u32) -> ImageSpaceDepthOfField {
        ImageSpaceDepthOfField {
            strength,
            distance: 1000.0 * strength,
            range: 100.0,
            mode,
            vignette_radius: 10.0,
            vignette_strength: strength,
        }
    }

    #[test]
    fn depth_of_field_blends_its_numbers_and_snaps_its_mode() {
        let mut image_spaces = [neutral_image_space(); TimeSlot::COUNT];
        image_spaces[Night.index()].depth_of_field = Some(depth_of_field(0.0, 1));
        image_spaces[Day.index()].depth_of_field = Some(depth_of_field(1.0, 2));
        let slots = slots_with(row(), &image_spaces).unwrap();
        let early = slots
            .sample(between(Night, Day, 0.25))
            .depth_of_field
            .unwrap();
        let late = slots
            .sample(between(Night, Day, 0.75))
            .depth_of_field
            .unwrap();
        assert!((early.strength - 0.25).abs() < 1e-6);
        assert!((early.distance - 250.0).abs() < 1e-3);
        assert!((late.vignette_strength - 0.75).abs() < 1e-6);
        assert_eq!((early.mode, late.mode), (1, 2));
    }

    #[test]
    fn depth_of_field_present_at_one_end_only_snaps_at_the_midpoint() {
        let mut image_spaces = [neutral_image_space(); TimeSlot::COUNT];
        image_spaces[Day.index()].depth_of_field = Some(depth_of_field(1.0, 2));
        let slots = slots_with(row(), &image_spaces).unwrap();
        assert_eq!(slots.sample(between(Night, Day, 0.4)).depth_of_field, None);
        assert_eq!(
            slots.sample(between(Night, Day, 0.6)).depth_of_field,
            Some(depth_of_field(1.0, 2))
        );
    }

    #[test]
    fn the_initial_weather_is_the_day_slot() {
        let slots = load(sunlight_by_slot()).unwrap();
        assert_eq!(slots.initial(), slots.sample(at(Day)));
    }

    #[test]
    fn a_sample_keeps_the_weather_identity() {
        let slots = load(row()).unwrap();
        assert_eq!(slots.form_key(), FORM_KEY);
        assert_eq!(slots.editor_id(), Some("CommonwealthClear"));
        let weather = slots.sample(between(Night, Day, 0.3));
        assert_eq!(&*weather.form_key, FORM_KEY);
        assert_eq!(weather.editor_id.as_deref(), Some("CommonwealthClear"));
    }

    fn fog_with(day: &Value, night: &Value) -> WeatherRow {
        let mut row = row();
        row.fog = Some(json!({"Day": day, "Night": night}).to_string());
        row
    }

    fn sampled_fog(slots: &ExteriorWeatherSlots, blend: SlotBlend) -> FogParams {
        match slots.sample(blend).fog {
            Some(Fog::Enabled(params)) => params,
            other => panic!("fog should be enabled: {other:?}"),
        }
    }

    #[test]
    fn fog_distances_blend_by_how_much_daylight_the_moment_has() {
        let slots = load(fog_with(&fog_slot(100.0, 1000.0), &fog_slot(300.0, 5000.0))).unwrap();
        let day = sampled_fog(&slots, at(Day));
        let night = sampled_fog(&slots, at(Night));
        assert!((day.far - 100.0).abs() < 1e-3);
        assert!((night.far - 500.0).abs() < 1e-3);
        let halfway = sampled_fog(&slots, between(Night, Day, 0.5));
        assert!((halfway.near - f32::midpoint(day.near, night.near)).abs() < 1e-4);
        assert!((halfway.far - 300.0).abs() < 1e-3);
    }

    #[test]
    fn a_slot_between_night_and_day_blends_the_fog_by_its_own_daylight_weight() {
        let slots = load(fog_with(&fog_slot(100.0, 1000.0), &fog_slot(300.0, 5000.0))).unwrap();
        let night = sampled_fog(&slots, at(Night));
        let day = sampled_fog(&slots, at(Day));
        // EarlySunrise sits a quarter of the way from Night to Day.
        let early = sampled_fog(&slots, at(EarlySunrise));
        assert!((early.far - (night.far + (day.far - night.far) * 0.25)).abs() < 1e-3);
        let start_of_blend = sampled_fog(&slots, between(Night, EarlySunrise, 0.0));
        assert_eq!(start_of_blend.far, night.far);
    }

    #[test]
    fn fog_colours_follow_the_slot_blend_not_the_daylight_weight() {
        let mut colors = colors();
        colors["FogNear"] = per_slot(|slot| {
            if slot == Sunrise {
                rgb(255, 255, 255)
            } else {
                rgb(0, 0, 0)
            }
        });
        let mut row = row();
        row.colors = Some(colors.to_string());
        let slots = load(row).unwrap();
        let at_sunrise = sampled_fog(&slots, at(Sunrise));
        assert_eq!(at_sunrise.near_color, Vec3::ONE);
        let past = sampled_fog(&slots, between(Sunrise, LateSunrise, 0.5));
        assert!(past.near_color.abs_diff_eq(Vec3::splat(0.5), 1e-6));
    }

    #[test]
    fn fog_that_is_off_by_day_and_on_by_night_snaps_at_half_the_daylight_weight() {
        let slots = load(fog_with(&fog_slot(0.0, 0.0), &fog_slot(300.0, 5000.0))).unwrap();
        assert_eq!(slots.sample(at(Day)).fog, Some(Fog::Disabled));
        assert!(matches!(slots.sample(at(Night)).fog, Some(Fog::Enabled(_))));
        // Daylight weight 0.4 is nearer to Night, 0.6 to Day.
        let near_night = slots.sample(between(Night, Day, 0.4)).fog;
        let near_day = slots.sample(between(Night, Day, 0.6)).fog;
        let Some(Fog::Enabled(params)) = near_night else {
            panic!("night fog should still apply: {near_night:?}");
        };
        assert!(
            (params.far - 500.0).abs() < 1e-3,
            "distances are not blended"
        );
        assert_eq!(near_day, Some(Fog::Disabled));
    }

    #[test]
    fn fog_that_is_on_by_day_and_off_by_night_snaps_the_other_way() {
        let slots = load(fog_with(&fog_slot(100.0, 1000.0), &fog_slot(0.0, 0.0))).unwrap();
        assert_eq!(
            slots.sample(between(Night, Day, 0.4)).fog,
            Some(Fog::Disabled)
        );
        assert!(matches!(
            slots.sample(between(Night, Day, 0.6)).fog,
            Some(Fog::Enabled(_))
        ));
    }

    /// A fog slot with height fog: near band `near` and far band `far` as (mid, range).
    fn height_fog_slot(near: (f32, f32), far: (f32, f32), high_density_scale: f32) -> Value {
        let mut slot = fog_slot(100.0, 1000.0);
        let fields = json!({
            "NearHeightMid": near.0, "NearHeightRange": near.1,
            "FarHeightMid": far.0, "FarHeightRange": far.1,
            "HighDensityScale": high_density_scale,
        });
        slot.as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        slot
    }

    fn with_high_colors(mut row: WeatherRow) -> WeatherRow {
        let mut colors = colors();
        colors["FogNearHigh"] = same_in_every_slot(&rgb(255, 0, 0));
        colors["FogFarHigh"] = same_in_every_slot(&rgb(0, 0, 255));
        row.colors = Some(colors.to_string());
        row
    }

    #[test]
    fn a_weather_without_height_fog_data_has_the_same_fog_at_every_height() {
        let fog = sampled_fog(&load(row()).unwrap(), at(Day));
        assert_eq!(
            fog.height,
            HeightFog {
                near_high_color: fog.near_color,
                far_high_color: fog.far_color,
                ..HeightFog::NONE
            }
        );
        let flat = FogParams {
            height: HeightFog::NONE,
            ..fog
        };
        assert_eq!(
            Fog::Enabled(fog).uniform_fields(),
            Fog::Enabled(flat).uniform_fields()
        );
    }

    #[test]
    fn height_fog_is_scaled_into_engine_units_with_its_high_colours() {
        let slot = height_fog_slot((500.0, 2000.0), (1800.0, 15_000.0), 0.95);
        let fog = sampled_fog(
            &load(with_high_colors(fog_with(&slot, &slot))).unwrap(),
            at(Day),
        );
        assert_band(fog.height.near_band, 50.0, 200.0);
        assert_band(fog.height.far_band, 180.0, 1500.0);
        assert_eq!(fog.height.high_density_scale, 0.95);
        assert_eq!(fog.height.near_high_color, Vec3::X);
        assert_eq!(fog.height.far_high_color, Vec3::Z);
    }

    #[test]
    fn height_bands_and_density_blend_by_daylight_like_the_distances() {
        let day = height_fog_slot((500.0, 2000.0), (1000.0, 10_000.0), 1.0);
        let night = height_fog_slot((1500.0, 4000.0), (3000.0, 30_000.0), 0.5);
        let slots = load(fog_with(&day, &night)).unwrap();
        let halfway = sampled_fog(&slots, between(Night, Day, 0.5)).height;
        assert_band(halfway.near_band, 100.0, 300.0);
        assert_band(halfway.far_band, 200.0, 2000.0);
        assert_eq!(halfway.high_density_scale, 0.75);
    }

    #[test]
    fn zero_height_ranges_leave_the_fog_the_same_at_every_height() {
        // Records from before height fog store zeros, a zero density scale included.
        let slot = height_fog_slot((0.0, 0.0), (0.0, 0.0), 0.0);
        let slots = load(with_high_colors(fog_with(&slot, &slot))).unwrap();
        let fog = sampled_fog(&slots, at(Day));
        assert_eq!(fog.height.near_band, HeightBand::UNSET);
        assert_eq!(fog.height.far_band, HeightBand::UNSET);
        let flat = FogParams {
            height: HeightFog::NONE,
            ..fog
        };
        assert_eq!(
            Fog::Enabled(fog).uniform_fields(),
            Fog::Enabled(flat).uniform_fields()
        );
    }

    #[test]
    fn a_height_mid_without_its_range_fails_the_weather_naming_the_band() {
        let mut slot = height_fog_slot((500.0, 2000.0), (1800.0, 15_000.0), 1.0);
        slot.as_object_mut().unwrap().remove("FarHeightRange");
        let result = load(fog_with(&fog_slot(100.0, 1000.0), &slot));
        assert!(
            matches!(&result, Err(ExteriorWeatherError::HeightFog { slot, source: HeightFogError::IncompleteBand { band }, .. }) if *slot == "Night" && *band == "far"),
            "{result:?}"
        );
    }

    #[test]
    fn a_negative_high_density_scale_fails_the_weather() {
        let slot = height_fog_slot((500.0, 2000.0), (1800.0, 15_000.0), -0.5);
        let result = load(fog_with(&slot, &fog_slot(100.0, 1000.0)));
        assert!(
            matches!(&result, Err(ExteriorWeatherError::HeightFog { slot, source: HeightFogError::NegativeHighDensityScale { .. }, .. }) if *slot == "Day"),
            "{result:?}"
        );
    }

    #[test]
    fn a_high_fog_colour_group_must_have_every_slot() {
        let mut colors = colors();
        colors["FogFarHigh"] = same_in_every_slot(&rgb(0, 0, 255));
        colors["FogFarHigh"]
            .as_object_mut()
            .unwrap()
            .remove("Sunset");
        let mut row = row();
        row.colors = Some(colors.to_string());
        let error = load(row).unwrap_err();
        assert!(
            matches!(&error, ExteriorWeatherError::MissingTimeSlot { entry, slot, .. } if entry == "FogFarHigh" && *slot == "Sunset"),
            "{error:?}"
        );
    }

    #[test]
    fn the_climate_gives_the_most_likely_weather_and_its_sky() {
        let climate = climate();
        assert_eq!(climate.initial_weather(), FORM_KEY);
        assert_eq!(climate.times.sunrise_begin(), 5.0);
        assert_eq!(climate.times.sunset_end(), 21.0);
        assert_eq!(climate.sky.moon_phase_length, Some(4));
        assert_eq!(climate.sky.model_path.as_deref(), Some("Sky\\Stars.nif"));
        assert_eq!(climate.moon.unwrap().phase_length(), 4);
    }

    #[test]
    fn among_equal_chances_the_first_listed_weather_wins() {
        let json = r#"{"Weathers":[{"Weather":"A","Chance":10},{"Weather":"B","Chance":10},{"Weather":"C","Chance":5}],"SunriseBegin":"05:00:00","SunriseEnd":"09:00:00","SunsetBegin":"17:00:00","SunsetEnd":"21:00:00"}"#;
        let climate = parse_climate("C", json).unwrap();
        assert_eq!(climate.initial_weather(), "A");
        assert_eq!(climate.initial_index(), 0);
    }

    fn climate_json(weathers: &str) -> String {
        format!(
            r#"{{"Weathers":{weathers},"SunriseBegin":"05:00:00","SunriseEnd":"09:00:00","SunsetBegin":"17:00:00","SunsetEnd":"21:00:00"}}"#
        )
    }

    #[test]
    fn weathers_gated_by_a_global_are_not_picked_and_never_start_the_climate() {
        let json = climate_json(
            r#"[{"Weather":"Gated","Chance":90,"Global":"00FF:Fallout4.esm"},{"Weather":"A","Chance":10,"Global":null},{"Weather":"B","Chance":20}]"#,
        );
        let climate = parse_climate("C", &json).unwrap();
        let listed: Vec<&str> = climate
            .entries()
            .iter()
            .map(|entry| entry.weather.as_str())
            .collect();
        assert_eq!(listed, ["A", "B"]);
        assert_eq!(climate.initial_weather(), "B");
        assert_eq!(climate.initial_index(), 1);
    }

    #[test]
    fn a_climate_with_only_gated_weathers_is_an_error_naming_it() {
        let json = climate_json(r#"[{"Weather":"A","Chance":50,"Global":"00FF:Fallout4.esm"}]"#);
        let error = parse_climate("Clim", &json).unwrap_err();
        assert!(
            matches!(&error, ExteriorWeatherError::NoUngatedWeather { climate } if climate == "Clim"),
            "{error:?}"
        );
    }

    #[test]
    fn a_climate_without_weathers_or_with_unusable_data_is_an_error_naming_it() {
        let times = r#""SunriseBegin":"05:00:00","SunriseEnd":"09:00:00","SunsetBegin":"17:00:00","SunsetEnd":"21:00:00""#;
        let weathers = r#""Weathers":[{"Weather":"A","Chance":1}]"#;
        let empty = format!(r#"{{"Weathers":[],{times}}}"#);
        assert!(matches!(
            parse_climate("Clim", &empty),
            Err(ExteriorWeatherError::EmptyWeatherList { climate }) if climate == "Clim"
        ));
        let no_times = format!("{{{weathers}}}");
        assert!(matches!(
            parse_climate("Clim", &no_times),
            Err(ExteriorWeatherError::ClimateTimes { climate, .. }) if climate == "Clim"
        ));
        let bad_sky = format!(r#"{{{weathers},{times},"MoonPhaseLength":"four"}}"#);
        assert!(matches!(
            parse_climate("Clim", &bad_sky),
            Err(ExteriorWeatherError::ClimateSky { climate, .. }) if climate == "Clim"
        ));
        let no_phase =
            format!(r#"{{{weathers},{times},"Moons":{{"Masser":true,"Secunda":false}}}}"#);
        assert!(matches!(
            parse_climate("Clim", &no_phase),
            Err(ExteriorWeatherError::ClimateMoon { climate, .. }) if climate == "Clim"
        ));
        assert!(matches!(
            parse_climate("Clim", "{ not json"),
            Err(ExteriorWeatherError::ClimateMalformed { climate, .. }) if climate == "Clim"
        ));
    }

    #[test]
    fn a_climate_from_an_older_database_has_no_moon() {
        let json = r#"{"Weathers":[{"Weather":"A","Chance":1}],"SunriseBegin":"05:00:00","SunriseEnd":"09:00:00","SunsetBegin":"17:00:00","SunsetEnd":"21:00:00"}"#;
        let climate = parse_climate("Clim", json).unwrap();
        assert_eq!(climate.moon, None);
        assert_eq!(climate.sky.moons, None);
    }

    #[test]
    fn the_loaded_slots_expose_the_climate_they_belong_to() {
        let slots = load(row()).unwrap();
        assert_eq!(slots.climate_times().sunrise_end(), 9.0);
        assert_eq!(slots.climate_sky().moon_phase_length, Some(4));
        assert_eq!(slots.moon().unwrap().phase_length(), 4);
    }

    const IMAGE_SPACE_HDR: &str = r#""Hdr":{"EyeAdaptSpeed":3.0,"SunlightScale":1.0,"SkyScale":1.0,"MiddleGray":0.18,"AutoExposureMin":1.6,"AutoExposureMax":3.25}"#;

    #[test]
    fn an_image_space_row_carries_its_depth_of_field_and_vignette() {
        let json = format!(
            r#"{{{IMAGE_SPACE_HDR},"DepthOfField":{{"Strength":0.65,"Distance":2000.0,"Range":5048.0,"Mode":2.0,"VignetteRadius":40.0,"VignetteStrength":0.5}}}}"#
        );
        let data: ImageSpaceData = serde_json::from_str(&json).unwrap();
        let dof = data.depth_of_field.expect("the block is exported");
        assert_eq!(
            (dof.strength, dof.distance, dof.range, dof.mode),
            (0.65, 2000.0, 5048.0, 2)
        );
        assert_eq!(dof.vignette_radius, 40.0);
        assert_eq!(dof.vignette_strength, 0.5);
    }

    #[test]
    fn an_image_space_row_from_an_older_database_has_no_depth_of_field() {
        let json = format!("{{{IMAGE_SPACE_HDR}}}");
        let data: ImageSpaceData = serde_json::from_str(&json).unwrap();
        assert_eq!(data.depth_of_field, None);
    }
}
