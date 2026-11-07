use std::sync::Arc;

use glam::Vec2;

use super::sides::Sides;
use crate::rendering::lighting::lerp::Lerp;
use crate::rendering::lighting::time_of_day::SlotBlend;
use crate::rendering::post::night_sky::CloudLayerDraw;
use crate::rendering::weather::CLOUD_LAYER_COUNT;
use crate::rendering::weather::clouds::CloudSlotValues;
use crate::rendering::weather::definition::WeatherDef;

const _: () = assert!(
    CLOUD_LAYER_COUNT <= u8::MAX as usize + 1,
    "a cloud layer is named by a byte"
);

/// The layers of the clouds a frame draws, back to front. While one weather turns into another, a
/// layer both weathers show with the same texture changes from the one to the other in a single
/// draw, as the game measurably cross-fades it. A layer only one of them shows there fades with
/// the share of its weather: those of the weather that goes out first, then the others in the
/// order of their layer index.
pub(super) fn cloud_layers(sides: &Sides, time: SlotBlend) -> Vec<CloudLayerDraw> {
    let incoming = layers_at(sides.incoming.weather, time);
    let Some(outgoing) = &sides.outgoing else {
        return incoming.iter().filter_map(LayerAt::draw).collect();
    };
    let progress = sides.incoming.role.weight();
    let mut outgoing = layers_at(outgoing.weather, time);
    let mut front = Vec::with_capacity(incoming.len());
    for layer in incoming {
        let partner = outgoing.iter().position(|old| {
            old.index == layer.index && old.texture.eq_ignore_ascii_case(&layer.texture)
        });
        front.push(match partner {
            Some(position) => outgoing.remove(position).cross_fade(&layer, progress),
            None => layer.faded(progress),
        });
    }
    outgoing
        .into_iter()
        .map(|layer| layer.faded(1.0 - progress))
        .chain(front)
        .filter_map(|layer| layer.draw())
        .collect()
}

/// One cloud layer of one weather at the time of day.
#[derive(Debug, Clone)]
struct LayerAt {
    index: u8,
    texture: Arc<str>,
    values: CloudSlotValues,
    scroll_speed: Vec2,
}

impl LayerAt {
    fn faded(mut self, weight: f32) -> Self {
        self.values.alpha *= weight;
        self
    }

    fn cross_fade(self, other: &Self, progress: f32) -> Self {
        Self {
            values: self.values.lerp(&other.values, progress),
            scroll_speed: self.scroll_speed.lerp(other.scroll_speed, progress),
            ..self
        }
    }

    /// The draw of a layer that has not faded out.
    fn draw(&self) -> Option<CloudLayerDraw> {
        (self.values.alpha > 0.0).then(|| CloudLayerDraw {
            layer: self.index,
            texture: Arc::clone(&self.texture),
            color: self.values.linear_color(),
            alpha: self.values.alpha,
            scroll_speed: self.scroll_speed,
        })
    }
}

/// The layers of a weather that show, in the order of their index: those that are enabled, have a
/// texture and colours for the time of day, and are not wholly transparent. A transparent layer is
/// left out, so its colour never blends into the other weather's.
fn layers_at(weather: &WeatherDef, time: SlotBlend) -> Vec<LayerAt> {
    weather
        .clouds
        .iter()
        .flat_map(|clouds| clouds.layers.iter().enumerate())
        .filter_map(|(index, layer)| {
            if !layer.enabled {
                return None;
            }
            let values = layer.sample(time)?;
            (values.alpha > 0.0).then_some(())?;
            Some(LayerAt {
                index: index as u8,
                texture: layer.texture.clone()?,
                values,
                scroll_speed: Vec2::new(layer.x_speed, layer.y_speed),
            })
        })
        .collect()
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the colours, alphas and weights of the tests are exact"
)]
mod tests {
    use glam::Vec3;
    use serde_json::json;

    use super::*;
    use crate::rendering::lighting::cell_lighting::srgb_unit_to_linear;
    use crate::rendering::lighting::time_of_day::TimeSlot;
    use crate::rendering::weather::clouds::fixtures::{clouds, layer};
    use crate::rendering::weather::definition::WeatherDef;
    use crate::rendering::weather::frame::fixtures::{
        A, B, DAY, DUSK, NIGHT, library, settled, transition, weather, with_clouds, without_clouds,
    };
    use crate::rendering::weather::library::fixtures::library_of;
    use crate::rendering::weather::{CLOUD_TEXTURE_COUNT, CloudLayer, Clouds, WeatherLibrary};
    use crate::sim::WeatherSnapshot;

