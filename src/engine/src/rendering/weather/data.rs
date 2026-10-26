use glam::Vec3;
use serde::Deserialize;

use super::error::{WeatherLibraryError, parse_json};
use super::flag_set::flag_set;
use crate::rendering::lighting::cell_lighting::srgb_to_linear;

flag_set! {
    /// The weather's flag byte.
    WeatherFlags of WeatherFlag {
        Pleasant = 1,
        Cloudy = 2,
        Rainy = 4,
        Snow = 8,
        SkyStaticsAlwaysVisible = 16,
        SkyStaticsFollowsSun = 32,
        RainOcclusion = 64,
        HudRainEffects = 128,
    }
}

/// The situation a weather sound plays in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundKind {
    Default,
    Precipitation,
    Wind,
    Thunder,
}

/// How far in and out an effect fades, as the game stores it for precipitation and thunder.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FadeRange {
    pub begin_fade_in: f32,
    pub end_fade_out: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindParams {
    /// 0..1.
    pub speed: f32,
    /// 0..1; `None` when the weather's data predates the field.
    pub turbulence: Option<f32>,
    /// The record's unscaled byte.
    pub direction_raw: u8,
    /// The record's unscaled byte.
    pub direction_range_raw: u8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightningParams {
    pub fade: FadeRange,
    /// 0..1.
    pub frequency: f32,
    /// Linear.
    pub color: Vec3,
}

/// The scalar values of a weather's `DATA` record.
#[derive(Debug, Clone, PartialEq)]
pub struct WeatherData {
    pub wind: WindParams,
    pub precipitation_fade: FadeRange,
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
}

#[derive(Debug, Deserialize)]
struct RawData {
    wind_speed: f32,
    wind_turbulence: Option<f32>,
    wind_direction_raw: u8,
    wind_direction_range_raw: u8,
    trans_delta_raw: u8,
    sun_glare: f32,
    sun_damage: f32,
    precipitation_begin_fade_in: f32,
    precipitation_end_fade_out: f32,
    thunder_begin_fade_in: f32,
    thunder_end_fade_out: f32,
    thunder_frequency: f32,
    lightning_color: [u8; 3],
    flags: u32,
    visual_effect_begin: f32,
    visual_effect_end: f32,
    volatility_mult: Option<f32>,
    visibility_mult: Option<f32>,
}

impl WeatherData {
    pub(super) fn parse(weather: &str, json: &str) -> Result<Self, WeatherLibraryError> {
        let raw: RawData = parse_json(weather, "data", json)?;
        let flags = WeatherFlags::from_bits(raw.flags).ok_or_else(|| {
            WeatherLibraryError::UnknownFlagBits {
                weather: weather.to_string(),
                field: "data flags",
                bits: raw.flags,
            }
        })?;
        let [r, g, b] = raw.lightning_color;
        Ok(Self {
            wind: WindParams {
                speed: raw.wind_speed,
                turbulence: raw.wind_turbulence,
                direction_raw: raw.wind_direction_raw,
                direction_range_raw: raw.wind_direction_range_raw,
            },
            precipitation_fade: FadeRange {
                begin_fade_in: raw.precipitation_begin_fade_in,
                end_fade_out: raw.precipitation_end_fade_out,
            },
            lightning: LightningParams {
                fade: FadeRange {
                    begin_fade_in: raw.thunder_begin_fade_in,
                    end_fade_out: raw.thunder_end_fade_out,
                },
                frequency: raw.thunder_frequency,
                color: Vec3::new(srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b)),
            },
            trans_delta_raw: raw.trans_delta_raw,
            sun_glare: raw.sun_glare,
            sun_damage: raw.sun_damage,
            visual_effect_begin: raw.visual_effect_begin,
            visual_effect_end: raw.visual_effect_end,
            flags,
            volatility_mult: raw.volatility_mult.unwrap_or(1.0),
            visibility_mult: raw.visibility_mult,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeatherSound {
    pub kind: SoundKind,
    /// Form key of the sound descriptor.
    pub descriptor: String,
}

#[derive(Debug, Deserialize)]
struct RawSound {
    kind: SoundKind,
    sound: String,
}

pub(super) fn parse_sounds(
    weather: &str,
    json: Option<&str>,
) -> Result<Vec<WeatherSound>, WeatherLibraryError> {
    let Some(json) = json else {
        return Ok(Vec::new());
    };
    let raw: Vec<RawSound> = parse_json(weather, "sounds", json)?;
    Ok(raw
        .into_iter()
        .map(|sound| WeatherSound {
            kind: sound.kind,
            descriptor: sound.sound,
        })
        .collect())
}

#[cfg(test)]
pub(super) mod fixtures {
    use serde_json::{Value, json};

    pub(in crate::rendering::weather) fn data() -> Value {
        json!({
            "wind_speed": 0.4,
            "wind_turbulence": 0.2,
            "wind_direction_raw": 128,
            "wind_direction_range_raw": 32,
            "trans_delta_raw": 4,
            "sun_glare": 0.5,
            "sun_damage": 0.0,
            "precipitation_begin_fade_in": 0.1,
            "precipitation_end_fade_out": 0.9,
            "thunder_begin_fade_in": 0.2,
            "thunder_end_fade_out": 0.8,
            "thunder_frequency": 0.25,
            "lightning_color": [255, 0, 128],
            "flags": 6,
            "visual_effect_begin": 0.0,
            "visual_effect_end": 1.0,
            "volatility_mult": 2.0,
            "visibility_mult": 0.5,
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact sentinels and copied values"
)]
mod tests {
    use serde_json::json;

    use super::fixtures::data;
    use super::*;

    const WEATHER: &str = "0012AB:Fallout4.esm";

    fn parse(value: &serde_json::Value) -> Result<WeatherData, WeatherLibraryError> {
        WeatherData::parse(WEATHER, &value.to_string())
    }

    #[test]
    fn a_complete_data_block_yields_wind_fades_lightning_and_flags() {
        let data = parse(&data()).unwrap();
        assert_eq!(
            data.wind,
            WindParams {
                speed: 0.4,
                turbulence: Some(0.2),
                direction_raw: 128,
                direction_range_raw: 32,
            }
        );
        assert_eq!(
            data.precipitation_fade,
            FadeRange {
                begin_fade_in: 0.1,
                end_fade_out: 0.9
            }
        );
        assert_eq!(data.lightning.fade.end_fade_out, 0.8);
        assert_eq!(data.lightning.frequency, 0.25);
        assert_eq!(data.lightning.color.x, 1.0);
        assert_eq!(data.lightning.color.y, 0.0);
        assert_eq!(data.trans_delta_raw, 4);
        assert_eq!(data.visibility_mult, Some(0.5));
        assert_eq!(data.volatility_mult, 2.0);
    }

    #[test]
    fn flags_are_typed() {
        let flags = parse(&data()).unwrap().flags;
        assert!(flags.contains(WeatherFlag::Cloudy));
        assert!(flags.contains(WeatherFlag::Rainy));
        assert!(!flags.contains(WeatherFlag::Pleasant));
        assert!(!flags.contains(WeatherFlag::Snow));
    }

    #[test]
    fn every_flag_has_its_own_bit_and_the_set_round_trips() {
        let bits: Vec<u8> = WeatherFlag::ALL.iter().map(|flag| flag.bit()).collect();
        assert_eq!(bits, [1, 2, 4, 8, 16, 32, 64, 128]);
        assert_eq!(WeatherFlags::from_bits(255).unwrap().bits(), 255);
    }

    #[test]
    fn flag_bits_no_flag_names_are_an_error() {
        let mut value = data();
        value["flags"] = json!(256);
        assert!(matches!(
            parse(&value),
            Err(WeatherLibraryError::UnknownFlagBits {
                field: "data flags",
                bits: 256,
                ..
            })
        ));
    }

    #[test]
    fn null_optionals_stay_absent_and_the_volatility_multiplier_defaults_to_one() {
        let mut value = data();
        value["wind_turbulence"] = json!(null);
        value["volatility_mult"] = json!(null);
        value.as_object_mut().unwrap().remove("visibility_mult");
        let data = parse(&value).unwrap();
        assert_eq!(data.wind.turbulence, None);
        assert_eq!(data.volatility_mult, 1.0);
        assert_eq!(data.visibility_mult, None);
    }

    #[test]
    fn malformed_data_is_an_error_naming_the_weather_and_field() {
        let mut missing = data();
        missing.as_object_mut().unwrap().remove("thunder_frequency");
        let mut wrong_type = data();
        wrong_type["wind_speed"] = json!("fast");
        let mut byte_overflow = data();
        byte_overflow["wind_direction_raw"] = json!(300);
        for value in [missing, wrong_type, byte_overflow] {
            let error = parse(&value).unwrap_err();
            assert!(
                matches!(&error, WeatherLibraryError::Malformed { weather, field: "data", .. } if weather == WEATHER),
                "{error:?}"
            );
        }
        assert!(matches!(
            WeatherData::parse(WEATHER, "{ not json"),
            Err(WeatherLibraryError::Malformed { field: "data", .. })
        ));
    }

    #[test]
    fn sounds_carry_their_kind_and_descriptor() {
        let sounds = parse_sounds(
            WEATHER,
            Some(r#"[{"kind": "thunder", "sound": "00AA:Fallout4.esm"}]"#),
        )
        .unwrap();
        assert_eq!(
            sounds,
            [WeatherSound {
                kind: SoundKind::Thunder,
                descriptor: "00AA:Fallout4.esm".to_owned()
            }]
        );
    }

    #[test]
    fn every_kind_spelling_is_understood() {
        let json = r#"[{"kind":"default","sound":"a"},{"kind":"precipitation","sound":"b"},{"kind":"wind","sound":"c"},{"kind":"thunder","sound":"d"}]"#;
        let kinds: Vec<SoundKind> = parse_sounds(WEATHER, Some(json))
            .unwrap()
            .iter()
            .map(|sound| sound.kind)
            .collect();
        assert_eq!(
            kinds,
            [
                SoundKind::Default,
                SoundKind::Precipitation,
                SoundKind::Wind,
                SoundKind::Thunder
            ]
        );
    }

    #[test]
    fn a_weather_without_sounds_has_none() {
        assert_eq!(parse_sounds(WEATHER, None).unwrap(), []);
        assert_eq!(parse_sounds(WEATHER, Some("[]")).unwrap(), []);
    }

    #[test]
    fn malformed_sounds_and_unknown_kinds_are_errors() {
        for json in [
            r#"[{"kind": "default"}]"#,
            r#"[{"kind": "rumble", "sound": "S"}]"#,
        ] {
            assert!(
                matches!(
                    parse_sounds(WEATHER, Some(json)),
                    Err(WeatherLibraryError::Malformed {
                        field: "sounds",
                        ..
                    })
                ),
                "{json}"
            );
        }
    }
}
