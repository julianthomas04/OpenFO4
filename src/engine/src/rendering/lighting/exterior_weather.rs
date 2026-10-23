use std::collections::HashMap;

use glam::Vec3;
use serde::Deserialize;

use super::ambient::AmbientCube;
use super::cell_lighting::{RawColor, bethesda_faces_to_engine_order};
use super::fog::{Fog, FogParams};
use crate::assets::db::DbClient;
use crate::assets::db::weather::WeatherRow;
use crate::rendering::post::depth_of_field::ImageSpaceDepthOfField;
use crate::rendering::world_streamer::SCALE;

const TIME_SLOT: &str = "Day";

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
        "weather {weather} fog range is not increasing (near {near}, far {far} Bethesda units)"
    )]
    InvertedFogRange {
        weather: String,
        near: f32,
        far: f32,
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
    #[error("malformed {what} JSON: {source}")]
    Malformed {
        what: &'static str,
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

#[derive(Debug, Deserialize)]
struct ClimateWeather {
    #[serde(rename = "Weather")]
    weather: String,
    #[serde(rename = "Chance")]
    chance: u32,
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

/// Optional height-fog fields in the JSON are ignored.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawFog {
    near: f32,
    far: f32,
    power: f32,
    max: f32,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct ImageSpaceData {
    #[serde(rename = "DepthOfField")]
    depth_of_field: Option<ImageSpaceDepthOfField>,
    #[serde(rename = "Hdr")]
    hdr: ImageSpaceHdr,
}

/// The HDR block of an image space; field meanings follow the Creation Kit
/// image space dialog.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ImageSpaceHdr {
    pub eye_adapt_speed: f32,
    pub sunlight_scale: f32,
    pub sky_scale: f32,
    pub middle_gray: f32,
    pub auto_exposure_min: f32,
    pub auto_exposure_max: f32,
}

/// `CommonwealthClear`'s values, with neutral light scales, for when no weather loads.
impl Default for ImageSpaceHdr {
    fn default() -> Self {
        Self {
            eye_adapt_speed: 3.0,
            sunlight_scale: 1.0,
            sky_scale: 1.0,
            middle_gray: 0.18,
            auto_exposure_min: 1.6,
            auto_exposure_max: 3.25,
        }
    }
}

type ColorGroups = HashMap<String, HashMap<String, RawColor>>;
type FogSlots = HashMap<String, RawFog>;
type DirectionalAmbient = HashMap<String, RawAmbientColors>;

/// The values of one weather at a fixed time slot that exterior lighting uses,
/// all linear.
#[derive(Debug, Clone, PartialEq)]
pub struct ExteriorWeather {
    pub form_key: String,
    pub editor_id: Option<String>,
    pub sun_light: Vec3,
    pub sun_disc: Vec3,
    pub sky_upper: Vec3,
    pub sky_lower: Vec3,
    pub horizon: Vec3,
    pub ambient_cube: AmbientCube,
    /// Extracted but not rendered yet.
    pub ambient_specular: Vec3,
    pub hdr: ImageSpaceHdr,
    /// The image space's depth of field and vignette: what the game's image-space manager
    /// holds for the depth of field effect. `None` when the database predates the export.
    pub depth_of_field: Option<ImageSpaceDepthOfField>,
    /// `None` when the database carries no fog data for the weather;
    /// `Fog::Disabled` when the weather explicitly has no fog.
    pub fog: Option<Fog>,
}

impl ExteriorWeather {
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
            Ok(weather) => {
                tracing::info!(
                    "exterior weather {} ({}) for {worldspace}, slot {TIME_SLOT}: sun light {:?}, sun disc {:?}, hdr {:?}, depth of field {:?}, sky upper {:?}, sky lower {:?}, horizon {:?}, ambient cube {:?}, ambient specular {:?}, fog source {}",
                    weather.editor_id.as_deref().unwrap_or("<no editor id>"),
                    weather.form_key,
                    weather.sun_light,
                    weather.sun_disc,
                    weather.hdr,
                    weather.depth_of_field,
                    weather.sky_upper,
                    weather.sky_lower,
                    weather.horizon,
                    weather.ambient_cube.0,
                    weather.ambient_specular,
                    weather
                        .fog
                        .map_or_else(|| "default".to_string(), |fog| format!("weather {fog:?}")),
                );
                Some(weather)
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
        let climate = db
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
        let weather_key = most_likely_weather(db, &climate)?;
        let row = db.get_weather(&weather_key).map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => ExteriorWeatherError::WeatherNotFound {
                weather: weather_key.clone(),
            },
            other => other.into(),
        })?;
        let image_space = load_image_space(db, &row)?;
        Self::from_row(row, image_space)
    }

    fn from_row(
        row: WeatherRow,
        image_space: ImageSpaceData,
    ) -> Result<Self, ExteriorWeatherError> {
        let WeatherRow {
            form_key,
            editor_id,
            colors,
            directional_ambient,
            image_spaces: _,
            fog,
        } = row;
        let colors: ColorGroups = parse_field(&form_key, "colors", colors)?;
        let ambient: DirectionalAmbient =
            parse_field(&form_key, "directional ambient", directional_ambient)?;
        let group = |name: &str| -> Result<RawColor, ExteriorWeatherError> {
            colors
                .get(name)
                .and_then(|times| times.get(TIME_SLOT))
                .copied()
                .ok_or_else(|| missing_slot(&form_key, name))
        };
        let dalc = ambient
            .get(TIME_SLOT)
            .ok_or_else(|| missing_slot(&form_key, "directional ambient"))?;

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

        let fog = match weather_fog(&form_key, fog, group("FogNear"), group("FogFar")) {
            Ok(fog) => Some(fog),
            Err(error @ ExteriorWeatherError::FogMissing { .. }) => {
                tracing::warn!("{error}; exterior fog keeps its defaults");
                None
            }
            Err(error) => return Err(error),
        };

        Ok(Self {
            fog,
            sun_light: group("Sunlight")?.linear(),
            sun_disc: group("Sun")?.linear(),
            sky_upper: group("SkyUpper")?.linear(),
            sky_lower: group("SkyLower")?.linear(),
            horizon: group("Horizon")?.linear(),
            ambient_cube,
            ambient_specular: dalc.specular.linear(),
            hdr: image_space.hdr,
            depth_of_field: image_space.depth_of_field,
            form_key,
            editor_id,
        })
    }
}