    const RED: [u8; 3] = [255, 0, 0];
    const BLUE: [u8; 3] = [0, 0, 255];

    fn layers_of(
        library: &WeatherLibrary,
        snapshot: &WeatherSnapshot<'_>,
        time: SlotBlend,
    ) -> Vec<CloudLayerDraw> {
        cloud_layers(&Sides::of(library, snapshot).unwrap(), time)
    }

    fn red_and_blue() -> WeatherLibrary {
        library_of(vec![
            with_clouds(weather("A"), |_| RED, |_| 0.5),
            with_clouds(weather("B"), |_| BLUE, |_| 0.5),
        ])
    }

    fn layer_indices(layers: &[CloudLayerDraw]) -> Vec<u8> {
        layers.iter().map(|layer| layer.layer).collect()
    }

    fn textured() -> Vec<u8> {
        (0..CLOUD_TEXTURE_COUNT as u8).collect()
    }

    fn with_layers(
        mut weather: WeatherDef,
        edit: impl FnOnce(&mut serde_json::Value),
    ) -> WeatherDef {
        let mut value = clouds();
        edit(&mut value);
        weather.clouds = Some(Clouds::parse(&weather.form_key, &value.to_string()).unwrap());
        weather
    }

    #[test]
    fn a_settled_weather_draws_its_layers_with_a_texture_in_the_order_of_their_index() {
        let layers = layers_of(&library(), &settled(A), DAY);
        assert_eq!(layer_indices(&layers), textured());
    }

    #[test]
    fn a_layer_is_drawn_with_the_texture_colour_alpha_and_speed_of_its_weather() {
        let layers = layers_of(&library(), &settled(A), DAY);
        let last = &layers[CLOUD_TEXTURE_COUNT - 1];
        assert_eq!(&*last.texture, "textures\\sky\\cloud28.dds");
        assert_eq!(last.color, Vec3::X);
        assert_eq!(last.alpha, 0.5);
        assert_eq!(last.scroll_speed, Vec2::new(0.01, -0.02));
    }

    #[test]
    fn a_layer_that_is_not_enabled_is_not_drawn() {
        let mut weather = weather("A");
        weather.clouds.as_mut().unwrap().layers[3].enabled = false;
        let layers = layers_of(&library_of(vec![weather]), &settled(A), DAY);
        let mut expected = textured();
        expected.remove(3);
        assert_eq!(layer_indices(&layers), expected);
    }

    #[test]
    fn a_layer_without_colours_is_not_drawn() {
        let weather = with_layers(weather("A"), |value| {
            value["layers"][5] = json!({
                "enabled": true, "x_speed": 0.0, "y_speed": 0.0, "colors": null, "alphas": null,
            });
        });
        let layers = layers_of(&library_of(vec![weather]), &settled(A), DAY);
        let indices = layer_indices(&layers);
        assert!(!indices.contains(&5));
        assert!(indices.contains(&4) && indices.contains(&6));
    }

    #[test]
    fn a_layer_that_is_wholly_transparent_at_the_time_of_day_is_not_drawn() {
        let weather = with_layers(weather("A"), |value| {
            value["layers"][2] = layer();
            value["layers"][2]["alphas"]["Day"] = json!(0.0);
        });
        let library = library_of(vec![weather]);
        assert!(!layer_indices(&layers_of(&library, &settled(A), DAY)).contains(&2));
        assert!(layer_indices(&layers_of(&library, &settled(A), NIGHT)).contains(&2));
    }

    #[test]
    fn a_weather_whose_layers_are_all_transparent_draws_none() {
        let transparent = with_clouds(weather("A"), |_| RED, |_| 0.0);
        assert_eq!(
            layers_of(&library_of(vec![transparent]), &settled(A), DAY),
            []
        );
    }

    #[test]
    fn a_weather_without_clouds_draws_no_layer() {
        let library = library_of(vec![without_clouds(weather("A"))]);
        assert_eq!(layers_of(&library, &settled(A), DAY), []);
    }

