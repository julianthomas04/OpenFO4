use std::collections::HashMap;

use glam::Vec3;
use serde::Deserialize;

use super::ambient::{AMBIENT_CUBE_FACES, AmbientCube};
use super::cell_lighting::{RawColor, StoredColourEncoding, bethesda_faces_to_engine_order};
use super::fog::measured::{BlendedFogColors, slot_colour_factors, slot_day_weight};
use super::fog::{FogColors, FogRange, HeightFogError, RawHeightFog};
use super::time_of_day::{
    ClimateMoon, ClimateMoonError, ClimateSky, ClimateSkyError, ClimateSunTextures, ClimateTimes,
    ClimateTimesError, TimeSlot,
};
use super::tonemap_settings::{ImageSpaceCinematic, ImageSpaceLut};
use super::weather_slots::{
    AMBIENT_ENCODING, GLARE_AND_STARS_ENCODING, SKY_ENCODING, SUN_DISC_ENCODING, SUNLIGHT_ENCODING,
    SlotFogColors, SlotValues, TINT_ENCODING, WATER_MULTIPLIER_ENCODING, WeatherFog,
};
pub use super::weather_slots::{
    ExteriorWeather, ExteriorWeatherSlots, ImageSpaceHdr, WeatherAmbient,
};
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
    #[error("image space {image_space} lacks {}; the database predates their export, re-run the installer", missing.join(", "))]
    ImageSpaceFieldsMissing {
        image_space: String,
        missing: Vec<&'static str>,
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

#[derive(Debug, Clone, Deserialize)]
#[serde(from = "RawImageSpaceData")]
struct ImageSpaceData {
    depth_of_field: Option<ImageSpaceDepthOfField>,
    hdr: ImageSpaceHdr,
    cinematic: ImageSpaceCinematic,
    lut: ImageSpaceLut,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawImageSpaceData {
    depth_of_field: Option<ImageSpaceDepthOfField>,
    hdr: ImageSpaceHdr,
    cinematic: RawCinematic,
    tint: RawTint,
    /// Below the textures folder; empty for an image space without one.
    lut: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawCinematic {
    saturation: f32,
    brightness: f32,
    contrast: f32,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawTint {
    amount: f32,
    color: RawColor,
}

impl From<RawImageSpaceData> for ImageSpaceData {
    fn from(raw: RawImageSpaceData) -> Self {
        Self {
            depth_of_field: raw.depth_of_field,
            hdr: raw.hdr,
            cinematic: ImageSpaceCinematic {
                saturation: raw.cinematic.saturation,
                brightness: raw.cinematic.brightness,
                contrast: raw.cinematic.contrast,
                tint_amount: raw.tint.amount,
                tint_color: raw.tint.color.decoded(TINT_ENCODING),
            },
            lut: ImageSpaceLut::named(&raw.lut),
        }
    }
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
                    initial.ambient,
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
                image_spaces[slot.index()].clone(),
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
    let group = |name: &str, encoding: StoredColourEncoding| {
        raw_color_in_slot(weather, colors, name, slot).map(|color| color.decoded(encoding))
    };
    let dalc = ambient
        .get(slot.json_key())
        .ok_or_else(|| missing_slot(weather, "directional ambient", slot))?;
    let ambient = WeatherAmbient {
        colors: AmbientCube(
            bethesda_faces_to_engine_order(faces_lit_by_weather_axes(dalc))
                .map(|face| face.decoded(AMBIENT_ENCODING)),
        ),
        scale: dalc.scale,
    };
    Ok(SlotValues {
        sun_light: group("Sunlight", SUNLIGHT_ENCODING)?,
        sun_disc: group("Sun", SUN_DISC_ENCODING)?,
        sun_glare: group("SunGlare", GLARE_AND_STARS_ENCODING)?,
        sky_upper: group("SkyUpper", SKY_ENCODING)?,
        sky_lower: group("SkyLower", SKY_ENCODING)?,
        horizon: group("Horizon", SKY_ENCODING)?,
        stars: group("Stars", GLARE_AND_STARS_ENCODING)?,
        ambient,
        ambient_specular: dalc.specular.linear(),
        water_multiplier: group("WaterMultiplier", WATER_MULTIPLIER_ENCODING)?,
        hdr: image_space.hdr,
        cinematic: image_space.cinematic,
        lut: image_space.lut,
        depth_of_field: image_space.depth_of_field,
    })
}

/// A weather's directional ambient colour lights the surfaces that face away from its axis, so
/// each goes to the opposite face, in Bethesda order +X, -X, +Y, -Y, +Z, -Z.
fn faces_lit_by_weather_axes(dalc: &RawAmbientColors) -> [RawColor; AMBIENT_CUBE_FACES] {
    [
        dalc.x_minus,
        dalc.x_plus,
        dalc.y_minus,
        dalc.y_plus,
        dalc.z_minus,
        dalc.z_plus,
    ]
}

fn raw_color_in_slot(
    weather: &str,
    colors: &ColorGroups,
    group: &str,
    slot: TimeSlot,
) -> Result<RawColor, ExteriorWeatherError> {
    colors
        .get(group)
        .and_then(|slots| slots.get(slot.json_key()))
        .copied()
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
        let fog_color =
            |group: &str| raw_color_in_slot(weather, colors, group, slot).map(RawColor::unit);
        let near = fog_color("FogNear")?;
        let far = fog_color("FogFar")?;
        let high_color = |group: &str, low: Vec3| {
            if colors.contains_key(group) {
                fog_color(group)
            } else {
                Ok(low)
            }
        };
        Ok::<_, ExteriorWeatherError>(SlotFogColors {
            fog: BlendedFogColors::from_stored(
                FogColors {
                    near,
                    far,
                    near_high: high_color("FogNearHigh", near)?,
                    far_high: high_color("FogFarHigh", far)?,
                },
                slot_colour_factors(slot),
            ),
            sunlight: raw_color_in_slot(weather, colors, "Sunlight", slot).map(RawColor::unit)?,
            day_weight: slot_day_weight(slot),
        })
    })?;
    Ok(WeatherFog {
        day: range_of(FOG_DAY_SLOT)?,
        night: range_of(FOG_NIGHT_SLOT)?,
        colors,
    })
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
    parse_image_space(image_space, &data)
}

/// The image space fields the tone curve, the cinematic stage and the colour grading read, which
/// older installers did not export.
const EXPORTED_LATER_FIELDS: [&[&str]; 4] =
    [&["Hdr", "TonemapE"], &["Cinematic"], &["Tint"], &["Lut"]];

fn parse_image_space(
    image_space: &str,
    data: &str,
) -> Result<ImageSpaceData, ExteriorWeatherError> {
    let malformed = |source| ExteriorWeatherError::ImageSpaceMalformed {
        image_space: image_space.to_string(),
        source,
    };
    let value: serde_json::Value = serde_json::from_str(data).map_err(malformed)?;
    let missing: Vec<&'static str> = EXPORTED_LATER_FIELDS
        .iter()
        .filter(|path| {
            path.iter()
                .try_fold(&value, |node, key| node.get(key))
                .is_none()
        })
        .map(|path| *path.last().expect("every path names a field"))
        .collect();
    if !missing.is_empty() {
        return Err(ExteriorWeatherError::ImageSpaceFieldsMissing {
            image_space: image_space.to_string(),
            missing,
        });
    }
    serde_json::from_value(value).map_err(malformed)
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
            "WaterMultiplier": group(rgb(51, 102, 255)),
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

    pub(super) fn neutral_image_spaces() -> [ImageSpaceData; TimeSlot::COUNT] {
        std::array::from_fn(|_| neutral_image_space())
    }

    pub(super) fn neutral_image_space() -> ImageSpaceData {
        ImageSpaceData {
            hdr: ImageSpaceHdr::default(),
            cinematic: ImageSpaceCinematic::NEUTRAL,
            lut: ImageSpaceLut::NONE,
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
        slots_with(row, &neutral_image_spaces())
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
                cinematic: ImageSpaceCinematic::NEUTRAL,
                lut: ImageSpaceLut::NONE,
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
    use TimeSlot::{Day, LateSunrise, Night, Sunrise};

    type Corrupt = fn(&mut WeatherRow, String);

    use super::super::fog::measured::{DAY_COLOUR_FACTORS, NON_DAY_COLOUR_FACTORS};
    use super::super::fog::test_support::assert_band;
    use super::super::fog::{ExteriorFog, Fog, FogTime, HeightBand, HeightBands};
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
        assert_eq!(weather.water_multiplier, Vec3::new(0.2, 0.4, 1.0));
        assert_eq!(&*weather.form_key, FORM_KEY);
        assert_eq!(weather.editor_id.as_deref(), Some("CommonwealthClear"));
    }

    #[test]
    fn weather_fog_is_scaled_into_engine_units() {
        let fog = sampled_fog(&load(row()).unwrap(), at(Day));
        let day = fog.day.expect("the day half has fog");
        assert!((day.near - 10.0).abs() < 1e-4);
        assert!((day.far - 100.0).abs() < 1e-3);
        assert_eq!((day.power, day.max), (1.5, 0.75));
        assert_eq!(fog.colors.linear().near, Vec3::ZERO);
        assert_eq!(fog.colors.linear().far, Vec3::ONE);
    }

    #[test]
    fn fog_colours_decode_with_gamma_two_point_two_and_the_sunlight_stays_as_stored() {
        let mut row = row();
        let mut colors: Value = serde_json::from_str(row.colors.as_deref().unwrap()).unwrap();
        colors["FogNear"] = same_in_every_slot(&rgb(128, 128, 128));
        colors["Sunlight"] = same_in_every_slot(&rgb(225, 225, 225));
        row.colors = Some(colors.to_string());
        let fog = sampled_fog(&load(row).unwrap(), at(Day));
        let near = (128.0_f32 / 255.0).powf(2.2) * DAY_COLOUR_FACTORS.near;
        assert!((fog.colors.linear().near.x - near).abs() < 1e-6);
        assert!((fog.sunlight.x - 225.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn each_ambient_colour_lights_the_face_opposite_its_axis_with_the_weather_scale() {
        let face = |r: u8| rgb(r, 0, 0);
        let slot = serde_json::json!({
            "XPlus": face(10), "XMinus": face(20),
            "YPlus": face(30), "YMinus": face(40),
            "ZPlus": face(50), "ZMinus": face(60),
            "Specular": rgb(0, 0, 0),
            "Scale": 0.5,
        });
        let mut row = row();
        row.directional_ambient = Some(same_in_every_slot(&slot).to_string());
        let cube = day_weather(row).ambient.cube().0;
        // Engine faces +X (east), -X, +Y (up), -Y, +Z (south), -Z take the Bethesda colours of
        // the opposite axes: X-, X+, Z-, Z+, Y+ (north), Y-.
        let expected_bytes: [u8; 6] = [20, 10, 60, 50, 30, 40];
        for (index, (face, byte)) in cube.iter().zip(expected_bytes).enumerate() {
            let expected = super::super::cell_lighting::srgb8_to_linear([byte, 0, 0]) * 0.5;
            assert!(face.abs_diff_eq(expected, 1e-7), "face {index}: {face}");
        }
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
            let fog = sampled_fog(&slots, blend);
            assert_eq!((fog.day, fog.night), (None, None));
            assert_eq!(fog.resolve(FogTime::DAY), Fog::Disabled);
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
    fn a_missing_water_multiplier_group_is_a_missing_slot_error() {
        let mut broken = colors();
        broken.as_object_mut().unwrap().remove("WaterMultiplier");
        let mut row = row();
        row.colors = Some(broken.to_string());
        assert!(matches!(
            load(row),
            Err(ExteriorWeatherError::MissingTimeSlot { entry, .. }) if entry == "WaterMultiplier"
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
    fn a_blend_mixes_exactly_its_two_slots_as_stored_bytes() {
        let slots = load(sunlight_by_slot()).unwrap();
        let weather = slots.sample(between(LateSunrise, Sunrise, 0.25));
        let stored_byte = f32::from(LateSunrise.index() as u8 * 30) * 0.75
            + f32::from(Sunrise.index() as u8 * 30) * 0.25;
        let expected = SUNLIGHT_ENCODING.decode(Vec3::new(stored_byte / 255.0, 0.0, 0.0));
        assert!((weather.sun_light.x - expected.x).abs() < 1e-6);
    }

    #[test]
    fn halfway_between_black_and_white_is_the_stored_mid_grey_decoded() {
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
        assert!((weather.sun_disc.x - 0.5_f32.powf(2.2)).abs() < 1e-6);
    }

    #[test]
    fn every_colour_group_is_blended() {
        let mut colors = colors();
        for group in [
            "Sunlight",
            "Sun",
            "SunGlare",
            "SkyUpper",
            "SkyLower",
            "Horizon",
            "Stars",
            "WaterMultiplier",
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
        let stored_half = 0.5_f32.powf(2.2);
        let srgb_half = ((0.5 + 0.055) / 1.055_f32).powf(2.4);
        for (name, colour, expected) in [
            ("sun light", weather.sun_light, srgb_half),
            ("sun disc", weather.sun_disc, stored_half),
            ("sun glare", weather.sun_glare, 0.5),
            ("sky upper", weather.sky_upper, stored_half),
            ("sky lower", weather.sky_lower, stored_half),
            ("horizon", weather.horizon, stored_half),
            ("stars", weather.stars, 0.5),
            ("water multiplier", weather.water_multiplier, 0.5),
        ] {
            assert!(
                colour.abs_diff_eq(Vec3::splat(expected), 1e-6),
                "{name}: {colour}"
            );
        }
    }

    #[test]
    fn the_sky_colours_blend_as_stored_and_decode_with_gamma_two_point_two() {
        let mut colors = colors();
        for group in ["SkyUpper", "SkyLower", "Horizon"] {
            colors[group] = per_slot(|slot| match slot {
                Sunrise => rgb(48, 48, 48),
                _ => rgb(144, 144, 144),
            });
        }
        let mut row = row();
        row.colors = Some(colors.to_string());
        let slots = load(row).unwrap();
        let pure = slots.sample(between(Sunrise, LateSunrise, 0.0));
        assert!((pure.sky_upper.x - (48.0_f32 / 255.0).powf(2.2)).abs() < 1e-6);
        let blended = slots.sample(between(Sunrise, LateSunrise, 0.25));
        let stored = (0.75 * 48.0 + 0.25 * 144.0) / 255.0;
        for colour in [blended.sky_upper, blended.sky_lower, blended.horizon] {
            assert!(
                colour.abs_diff_eq(Vec3::splat(f32::powf(stored, 2.2)), 1e-6),
                "{colour}"
            );
        }
    }

    #[test]
    fn the_sun_disc_decodes_with_gamma_two_point_two() {
        let mut colors = colors();
        colors["Sun"] = per_slot(|_| rgb(16, 48, 255));
        let mut row = row();
        row.colors = Some(colors.to_string());
        let weather = load(row).unwrap().sample(between(Day, Day, 0.0));
        let expected = Vec3::new(16.0, 48.0, 255.0).map(|byte| (byte / 255.0).powf(2.2));
        assert!(
            weather.sun_disc.abs_diff_eq(expected, 1e-6),
            "{}",
            weather.sun_disc
        );
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
        assert!(
            night
                .ambient
                .cube()
                .0
                .iter()
                .all(|face| *face == Vec3::ZERO)
        );
        for face in 0..6 {
            assert!(
                halfway.ambient.cube().0[face].abs_diff_eq(day.ambient.cube().0[face] * 0.5, 1e-6),
                "face {face}"
            );
        }
    }

    #[test]
    fn the_ambient_colours_blend_as_stored_bytes_and_their_scale_linearly() {
        let mut ambient = directional_ambient(1.0);
        let mut night = ambient_slot(0.5);
        night["YPlus"] = rgb(0, 0, 0);
        ambient["Night"] = night;
        let mut row = row();
        row.directional_ambient = Some(ambient.to_string());
        let halfway = load(row).unwrap().sample(between(Night, Day, 0.5));
        let srgb_half = ((0.5 + 0.055) / 1.055_f32).powf(2.4);
        let lit = halfway
            .ambient
            .colors
            .0
            .iter()
            .find(|face| face.x > 0.0)
            .unwrap();
        assert!(lit.abs_diff_eq(Vec3::splat(srgb_half), 1e-6), "{lit}");
        assert!((halfway.ambient.scale - 0.75).abs() < 1e-6);
    }

    #[test]
    fn the_image_space_light_scales_and_exposure_limits_are_blended() {
        let mut image_spaces = neutral_image_spaces();
        image_spaces[Night.index()].hdr = ImageSpaceHdr {
            eye_adapt_speed: 1.0,
            sunlight_scale: 0.0,
            sky_scale: 0.5,
            middle_gray: 0.1,
            auto_exposure_min: 0.2,
            auto_exposure_max: 2.0,
            tonemap_e: 0.04,
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
        assert!((hdr.tonemap_e - f32::midpoint(0.04, day.tonemap_e)).abs() < 1e-6);
    }

    #[test]
    fn the_image_space_cinematic_fields_and_tint_are_blended() {
        let mut image_spaces = neutral_image_spaces();
        image_spaces[Night.index()].cinematic = ImageSpaceCinematic {
            saturation: 0.0,
            brightness: 2.0,
            contrast: 0.5,
            tint_amount: 1.0,
            tint_color: Vec3::new(1.0, 0.0, 0.0),
        };
        let slots = slots_with(row(), &image_spaces).unwrap();
        let cinematic = slots.sample(between(Night, Day, 0.5)).cinematic;
        let stored_half = 0.5_f32.powf(2.2);
        assert_eq!(
            ImageSpaceCinematic {
                tint_color: Vec3::ZERO,
                ..cinematic
            },
            ImageSpaceCinematic {
                saturation: 0.5,
                brightness: 1.5,
                contrast: 0.75,
                tint_amount: 0.5,
                tint_color: Vec3::ZERO,
            }
        );
        assert!(
            cinematic
                .tint_color
                .abs_diff_eq(Vec3::new(1.0, stored_half, stored_half), 1e-6),
            "the tint blends as stored: {}",
            cinematic.tint_color
        );
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
        let mut image_spaces = neutral_image_spaces();
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
        let mut image_spaces = neutral_image_spaces();
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

    fn sampled_fog(slots: &ExteriorWeatherSlots, blend: SlotBlend) -> ExteriorFog {
        slots
            .sample(blend)
            .fog
            .expect("the weather carries fog data")
    }

    #[test]
    fn both_fog_halves_reach_the_sample_whatever_the_slot() {
        let slots = load(fog_with(&fog_slot(100.0, 1000.0), &fog_slot(300.0, 5000.0))).unwrap();
        for blend in [at(Day), at(Night), between(Sunrise, LateSunrise, 0.3)] {
            let fog = sampled_fog(&slots, blend);
            assert!((fog.day.unwrap().near - 10.0).abs() < 1e-4);
            assert!((fog.night.unwrap().near - 30.0).abs() < 1e-4);
        }
    }

    #[test]
    fn fog_colours_blend_as_stored_across_slots_with_their_slot_factors() {
        let mut row = fog_with(&fog_slot(100.0, 1000.0), &fog_slot(300.0, 5000.0));
        let mut colors: Value = serde_json::from_str(row.colors.as_deref().unwrap()).unwrap();
        colors["FogNear"] = per_slot(|slot| {
            if slot == Sunrise {
                rgb(255, 255, 255)
            } else {
                rgb(0, 0, 0)
            }
        });
        row.colors = Some(colors.to_string());
        let slots = load(row).unwrap();
        let night_factor = NON_DAY_COLOUR_FACTORS.near;
        let sunrise = sampled_fog(&slots, at(Sunrise)).colors.linear().near;
        assert!(
            sunrise.abs_diff_eq(Vec3::splat(night_factor), 1e-6),
            "{sunrise}"
        );
        let past = sampled_fog(&slots, between(Sunrise, LateSunrise, 0.5));
        // Halfway between the stored black and white, decoded after the blend.
        let halfway = 0.5_f32.powf(2.2) * night_factor;
        assert!((past.colors.linear().near.x - halfway).abs() < 1e-6);
    }

    #[test]
    fn a_half_without_fog_stays_without_it() {
        let slots = load(fog_with(&fog_slot(0.0, 0.0), &fog_slot(300.0, 5000.0))).unwrap();
        let fog = sampled_fog(&slots, at(Day));
        assert_eq!(fog.day, None);
        assert!(fog.night.is_some());
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

    fn day_half(fog: &ExteriorFog) -> FogRange {
        fog.day.expect("the day half has fog")
    }

    #[test]
    fn a_weather_without_height_fog_data_has_the_same_fog_at_every_height() {
        let fog = sampled_fog(&load(row()).unwrap(), at(Day));
        assert_eq!(day_half(&fog).height, HeightBands::NONE);
        // The fixture's near colour is black and its far colour white: the high colours repeat
        // them, so at Day the far high colour is white times the far high factor 0.58.
        let colors = fog.colors.linear();
        assert_eq!(colors.near, Vec3::ZERO);
        assert_eq!(colors.near_high, Vec3::ZERO);
        assert!(colors.far.abs_diff_eq(Vec3::ONE, 1e-6), "{}", colors.far);
        assert!(
            colors.far_high.abs_diff_eq(Vec3::splat(0.58), 1e-6),
            "{}",
            colors.far_high
        );
    }

    #[test]
    fn height_fog_is_scaled_into_engine_units_with_its_high_colours() {
        let slot = height_fog_slot((500.0, 2000.0), (1800.0, 15_000.0), 0.95);
        let fog = sampled_fog(
            &load(with_high_colors(fog_with(&slot, &slot))).unwrap(),
            at(Day),
        );
        let height = day_half(&fog).height;
        assert_band(height.near, 50.0, 200.0);
        assert_band(height.far, 180.0, 1500.0);
        assert_eq!(height.high_density_scale, 0.95);
        let colors = fog.colors.linear();
        assert!(
            colors
                .near_high
                .abs_diff_eq(Vec3::X * DAY_COLOUR_FACTORS.near_high, 1e-6)
        );
        assert!(
            colors
                .far_high
                .abs_diff_eq(Vec3::Z * DAY_COLOUR_FACTORS.far_high, 1e-6)
        );
    }

    #[test]
    fn height_bands_and_density_blend_by_the_night_weight_like_the_distances() {
        let day = height_fog_slot((500.0, 2000.0), (1000.0, 10_000.0), 1.0);
        let night = height_fog_slot((1500.0, 4000.0), (3000.0, 30_000.0), 0.5);
        let slots = load(fog_with(&day, &night)).unwrap();
        let Fog::Enabled(halfway) =
            sampled_fog(&slots, at(Day)).resolve(FogTime { night_weight: 0.5 })
        else {
            panic!("both halves have fog");
        };
        assert_band(halfway.height.near, 100.0, 300.0);
        assert_band(halfway.height.far, 200.0, 2000.0);
        assert_eq!(halfway.height.high_density_scale, 0.75);
    }

    #[test]
    fn zero_height_ranges_leave_the_fog_the_same_at_every_height() {
        // Records from before height fog store zeros, a zero density scale included.
        let slot = height_fog_slot((0.0, 0.0), (0.0, 0.0), 0.0);
        let slots = load(with_high_colors(fog_with(&slot, &slot))).unwrap();
        let height = day_half(&sampled_fog(&slots, at(Day))).height;
        assert_eq!(height.near, HeightBand::UNSET);
        assert_eq!(height.far, HeightBand::UNSET);
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

    const IMAGE_SPACE_HDR: &str = r#""Hdr":{"EyeAdaptSpeed":3.0,"TonemapE":0.02,"SunlightScale":1.0,"SkyScale":1.0,"MiddleGray":0.18,"AutoExposureMin":1.6,"AutoExposureMax":3.25},"Cinematic":{"Saturation":1.1,"Brightness":0.9,"Contrast":1.2},"Tint":{"Amount":0.2,"Color":{"R":255,"G":128,"B":0}},"Lut":"""#;

    #[test]
    fn an_image_space_row_carries_its_cinematic_fields_and_its_tint_in_linear_light() {
        let json = format!("{{{IMAGE_SPACE_HDR}}}");
        let data: ImageSpaceData = serde_json::from_str(&json).unwrap();
        assert_eq!(data.hdr.tonemap_e, 0.02);
        let cinematic = data.cinematic;
        assert_eq!(
            (
                cinematic.saturation,
                cinematic.brightness,
                cinematic.contrast
            ),
            (1.1, 0.9, 1.2)
        );
        assert_eq!(cinematic.tint_amount, 0.2);
        let half_byte = (128.0_f32 / 255.0).powf(2.2);
        assert!(
            cinematic
                .tint_color
                .abs_diff_eq(Vec3::new(1.0, half_byte, 0.0), 1e-6)
        );
    }

    #[test]
    fn an_image_space_row_from_before_the_tone_and_cinematic_export_asks_for_a_reinstall() {
        let json = r#"{"Hdr":{"EyeAdaptSpeed":3.0,"SunlightScale":1.0,"SkyScale":1.0,"MiddleGray":0.18,"AutoExposureMin":1.6,"AutoExposureMax":3.25}}"#;
        let error = parse_image_space("OldIS", json).unwrap_err();
        assert!(matches!(
            &error,
            ExteriorWeatherError::ImageSpaceFieldsMissing { image_space, missing }
                if image_space == "OldIS" && *missing == ["TonemapE", "Cinematic", "Tint", "Lut"]
        ));
        assert!(
            error.to_string().contains("re-run the installer"),
            "{error}"
        );
    }

    #[test]
    fn an_image_space_row_from_before_the_lut_export_asks_for_a_reinstall() {
        let json = format!("{{{IMAGE_SPACE_HDR}}}").replace(r#","Lut":"""#, "");
        assert!(matches!(
            parse_image_space("OldIS", &json),
            Err(ExteriorWeatherError::ImageSpaceFieldsMissing { missing, .. }) if missing == ["Lut"]
        ));
    }

    #[test]
    fn an_image_space_row_names_its_lut_and_an_empty_one_names_none() {
        let json = format!("{{{IMAGE_SPACE_HDR}}}");
        assert_eq!(
            parse_image_space("IS", &json).unwrap().lut,
            ImageSpaceLut::NONE
        );
        let graded = json.replace(
            r#""Lut":"""#,
            r#""Lut":"Effects\\LUTs\\LUT_CW_ClearDAY.DDS""#,
        );
        assert_eq!(
            parse_image_space("IS", &graded).unwrap().lut,
            ImageSpaceLut::named(r"Effects\LUTs\LUT_CW_ClearDAY.DDS")
        );
    }

    #[test]
    fn an_image_space_row_with_a_mistyped_field_is_malformed() {
        let json = format!("{{{IMAGE_SPACE_HDR}}}")
            .replace(r#""Saturation":1.1"#, r#""Saturation":"high""#);
        assert!(matches!(
            parse_image_space("BadIS", &json),
            Err(ExteriorWeatherError::ImageSpaceMalformed { .. })
        ));
    }

    #[test]
    fn a_complete_image_space_row_parses() {
        let json = format!("{{{IMAGE_SPACE_HDR}}}");
        assert_eq!(
            parse_image_space("IS", &json).unwrap().cinematic.contrast,
            1.2
        );
    }

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
