use std::sync::Arc;

use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
#[error("malformed climate JSON: {0}")]
pub struct ClimateSkyError(#[from] serde_json::Error);

/// Which moons the climate shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Moons {
    pub masser: bool,
    pub secunda: bool,
}

/// The sky keys of a climate `data` JSON. Each is `None` when the key is missing,
/// as in a database converted before the installer exported it; the caller decides what that means.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ClimateSky {
    #[serde(default)]
    pub moon_phase_length: Option<u32>,
    #[serde(default)]
    pub moons: Option<Moons>,
    #[serde(default)]
    pub model_path: Option<String>,
}

/// The sun sprite textures of a climate, as paths below the textures folder (`Sky\Sun_d.dds`).
/// `None` where the climate names no texture.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClimateSunTextures {
    pub sun: Option<Arc<str>>,
    pub glare: Option<Arc<str>>,
}

impl ClimateSunTextures {
    /// An empty name is no name.
    pub fn new(sun: Option<&str>, glare: Option<&str>) -> Self {
        let named = |path: Option<&str>| {
            path.map(str::trim)
                .filter(|path| !path.is_empty())
                .map(Arc::from)
        };
        Self {
            sun: named(sun),
            glare: named(glare),
        }
    }
}

impl ClimateSky {
    pub fn parse(climate_json: &str) -> Result<Self, ClimateSkyError> {
        Ok(serde_json::from_str(climate_json)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_climate_with_every_sky_key_parses_them() {
        let json = r#"{"Weathers":[],"SunriseBegin":"05:00:00","MoonPhaseLength":4,"Moons":{"Masser":true,"Secunda":false},"ModelPath":"Sky\\Stars.nif"}"#;
        assert_eq!(
            ClimateSky::parse(json).unwrap(),
            ClimateSky {
                moon_phase_length: Some(4),
                moons: Some(Moons {
                    masser: true,
                    secunda: false
                }),
                model_path: Some("Sky\\Stars.nif".to_owned()),
            }
        );
    }

    #[test]
    fn an_older_climate_without_sky_keys_yields_none() {
        let json = r#"{"Weathers":[],"SunriseBegin":"05:00:00"}"#;
        assert_eq!(
            ClimateSky::parse(json).unwrap(),
            ClimateSky {
                moon_phase_length: None,
                moons: None,
                model_path: None
            }
        );
    }

    #[test]
    fn null_values_read_as_missing() {
        let json = r#"{"MoonPhaseLength":null,"Moons":null,"ModelPath":null}"#;
        let sky = ClimateSky::parse(json).unwrap();
        assert_eq!(sky.moon_phase_length, None);
        assert_eq!(sky.moons, None);
        assert_eq!(sky.model_path, None);
    }

    #[test]
    fn each_key_can_be_present_on_its_own() {
        let sky = ClimateSky::parse(r#"{"MoonPhaseLength":3}"#).unwrap();
        assert_eq!(sky.moon_phase_length, Some(3));
        assert_eq!(sky.moons, None);
    }

    #[test]
    fn sun_texture_names_are_kept_and_blank_ones_are_none() {
        let textures = ClimateSunTextures::new(Some(r"Sky\Sun_d.dds"), Some("  "));
        assert_eq!(textures.sun.as_deref(), Some(r"Sky\Sun_d.dds"));
        assert_eq!(textures.glare, None);
        assert_eq!(
            ClimateSunTextures::new(None, Some("")),
            ClimateSunTextures::default()
        );
    }

    #[test]
    fn values_of_the_wrong_type_are_errors() {
        for bad in [
            r#"{"MoonPhaseLength":"four"}"#,
            r#"{"MoonPhaseLength":-1}"#,
            r#"{"Moons":{"Masser":1,"Secunda":false}}"#,
            r#"{"Moons":{"Masser":true}}"#,
            r#"{"ModelPath":5}"#,
            "{ not json",
        ] {
            assert!(ClimateSky::parse(bad).is_err(), "{bad}");
        }
    }
}