    #[test]
    fn the_colour_and_alpha_of_a_layer_are_those_of_the_time_of_day() {
        let weather = with_clouds(
            weather("A"),
            |slot| if slot == TimeSlot::Night { BLUE } else { RED },
            |slot| if slot == TimeSlot::Night { 1.0 } else { 0.5 },
        );
        let library = library_of(vec![weather]);
        let at = |time| layers_of(&library, &settled(A), time)[0].clone();
        assert_eq!((at(DAY).color, at(DAY).alpha), (Vec3::X, 0.5));
        assert_eq!((at(NIGHT).color, at(NIGHT).alpha), (Vec3::Z, 1.0));
        let dusk = at(DUSK);
        let expected = Vec3::new(srgb_unit_to_linear(0.75), 0.0, srgb_unit_to_linear(0.25));
        assert!(dusk.color.abs_diff_eq(expected, 1e-6));
        assert!((dusk.alpha - 0.625).abs() < 1e-6);
    }

    /// `weather` with its layers edited by `edit`.
    fn edited(mut weather: WeatherDef, edit: impl Fn(usize, &mut CloudLayer)) -> WeatherDef {
        for (index, layer) in weather
            .clouds
            .as_mut()
            .unwrap()
            .layers
            .iter_mut()
            .enumerate()
        {
            edit(index, layer);
        }
        weather
    }

    fn other_texture(index: usize) -> Arc<str> {
        format!(r"textures\sky\other{index:02}.dds").into()
    }

    /// `weather` with another texture on every textured layer.
    fn retextured(weather: WeatherDef) -> WeatherDef {
        edited(weather, |index, layer| {
            if layer.texture.is_some() {
                layer.texture = Some(other_texture(index));
            }
        })
    }

    #[test]
    fn a_layer_both_weathers_show_with_one_texture_changes_in_one_draw_as_its_stored_colour_blends()
    {
        let layers = layers_of(&red_and_blue(), &transition(B, A, 0.25), DAY);
        assert_eq!(layer_indices(&layers), textured());
        let expected = Vec3::new(srgb_unit_to_linear(0.75), 0.0, srgb_unit_to_linear(0.25));
        for layer in &layers {
            assert!(layer.color.abs_diff_eq(expected, 1e-6), "{}", layer.color);
            assert_eq!(layer.alpha, 0.5);
        }
    }

    #[test]
    fn the_alpha_and_speed_of_a_shared_layer_change_with_the_transition() {
        let library = library_of(vec![
            with_clouds(weather("A"), |_| RED, |_| 0.2),
            edited(with_clouds(weather("B"), |_| RED, |_| 1.0), |_, layer| {
                layer.x_speed = 0.05;
            }),
        ]);
        let first = layers_of(&library, &transition(B, A, 0.25), DAY)[0].clone();
        assert!((first.alpha - 0.4).abs() < 1e-6);
        assert!(first.scroll_speed.abs_diff_eq(Vec2::new(0.02, -0.02), 1e-6));
    }

    #[test]
    fn layers_with_other_textures_fade_with_their_weather_the_one_that_goes_out_drawn_first() {
        let library = library_of(vec![
            with_clouds(weather("A"), |_| RED, |_| 0.5),
            retextured(with_clouds(weather("B"), |_| BLUE, |_| 0.5)),
        ]);
        let layers = layers_of(&library, &transition(B, A, 0.25), DAY);
        assert_eq!(layers.len(), 2 * CLOUD_TEXTURE_COUNT);
        let (leaving, coming) = layers.split_at(CLOUD_TEXTURE_COUNT);
        assert_eq!(layer_indices(leaving), textured());
        assert_eq!(layer_indices(coming), textured());
        assert!(
            leaving
                .iter()
                .all(|layer| layer.color == Vec3::X && layer.alpha == 0.375)
        );
        assert!(
            coming
                .iter()
                .all(|layer| layer.color == Vec3::Z && layer.alpha == 0.125)
        );
    }

    /// Red layers alone by day, at alpha 0.5 and three quarters of the frame: what is left of
    /// `A` a quarter of the way to `B`.
    fn faded_red(layer: &CloudLayerDraw) -> bool {
        layer.color == Vec3::X && layer.alpha == 0.375
    }

