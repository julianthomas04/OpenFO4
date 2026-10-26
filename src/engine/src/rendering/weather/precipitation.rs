use std::sync::{Arc, Once};

use glam::{UVec2, Vec2};
use serde::Deserialize;

use super::error::WeatherLibraryError;
use crate::assets::db::weather_library::ParticleGeometryRow;
use crate::rendering::effects::material_bindings::path_below_textures;
use crate::rendering::lighting::exterior_weather::parse_json;

static SNOW_UNSUPPORTED_LOGGED: Once = Once::new();

const RAIN_TYPE: u8 = 0;
const SNOW_TYPE: u8 = 1;

/// How the rain of a weather is drawn, from its shader particle geometry record.
#[derive(Debug, Clone, PartialEq)]
pub struct PrecipitationParams {
    /// The resolved diffuse texture, named below the textures folder; the renderer decides what to
    /// do without one.
    pub texture: Option<Arc<str>>,
    /// The authored BGEM path.
    pub material: Option<String>,
    pub gravity_velocity: f32,
    pub rotation_velocity: f32,
    pub particle_size: Vec2,
    pub center_offset_min: f32,
    pub center_offset_max: f32,
    pub initial_rotation: f32,
    pub subtextures: UVec2,
    pub box_size: u32,
    pub particle_density: f32,
}

#[derive(Debug, Deserialize)]
struct RawGeometry {
    #[serde(rename = "type")]
    kind: u8,
    gravity_velocity: f32,
    rotation_velocity: f32,
    particle_size_x: f32,
    particle_size_y: f32,
    center_offset_min: f32,
    center_offset_max: f32,
    initial_rotation: f32,
    subtextures_x: u32,
    subtextures_y: u32,
    box_size: u32,
    particle_density: f32,
}

impl PrecipitationParams {
    /// `Ok(None)` for snow, which is not rendered yet.
    pub(super) fn parse(
        weather: &str,
        particle_geometry: &str,
        row: ParticleGeometryRow,
    ) -> Result<Option<Self>, WeatherLibraryError> {
        let missing = |field| WeatherLibraryError::ParticleGeometryMissingField {
            weather: weather.to_string(),
            particle_geometry: particle_geometry.to_string(),
            field,
        };
        let data = row.data.as_deref().ok_or_else(|| missing("data"))?;
        let raw: RawGeometry = parse_json(weather, "particle geometry", data)?;
        match raw.kind {
            RAIN_TYPE => {}
            SNOW_TYPE => {
                SNOW_UNSUPPORTED_LOGGED.call_once(|| {
                    tracing::warn!(
                        "weather {weather} has snow precipitation ({particle_geometry}), which is unsupported; snowy weathers show no precipitation"
                    );
                });
                return Ok(None);
            }
            kind => {
                return Err(WeatherLibraryError::UnknownPrecipitationType {
                    weather: weather.to_string(),
                    particle_geometry: particle_geometry.to_string(),
                    kind,
                });
            }
        }
        Ok(Some(Self {
            texture: row
                .particle_texture
                .as_deref()
                .map(path_below_textures)
                .map(Arc::from),
            material: row.particle_material,
            gravity_velocity: raw.gravity_velocity,
            rotation_velocity: raw.rotation_velocity,
            particle_size: Vec2::new(raw.particle_size_x, raw.particle_size_y),
            center_offset_min: raw.center_offset_min,
            center_offset_max: raw.center_offset_max,
            initial_rotation: raw.initial_rotation,
            subtextures: UVec2::new(raw.subtextures_x, raw.subtextures_y),
            box_size: raw.box_size,
            particle_density: raw.particle_density,
        }))
    }
}

#[cfg(test)]
pub(super) mod fixtures {
    use serde_json::{Value, json};