fn weather_fog(
    weather: &str,
    json: Option<String>,
    near_color: Result<RawColor, ExteriorWeatherError>,
    far_color: Result<RawColor, ExteriorWeatherError>,
) -> Result<Fog, ExteriorWeatherError> {
    let json = json.ok_or_else(|| ExteriorWeatherError::FogMissing {
        weather: weather.to_string(),
    })?;
    let slots: FogSlots = parse_field(weather, "fog", Some(json))?;
    let slot = slots
        .get(TIME_SLOT)
        .ok_or_else(|| missing_slot(weather, "fog"))?;
    let near_color = near_color?.linear();
    let far_color = far_color?.linear();
    if slot.near == 0.0 && slot.far == 0.0 {
        return Ok(Fog::Disabled);
    }
    if !(slot.far > slot.near) {
        return Err(ExteriorWeatherError::InvertedFogRange {
            weather: weather.to_string(),
            near: slot.near,
            far: slot.far,
        });
    }
    Ok(Fog::Enabled(FogParams {
        near_color,
        far_color,
        near: slot.near * SCALE,
        far: slot.far * SCALE,
        power: slot.power,
        max: slot.max,
    }))
}

fn load_image_space(
    db: &DbClient,
    row: &WeatherRow,
) -> Result<ImageSpaceData, ExteriorWeatherError> {
    let slots: HashMap<String, Option<String>> =
        parse_field(&row.form_key, "image spaces", row.image_spaces.clone())?;
    let image_space = slots
        .get(TIME_SLOT)
        .cloned()
        .flatten()
        .ok_or_else(|| missing_slot(&row.form_key, "image space"))?;
    let data = db
        .get_image_space_data(&image_space)
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => ExteriorWeatherError::ImageSpaceNotFound {
                image_space: image_space.clone(),
            },
            other => other.into(),
        })?
        .ok_or_else(|| ExteriorWeatherError::ImageSpaceWithoutData {
            image_space: image_space.clone(),
        })?;
    let data: ImageSpaceData =
        serde_json::from_str(&data).map_err(|source| ExteriorWeatherError::Malformed {
            what: "image space",
            source,
        })?;
    Ok(data)
}