    fn blended_red_and_blue(layer: &CloudLayerDraw) -> bool {
        let expected = Vec3::new(srgb_unit_to_linear(0.75), 0.0, srgb_unit_to_linear(0.25));
        layer.color.abs_diff_eq(expected, 1e-6) && layer.alpha == 0.5
    }

    fn a_quarter_from_red_to(blue: WeatherDef) -> Vec<CloudLayerDraw> {
        let library = library_of(vec![with_clouds(weather("A"), |_| RED, |_| 0.5), blue]);
        layers_of(&library, &transition(B, A, 0.25), DAY)
    }

    fn blue() -> WeatherDef {
        with_clouds(weather("B"), |_| BLUE, |_| 0.5)
    }

    #[test]
    fn textures_that_differ_only_in_case_are_the_same_texture() {
        let layers = a_quarter_from_red_to(edited(blue(), |_, layer| {
            layer.texture = layer
                .texture
                .as_deref()
                .map(|texture| texture.to_uppercase().into());
        }));
        assert_eq!(layer_indices(&layers), textured());
        assert!(layers.iter().all(blended_red_and_blue));
    }

    #[test]
    fn a_layer_without_a_texture_in_the_weather_that_comes_leaves_the_other_to_fade_out() {
        let layers = a_quarter_from_red_to(edited(blue(), |index, layer| {
            if index == 3 {
                layer.texture = None;
            }
        }));
        assert_eq!(layers.len(), CLOUD_TEXTURE_COUNT);
        assert_eq!(layers[0].layer, 3);
        assert!(faded_red(&layers[0]));
        assert!(layers[1..].iter().all(blended_red_and_blue));
    }

    #[test]
    fn a_layer_the_weather_that_comes_does_not_have_leaves_the_other_to_fade_out() {
        let layers = a_quarter_from_red_to(edited(blue(), |index, layer| {
            if index == 5 {
                layer.enabled = false;
            }
        }));
        assert_eq!(layers.len(), CLOUD_TEXTURE_COUNT);
        assert_eq!(layers[0].layer, 5);
        assert!(faded_red(&layers[0]));
        assert!(layers[1..].iter().all(blended_red_and_blue));
    }

    #[test]
    fn a_transparent_layer_lends_none_of_its_colour_to_the_other_weather() {
        let layers = a_quarter_from_red_to(with_clouds(weather("B"), |_| BLUE, |_| 0.0));
        assert_eq!(layer_indices(&layers), textured());
        assert!(layers.iter().all(faded_red));
    }

    #[test]
    fn shared_and_unshared_layers_mix_with_the_unshared_ones_of_the_weather_that_goes_out_first() {
        let layers = a_quarter_from_red_to(edited(blue(), |index, layer| {
            if index % 2 == 0 && layer.texture.is_some() {
                layer.texture = Some(other_texture(index));
            }
        }));
        let even: Vec<u8> = textured()
            .into_iter()
            .filter(|index| index % 2 == 0)
            .collect();
        let (leaving, rest) = layers.split_at(even.len());
        assert_eq!(layer_indices(leaving), even);
        assert!(leaving.iter().all(faded_red));
        assert_eq!(layer_indices(rest), textured());
        for layer in rest {
            if layer.layer % 2 == 0 {
                assert!(layer.color == Vec3::Z && layer.alpha == 0.125, "{layer:?}");
            } else {
                assert!(blended_red_and_blue(layer), "{layer:?}");
            }
        }
    }

    #[test]
    fn a_weather_that_is_none_of_the_frame_draws_no_layer() {
        let library = red_and_blue();
        let start = layers_of(&library, &transition(B, A, 0.0), DAY);
        assert_eq!(start.len(), CLOUD_TEXTURE_COUNT);
        assert!(start.iter().all(|layer| layer.color == Vec3::X));
        let end = layers_of(&library, &transition(B, A, 1.0), DAY);
        assert_eq!(end.len(), CLOUD_TEXTURE_COUNT);
        assert!(end.iter().all(|layer| layer.color == Vec3::Z));
    }

    #[test]
    fn a_weather_without_clouds_leaves_the_clouds_of_the_other_as_they_are() {
        let library = library_of(vec![weather("A"), without_clouds(weather("B"))]);
        let layers = layers_of(&library, &transition(B, A, 0.5), DAY);
        assert_eq!(layer_indices(&layers), textured());
        assert!(layers.iter().all(|layer| layer.alpha == 0.25));
    }
}