    pub(in crate::rendering::weather) fn geometry(kind: u8) -> Value {
        json!({
            "type": kind,
            "gravity_velocity": 9.5,
            "rotation_velocity": 0.25,
            "particle_size_x": 0.5,
            "particle_size_y": 4.0,
            "center_offset_min": 1.0,
            "center_offset_max": 2.0,
            "initial_rotation": 0.0,
            "subtextures_x": 4,
            "subtextures_y": 2,
            "box_size": 1200,
            "particle_density": 0.75,
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact sentinels and copied values"
)]
mod tests {
    use super::fixtures::geometry;
    use super::*;
    use crate::rendering::lighting::exterior_weather::ExteriorWeatherError;

    const WEATHER: &str = "0012AB:Fallout4.esm";
    const GEOMETRY: &str = "00AA01:Fallout4.esm";

    fn row(data: &serde_json::Value, texture: Option<&str>) -> ParticleGeometryRow {
        ParticleGeometryRow {
            data: Some(data.to_string()),
            particle_texture: texture.map(str::to_owned),
            particle_material: None,
        }
    }

    fn parse(row: ParticleGeometryRow) -> Result<Option<PrecipitationParams>, WeatherLibraryError> {
        PrecipitationParams::parse(WEATHER, GEOMETRY, row)
    }

    #[test]
    fn a_rain_geometry_yields_its_parameters_and_texture() {
        let params = parse(row(&geometry(0), Some("textures\\rain.dds")))
            .unwrap()
            .unwrap();
        assert_eq!(params.texture.as_deref(), Some("rain.dds"));
        assert_eq!(params.gravity_velocity, 9.5);
        assert_eq!(params.particle_size, Vec2::new(0.5, 4.0));
        assert_eq!(params.subtextures, UVec2::new(4, 2));
        assert_eq!(params.box_size, 1200);
        assert_eq!(params.particle_density, 0.75);
        assert_eq!(
            (params.center_offset_min, params.center_offset_max),
            (1.0, 2.0)
        );
    }

    #[test]
    fn a_texture_path_with_the_textures_folder_is_named_below_it() {
        let params = parse(row(
            &geometry(0),
            Some(r"textures\Effects\FXRaindrops_D.DDS"),
        ))
        .unwrap()
        .unwrap();

        assert_eq!(
            params.texture.as_deref(),
            Some(r"Effects\FXRaindrops_D.DDS")
        );
    }

    #[test]
    fn snow_is_unsupported_and_yields_no_parameters_even_without_a_texture() {
        assert_eq!(parse(row(&geometry(1), None)).unwrap(), None);
    }

    #[test]
    fn an_unknown_precipitation_type_is_an_error() {
        let error = parse(row(&geometry(7), Some("t.dds"))).unwrap_err();
        assert!(
            matches!(&error, WeatherLibraryError::UnknownPrecipitationType { kind: 7, particle_geometry, .. } if particle_geometry == GEOMETRY),
            "{error:?}"
        );
    }

    #[test]
    fn missing_data_names_the_geometry_and_field() {
        let no_data = ParticleGeometryRow {
            data: None,
            particle_texture: Some("t.dds".to_owned()),
            particle_material: None,
        };
        assert!(matches!(
            parse(no_data),
            Err(WeatherLibraryError::ParticleGeometryMissingField { field: "data", .. })
        ));
    }

    #[test]
    fn a_geometry_without_a_resolved_texture_keeps_its_material() {
        let mut geometry_row = row(&geometry(0), None);
        geometry_row.particle_material = Some("materials\\rain.bgem".to_owned());
        let params = parse(geometry_row).unwrap().unwrap();
        assert_eq!(params.texture, None);
        assert_eq!(params.material.as_deref(), Some("materials\\rain.bgem"));
    }

    #[test]
    fn malformed_geometry_json_is_an_error_not_a_default() {
        let mut incomplete = geometry(0);
        incomplete.as_object_mut().unwrap().remove("box_size");
        for data in ["{ not json".to_owned(), incomplete.to_string()] {
            let row = ParticleGeometryRow {
                data: Some(data),
                particle_texture: Some("t.dds".to_owned()),
                particle_material: None,
            };
            assert!(matches!(
                parse(row),
                Err(WeatherLibraryError::Lighting(ExteriorWeatherError::Malformed {
                    weather,
                    field: "particle geometry",
                    ..
                })) if weather == WEATHER
            ));
        }
    }
}
