use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use serde::Deserialize;

use super::error::WeatherLibraryError;
use crate::rendering::lighting::cell_lighting::srgb8_to_linear;
use crate::rendering::lighting::exterior_weather::parse_json;
use crate::rendering::lighting::lerp::{Lerp, lerp_fields};
use crate::rendering::lighting::time_of_day::{SlotBlend, TimeSlot};

pub const CLOUD_TEXTURE_COUNT: usize = 29;
pub const CLOUD_LAYER_COUNT: usize = 32;

/// What one cloud layer looks like in one time slot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CloudSlotValues {
    pub color: Vec3,
    /// The alpha channel of the colour, 0..1.
    pub color_alpha: f32,
    /// The layer's opacity, 0..1.
    pub alpha: f32,
}

lerp_fields!(CloudSlotValues {
    color,
    color_alpha,
    alpha
});

#[derive(Debug, Clone, PartialEq)]
pub struct CloudLayer {
    pub enabled: bool,
    /// The cloud texture with the layer's index, when the weather has one.
    pub texture: Option<Arc<str>>,
    pub x_speed: f32,
    pub y_speed: f32,
    /// In the order of [`TimeSlot::index`]; `None` for a layer without colours and alphas.
    slots: Option<[CloudSlotValues; TimeSlot::COUNT]>,
}

impl CloudLayer {
    pub fn at(&self, slot: TimeSlot) -> Option<&CloudSlotValues> {
        self.slots.as_ref().map(|slots| &slots[slot.index()])
    }

