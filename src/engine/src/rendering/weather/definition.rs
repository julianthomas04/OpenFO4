use super::clouds::Clouds;
use super::data::{
    FadeRange, LightningParams, WeatherData, WeatherFlags, WeatherSound, WindParams, parse_sounds,
};
use super::error::{WeatherLibraryError, require_field};
use super::precipitation::PrecipitationParams;
use crate::assets::db::DbClient;
use crate::assets::db::weather_library::{ParticleGeometryRow, WeatherLibraryRow};
use crate::rendering::lighting::exterior_weather::{
    ClimateInfo, ExteriorWeatherError, ExteriorWeatherSlots,
};

/// Everything the engine knows about one weather: its lighting per time slot and the sky,
/// precipitation, wind, lightning and sound data a weather cycle needs.
#[derive(Debug, Clone, PartialEq)]
pub struct WeatherDef {
    pub form_key: String,
    pub editor_id: Option<String>,
    pub slots: ExteriorWeatherSlots,
    /// `None` when the weather has no cloud data.
    pub clouds: Option<Clouds>,
    /// `None` when the weather has no precipitation or only an unsupported kind (snow).
    pub precipitation: Option<PrecipitationParams>,
    pub precipitation_fade: FadeRange,
    pub wind: WindParams,
    pub lightning: LightningParams,
    /// The record's unscaled byte.
    pub trans_delta_raw: u8,
    pub sun_glare: f32,
    pub sun_damage: f32,
    pub visual_effect_begin: f32,
    pub visual_effect_end: f32,
    pub flags: WeatherFlags,
    /// 1.0 when the weather has no multiplier.
    pub volatility_mult: f32,
    pub visibility_mult: Option<f32>,
    pub sounds: Vec<WeatherSound>,
}

impl WeatherDef {
    pub(super) fn load(
        db: &DbClient,
        weather_key: &str,
        climate: &ClimateInfo,
    ) -> Result<Self, WeatherLibraryError> {
        let slots = ExteriorWeatherSlots::load_weather(db, weather_key, climate)?;
        let row = db
            .get_weather_library_row(weather_key)
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => ExteriorWeatherError::WeatherNotFound {
                    weather: weather_key.to_string(),
                }
                .into(),
                other => WeatherLibraryError::from(other),
            })?;
        let geometry = row
            .precipitation
            .as_deref()
            .map(|key| {
                db.get_particle_geometry(key).map_err(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => {
                        WeatherLibraryError::ParticleGeometryNotFound {
                            weather: weather_key.to_string(),
                            particle_geometry: key.to_string(),
                        }
                    }
                    other => other.into(),
                })
            })
            .transpose()?;
        Self::from_rows(slots, &row, geometry)
    }

    /// `geometry` is the row of the particle geometry named by `row.precipitation`.
    pub(super) fn from_rows(
        slots: ExteriorWeatherSlots,
        row: &WeatherLibraryRow,
        geometry: Option<ParticleGeometryRow>,
    ) -> Result<Self, WeatherLibraryError> {
        let form_key = slots.form_key().to_string();
        let editor_id = slots.editor_id().map(str::to_owned);
        let data = WeatherData::parse(
            &form_key,
            require_field(&form_key, "data", row.data.as_deref())?,
        )?;
        let clouds = row
            .clouds
            .as_deref()
            .map(|json| Clouds::parse(&form_key, json))
            .transpose()?;
        let precipitation = match (row.precipitation.as_deref(), geometry) {
            (Some(key), Some(geometry)) => PrecipitationParams::parse(&form_key, key, geometry)?,
            (Some(key), None) => {
                return Err(WeatherLibraryError::ParticleGeometryNotFound {
                    weather: form_key,
                    particle_geometry: key.to_string(),
                });
            }
            (None, _) => None,
        };
        let sounds = parse_sounds(&form_key, row.sounds.as_deref())?;
        Ok(Self {
            form_key,
            editor_id,
            slots,
            clouds,
            precipitation,
            precipitation_fade: data.precipitation_fade,
            wind: data.wind,
            lightning: data.lightning,
            trans_delta_raw: data.trans_delta_raw,
            sun_glare: data.sun_glare,
            sun_damage: data.sun_damage,
            visual_effect_begin: data.visual_effect_begin,
            visual_effect_end: data.visual_effect_end,
            flags: data.flags,
            volatility_mult: data.volatility_mult,
            visibility_mult: data.visibility_mult,
            sounds,
        })
    }
}

#[cfg(test)]
pub(super) mod fixtures {
    use serde_json::json;

    use super::*;
    use crate::rendering::lighting::exterior_weather::fixtures::slots_for;
    use crate::rendering::weather::clouds::fixtures::clouds;
    use crate::rendering::weather::data::fixtures::data;
    use crate::rendering::weather::precipitation::fixtures::geometry;