fn most_likely_weather(db: &DbClient, climate: &str) -> Result<String, ExteriorWeatherError> {
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
    let data: ClimateData =
        serde_json::from_str(&data).map_err(|source| ExteriorWeatherError::Malformed {
            what: "climate",
            source,
        })?;
    // Reversed so that among equal chances the first listed weather wins.
    data.weathers
        .into_iter()
        .rev()
        .max_by_key(|entry| entry.chance)
        .map(|entry| entry.weather)
        .ok_or_else(|| ExteriorWeatherError::EmptyWeatherList {
            climate: climate.to_string(),
        })
}

fn parse_field<T: serde::de::DeserializeOwned>(
    weather: &str,
    field: &'static str,
    json: Option<String>,
) -> Result<T, ExteriorWeatherError> {
    let json = json.ok_or_else(|| ExteriorWeatherError::MissingField {
        weather: weather.to_string(),
        field,
    })?;
    serde_json::from_str(&json).map_err(|source| ExteriorWeatherError::Malformed {
        what: field,
        source,
    })
}

fn missing_slot(weather: &str, entry: &str) -> ExteriorWeatherError {
    ExteriorWeatherError::MissingTimeSlot {
        weather: weather.to_string(),
        entry: entry.to_string(),
        slot: TIME_SLOT,
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

    const FORM_KEY: &str = "0012AB:Fallout4.esm";

    type Corrupt = fn(&mut WeatherRow, String);

    fn rgb(r: u8, g: u8, b: u8) -> Value {
        json!({"R": r, "G": g, "B": b})
    }

    #[allow(
        clippy::needless_pass_by_value,
        reason = "json! literals are passed by value"
    )]
    fn day(value: Value) -> Value {
        json!({"Day": value})
    }

    fn colors() -> Value {
        json!({
            "Sunlight": day(rgb(255, 0, 0)),
            "Sun": day(rgb(0, 255, 0)),
            "SkyUpper": day(rgb(0, 0, 255)),
            "SkyLower": day(rgb(255, 255, 0)),
            "Horizon": day(rgb(0, 255, 255)),
            "FogNear": day(rgb(0, 0, 0)),
            "FogFar": day(rgb(255, 255, 255)),
        })
    }

    fn directional_ambient(scale: f32) -> Value {
        let face = |r: u8| rgb(r, r, r);
        day(json!({
            "XPlus": face(0), "XMinus": face(0),
            "YPlus": face(255), "YMinus": face(0),
            "ZPlus": face(0), "ZMinus": face(0),
            "Specular": rgb(255, 255, 255),
            "Scale": scale,
        }))
    }

    fn fog(near: f32, far: f32) -> Value {
        day(json!({"Near": near, "Far": far, "Power": 1.5, "Max": 0.75}))
    }

    fn row() -> WeatherRow {
        WeatherRow {
            form_key: FORM_KEY.to_owned(),
            editor_id: Some("CommonwealthClear".to_owned()),
            colors: Some(colors().to_string()),
            directional_ambient: Some(directional_ambient(1.0).to_string()),
            image_spaces: Some(json!({"Day": null}).to_string()),
            fog: Some(fog(100.0, 1000.0).to_string()),
        }
    }

    fn load(row: WeatherRow) -> Result<ExteriorWeather, ExteriorWeatherError> {
        ExteriorWeather::from_row(
            row,
            ImageSpaceData {
                hdr: ImageSpaceHdr::default(),
                depth_of_field: None,
            },
        )
    }

    #[test]
    fn a_complete_row_yields_linear_colours_for_every_group() {
        let weather = load(row()).unwrap();
        assert_eq!(weather.sun_light, Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(weather.sun_disc, Vec3::new(0.0, 1.0, 0.0));
        assert_eq!(weather.sky_upper, Vec3::new(0.0, 0.0, 1.0));
        assert_eq!(weather.sky_lower, Vec3::new(1.0, 1.0, 0.0));
        assert_eq!(weather.horizon, Vec3::new(0.0, 1.0, 1.0));
        assert_eq!(weather.form_key, FORM_KEY);
        assert_eq!(weather.editor_id.as_deref(), Some("CommonwealthClear"));
    }

    #[test]
    fn weather_fog_is_scaled_into_engine_units() {
        let weather = load(row()).unwrap();
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
        let cube = load(row).unwrap().ambient_cube.0;
        // Bethesda +Y (forward) is the engine -Z face, the last one.
        let nonzero: Vec<usize> = (0..6).filter(|i| cube[*i] != Vec3::ZERO).collect();
        assert_eq!(nonzero, vec![5]);
        let unscaled = load(self::row()).unwrap().ambient_cube.0[5];
        assert!(cube[5].x < unscaled.x);
    }

    #[test]
    fn a_row_without_fog_data_still_loads_with_default_fog() {
        let mut row = row();
        row.fog = None;
        assert_eq!(load(row).unwrap().fog, None);
    }

    #[test]
    fn a_zero_fog_range_means_the_weather_has_no_fog() {
        let mut row = row();
        row.fog = Some(fog(0.0, 0.0).to_string());
        assert_eq!(load(row).unwrap().fog, Some(Fog::Disabled));
    }

    #[test]
    fn a_fog_range_that_does_not_increase_fails_the_weather() {
        for (near, far) in [(1000.0, 100.0), (500.0, 500.0)] {
            let mut row = row();
            row.fog = Some(fog(near, far).to_string());
            assert!(
                matches!(
                    load(row),
                    Err(ExteriorWeatherError::InvertedFogRange { .. })
                ),
                "{near}..{far}"
            );
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
        for (what, corrupt) in cases {
            let mut row = row();
            corrupt(&mut row, "{ not json".to_owned());
            let result = load(row);
            assert!(
                matches!(&result, Err(ExteriorWeatherError::Malformed { what: w, .. }) if *w == what),
                "{what}: {result:?}"
            );
        }
    }

    #[test]
    fn a_missing_colour_or_ambient_column_is_reported_with_the_weather() {
        let mut no_colors = row();
        no_colors.colors = None;
        assert!(matches!(
            load(no_colors),
            Err(ExteriorWeatherError::MissingField {
                field: "colors",
                ..
            })
        ));
        let mut no_ambient = row();
        no_ambient.directional_ambient = None;
        assert!(matches!(
            load(no_ambient),
            Err(ExteriorWeatherError::MissingField {
                field: "directional ambient",
                ..
            })
        ));
    }

    #[test]
    fn a_colour_group_without_the_day_slot_names_the_group() {
        let mut broken = colors();
        broken["Sunlight"] = json!({"Night": rgb(1, 2, 3)});
        let mut row = row();
        row.colors = Some(broken.to_string());
        let error = load(row).unwrap_err();
        assert!(
            matches!(&error, ExteriorWeatherError::MissingTimeSlot { entry, slot, .. } if entry == "Sunlight" && *slot == "Day"),
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