    pub fn sample(&self, blend: SlotBlend) -> Option<CloudSlotValues> {
        Some(self.at(blend.from)?.lerp(self.at(blend.to)?, blend.t))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Clouds {
    pub layers: [CloudLayer; CLOUD_LAYER_COUNT],
}

#[derive(Debug, Deserialize)]
struct RawClouds {
    textures: Vec<Option<String>>,
    layers: Vec<RawLayer>,
}

#[derive(Debug, Deserialize)]
struct RawLayer {
    enabled: Option<bool>,
    x_speed: Option<f32>,
    y_speed: Option<f32>,
    colors: Option<HashMap<String, [u8; 4]>>,
    alphas: Option<HashMap<String, f32>>,
}

impl Clouds {
    pub(super) fn parse(weather: &str, json: &str) -> Result<Self, WeatherLibraryError> {
        let raw: RawClouds = parse_json(weather, "clouds", json)?;
        expect_count(
            weather,
            "cloud textures",
            CLOUD_TEXTURE_COUNT,
            raw.textures.len(),
        )?;
        expect_count(weather, "cloud layers", CLOUD_LAYER_COUNT, raw.layers.len())?;
        let layers = raw
            .layers
            .iter()
            .enumerate()
            .map(|(index, layer)| {
                let texture = raw.textures.get(index).cloned().flatten().map(Arc::from);
                parse_layer(weather, index, texture, layer)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            layers: layers
                .try_into()
                .expect("the layer count was checked against CLOUD_LAYER_COUNT"),
        })
    }
}

fn expect_count(
    weather: &str,
    field: &'static str,
    expected: usize,
    found: usize,
) -> Result<(), WeatherLibraryError> {
    if expected == found {
        return Ok(());
    }
    Err(WeatherLibraryError::WrongCount {
        weather: weather.to_string(),
        field,
        expected,
        found,
    })
}

fn parse_layer(
    weather: &str,
    index: usize,
    texture: Option<Arc<str>>,
    raw: &RawLayer,
) -> Result<CloudLayer, WeatherLibraryError> {
    let missing = |entry, slot: TimeSlot| WeatherLibraryError::MissingCloudSlot {
        weather: weather.to_string(),
        layer: index,
        entry,
        slot: slot.json_key(),
    };
    let unpaired = |present, absent| WeatherLibraryError::UnpairedCloudValues {
        weather: weather.to_string(),
        layer: index,
        present,
        absent,
    };
    let slots = match (&raw.colors, &raw.alphas) {
        (Some(colors), Some(alphas)) => Some(TimeSlot::try_map_all(|slot| {
            let [r, g, b, a] = *colors
                .get(slot.json_key())
                .ok_or_else(|| missing("colors", slot))?;
            let alpha = *alphas
                .get(slot.json_key())
                .ok_or_else(|| missing("alphas", slot))?;
            Ok::<_, WeatherLibraryError>(CloudSlotValues {
                color: srgb8_to_linear([r, g, b]),
                color_alpha: f32::from(a) / 255.0,
                alpha,
            })
        })?),
        (None, None) => None,
        (Some(_), None) => return Err(unpaired("colors", "alphas")),
        (None, Some(_)) => return Err(unpaired("alphas", "colors")),
    };
    Ok(CloudLayer {
        enabled: raw.enabled.unwrap_or(false),
        texture,
        x_speed: raw.x_speed.unwrap_or(0.0),
        y_speed: raw.y_speed.unwrap_or(0.0),
        slots,
    })
}

#[cfg(test)]
pub(super) mod fixtures {
    use serde_json::{Value, json};

    use super::*;

    fn per_slot(value: &Value) -> Value {
        Value::Object(
            TimeSlot::ALL
                .into_iter()
                .map(|slot| (slot.json_key().to_owned(), value.clone()))
                .collect(),
        )
    }

    pub(in crate::rendering::weather) fn layer() -> Value {
        json!({
            "enabled": true,
            "x_speed": 0.01,
            "y_speed": -0.02,
            "colors": per_slot(&json!([255, 0, 0, 128])),
            "alphas": per_slot(&json!(0.5)),
        })
    }

    pub(in crate::rendering::weather) fn clouds() -> Value {
        let textures: Vec<Value> = (0..CLOUD_TEXTURE_COUNT)
            .map(|index| json!(format!("textures\\sky\\cloud{index:02}.dds")))
            .collect();
        json!({
            "textures": textures,
            "layers": vec![layer(); CLOUD_LAYER_COUNT],
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

    use super::fixtures::{clouds, layer};
    use super::*;
    use crate::rendering::lighting::exterior_weather::ExteriorWeatherError;

    const WEATHER: &str = "0012AB:Fallout4.esm";

    fn parse(value: &serde_json::Value) -> Result<Clouds, WeatherLibraryError> {
        Clouds::parse(WEATHER, &value.to_string())
    }

    #[test]
    fn a_complete_cloud_block_yields_all_layers_with_their_textures() {
        let clouds = parse(&clouds()).unwrap();
        assert_eq!(clouds.layers.len(), CLOUD_LAYER_COUNT);
        let first = &clouds.layers[0];
        assert!(first.enabled);
        assert_eq!(first.texture.as_deref(), Some("textures\\sky\\cloud00.dds"));
        assert_eq!((first.x_speed, first.y_speed), (0.01, -0.02));
        assert_eq!(
            clouds.layers[CLOUD_TEXTURE_COUNT - 1].texture.as_deref(),
            Some("textures\\sky\\cloud28.dds")
        );
    }

    #[test]
    fn a_database_converted_with_a_cloud_model_is_still_read() {
        let mut value = clouds();
        value["model"] = json!("sky\\clouds.nif");
        assert_eq!(parse(&value).unwrap(), parse(&clouds()).unwrap());
    }

    #[test]
    fn layers_beyond_the_last_texture_have_none() {
        let clouds = parse(&clouds()).unwrap();
        for layer in &clouds.layers[CLOUD_TEXTURE_COUNT..] {
            assert_eq!(layer.texture, None);
        }
    }

    #[test]
    fn a_null_texture_leaves_its_layer_untextured() {
        let mut value = clouds();
        value["textures"][3] = json!(null);
        assert_eq!(parse(&value).unwrap().layers[3].texture, None);
    }

    #[test]
    fn slot_colours_are_linear_and_alphas_are_unit_values() {
        let clouds = parse(&clouds()).unwrap();
        let values = clouds.layers[0].at(TimeSlot::Night).unwrap();
        assert_eq!(values.color, Vec3::new(1.0, 0.0, 0.0));
        assert!((values.color_alpha - 128.0 / 255.0).abs() < 1e-6);
        assert_eq!(values.alpha, 0.5);
    }

    #[test]
    fn slot_colours_are_decoded_from_srgb_channel_by_channel() {
        let mut value = clouds();
        value["layers"][0]["colors"]["Day"] = json!([128, 64, 200, 51]);
        let clouds = parse(&value).unwrap();
        let values = clouds.layers[0].at(TimeSlot::Day).unwrap();
        for (channel, expected) in [
            (values.color.x, 0.215_860_5),
            (values.color.y, 0.051_269_46),
            (values.color.z, 0.577_580_4),
        ] {
            assert!((channel - expected).abs() < 1e-6, "{channel} vs {expected}");
        }
        assert!(
            (values.color_alpha - 0.2).abs() < 1e-6,
            "the colour alpha is linear already"
        );
    }

    #[test]
    fn sampling_blends_both_slots_of_the_layer() {
        let mut value = clouds();
        value["layers"][0]["alphas"]["Night"] = json!(1.0);
        let clouds = parse(&value).unwrap();
        let blend = SlotBlend {
            from: TimeSlot::Day,
            to: TimeSlot::Night,
            t: 0.5,
        };
        assert_eq!(clouds.layers[0].sample(blend).unwrap().alpha, 0.75);
    }

    #[test]
    fn a_layer_with_null_fields_is_a_disabled_layer_without_slot_values() {
        let mut value = clouds();
        value["layers"][4] = json!({
            "enabled": null, "x_speed": null, "y_speed": null,
            "colors": null, "alphas": null,
        });
        let clouds = parse(&value).unwrap();
        let layer = &clouds.layers[4];
        assert!(!layer.enabled);
        assert_eq!((layer.x_speed, layer.y_speed), (0.0, 0.0));
        assert_eq!(layer.at(TimeSlot::Day), None);
        let blend = SlotBlend {
            from: TimeSlot::Day,
            to: TimeSlot::Night,
            t: 0.5,
        };
        assert_eq!(layer.sample(blend), None);
        assert!(clouds.layers[5].at(TimeSlot::Day).is_some());
    }

    #[test]
    fn a_layer_with_colours_but_no_alphas_or_the_other_way_round_is_an_error() {
        for (present, absent) in [("colors", "alphas"), ("alphas", "colors")] {
            let mut value = clouds();
            value["layers"][6][absent] = json!(null);
            let error = parse(&value).unwrap_err();
            assert!(
                matches!(
                    &error,
                    WeatherLibraryError::UnpairedCloudValues { weather, layer: 6, present: p, absent: a }
                        if weather == WEATHER && *p == present && *a == absent
                ),
                "{present} without {absent}: {error:?}"
            );
        }
    }

    #[test]
    fn a_wrong_layer_or_texture_count_is_an_error() {
        let mut few_layers = clouds();
        few_layers["layers"].as_array_mut().unwrap().pop();
        assert!(matches!(
            parse(&few_layers),
            Err(WeatherLibraryError::WrongCount {
                field: "cloud layers",
                expected: 32,
                found: 31,
                ..
            })
        ));
        let mut few_textures = clouds();
        few_textures["textures"].as_array_mut().unwrap().pop();
        assert!(matches!(
            parse(&few_textures),
            Err(WeatherLibraryError::WrongCount {
                field: "cloud textures",
                ..
            })
        ));
    }

    #[test]
    fn a_layer_missing_a_slot_names_the_layer_entry_and_slot() {
        let mut value = clouds();
        value["layers"][7]["colors"]
            .as_object_mut()
            .unwrap()
            .remove("LateSunset");
        let error = parse(&value).unwrap_err();
        assert!(
            matches!(
                &error,
                WeatherLibraryError::MissingCloudSlot {
                    layer: 7,
                    entry: "colors",
                    slot: "LateSunset",
                    ..
                }
            ),
            "{error:?}"
        );
        let mut value = clouds();
        value["layers"][2]["alphas"]
            .as_object_mut()
            .unwrap()
            .remove("Day");
        assert!(matches!(
            parse(&value),
            Err(WeatherLibraryError::MissingCloudSlot {
                layer: 2,
                entry: "alphas",
                slot: "Day",
                ..
            })
        ));
    }

    #[test]
    fn malformed_cloud_json_is_an_error_naming_the_weather() {
        for text in ["{ not json", r#"{"textures":[],"layers":"x"}"#] {
            let error = Clouds::parse(WEATHER, text).unwrap_err();
            assert!(
                matches!(&error, WeatherLibraryError::Lighting(ExteriorWeatherError::Malformed { weather, field: "clouds", .. }) if weather == WEATHER),
                "{error:?}"
            );
        }
    }

    #[test]
    fn a_colour_channel_outside_a_byte_is_malformed() {
        let mut value = clouds();
        value["layers"][0] = layer();
        value["layers"][0]["colors"]["Day"] = json!([256, 0, 0, 0]);
        assert!(matches!(
            parse(&value),
            Err(WeatherLibraryError::Lighting(
                ExteriorWeatherError::Malformed {
                    field: "clouds",
                    ..
                }
            ))
        ));
    }
}