    pub(in crate::rendering::weather) const GEOMETRY_KEY: &str = "00AA01:Fallout4.esm";

    pub(in crate::rendering::weather) fn library_row() -> WeatherLibraryRow {
        WeatherLibraryRow {
            precipitation: Some(GEOMETRY_KEY.to_owned()),
            clouds: Some(clouds().to_string()),
            data: Some(data().to_string()),
            sounds: Some(
                json!([{"kind": "precipitation", "sound": "00BB01:Fallout4.esm"}]).to_string(),
            ),
        }
    }

    pub(in crate::rendering::weather) fn rain_geometry() -> ParticleGeometryRow {
        ParticleGeometryRow {
            data: Some(geometry(0).to_string()),
            particle_texture: Some("textures\\rain.dds".to_owned()),
            particle_material: Some("materials\\rain.bgem".to_owned()),
        }
    }

    pub(in crate::rendering::weather) fn weather_def(form_key: &str) -> WeatherDef {
        WeatherDef::from_rows(
            slots_for(form_key, "TestWeather"),
            &library_row(),
            Some(rain_geometry()),
        )
        .unwrap()
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact sentinels and copied values"
)]
mod tests {
    use serde_json::json;

    use super::fixtures::{GEOMETRY_KEY, library_row, rain_geometry, weather_def};
    use super::*;
    use crate::rendering::lighting::exterior_weather::fixtures::slots_for;

    const FORM_KEY: &str = "0012AB:Fallout4.esm";

    fn from_rows(
        row: &WeatherLibraryRow,
        geometry: Option<ParticleGeometryRow>,
    ) -> Result<WeatherDef, WeatherLibraryError> {
        WeatherDef::from_rows(slots_for(FORM_KEY, "Clear"), row, geometry)
    }

    #[test]
    fn a_complete_weather_carries_every_part() {
        let def = weather_def(FORM_KEY);
        assert_eq!(def.form_key, FORM_KEY);
        assert_eq!(def.editor_id.as_deref(), Some("TestWeather"));
        assert_eq!(def.slots.form_key(), FORM_KEY);
        assert!(def.clouds.is_some());
        assert_eq!(
            def.precipitation
                .as_ref()
                .and_then(|rain| rain.texture.as_deref()),
            Some("textures\\rain.dds")
        );
        assert_eq!(def.wind.speed, 0.4);
        assert_eq!(def.precipitation_fade.end_fade_out, 0.9);
        assert_eq!(def.lightning.frequency, 0.25);
        assert_eq!(def.sounds.len(), 1);
        assert_eq!(def.sounds[0].descriptor, "00BB01:Fallout4.esm");
    }

    #[test]
    fn a_weather_without_clouds_or_precipitation_still_loads() {
        let mut row = library_row();
        row.clouds = None;
        row.precipitation = None;
        let def = from_rows(&row, None).unwrap();
        assert!(def.clouds.is_none());
        assert!(def.precipitation.is_none());
    }

    #[test]
    fn snow_precipitation_leaves_the_weather_without_precipitation() {
        let geometry = ParticleGeometryRow {
            data: Some(crate::rendering::weather::precipitation::fixtures::geometry(1).to_string()),
            particle_texture: None,
            particle_material: None,
        };
        let def = from_rows(&library_row(), Some(geometry)).unwrap();
        assert!(def.precipitation.is_none());
    }

    #[test]
    fn a_weather_without_data_is_an_error_naming_it() {
        let mut row = library_row();
        row.data = None;
        let error = from_rows(&row, Some(rain_geometry())).unwrap_err();
        assert!(
            matches!(&error, WeatherLibraryError::MissingField { weather, field: "data" } if weather == FORM_KEY),
            "{error:?}"
        );
    }

    #[test]
    fn precipitation_without_its_geometry_row_is_an_error() {
        let error = from_rows(&library_row(), None).unwrap_err();
        assert!(
            matches!(&error, WeatherLibraryError::ParticleGeometryNotFound { particle_geometry, .. } if particle_geometry == GEOMETRY_KEY),
            "{error:?}"
        );
    }

    #[test]
    fn a_broken_cell_fails_the_whole_weather_naming_the_field() {
        let mut bad_clouds = library_row();
        bad_clouds.clouds = Some("{ not json".to_owned());
        let mut bad_data = library_row();
        bad_data.data = Some(json!({"wind_speed": 1.0}).to_string());
        let mut bad_sounds = library_row();
        bad_sounds.sounds = Some("[1]".to_owned());
        for (field, row) in [
            ("clouds", bad_clouds),
            ("data", bad_data),
            ("sounds", bad_sounds),
        ] {
            let error = from_rows(&row, Some(rain_geometry())).unwrap_err();
            assert!(
                matches!(&error, WeatherLibraryError::Malformed { weather, field: f, .. } if weather == FORM_KEY && *f == field),
                "{field}: {error:?}"
            );
        }
    }
}
