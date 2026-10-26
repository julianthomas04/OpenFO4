use glam::Vec2;

use super::sides::{Side, Sides};
use crate::rendering::lighting::time_of_day::SlotBlend;
use crate::rendering::post::night_sky::CloudLayerDraw;
use crate::rendering::weather::CLOUD_LAYER_COUNT;

const _: () = assert!(
    CLOUD_LAYER_COUNT <= u8::MAX as usize + 1,
    "a cloud layer is named by a byte"
);

/// The layers of the clouds a frame draws, back to front: the layers of the weather that goes out
/// first and over them those of the weather that comes in, each in the order of its layer index.
pub(super) fn cloud_layers(sides: &Sides, time: SlotBlend) -> Vec<CloudLayerDraw> {
    sides
        .back_to_front()
        .flat_map(|side| side_layers(side, time))
        .collect()
}

/// The layers of one weather that show: those that are enabled, have a texture and colours for
/// the time of day, and are not wholly transparent. A layer is as opaque as its weather is much
/// of the frame.
fn side_layers<'a>(
    side: &Side<'a>,
    time: SlotBlend,
) -> impl Iterator<Item = CloudLayerDraw> + use<'a> {
    let weight = side.role.weight();
    side.weather
        .clouds
        .iter()
        .flat_map(|clouds| clouds.layers.iter().enumerate())
        .filter_map(move |(index, layer)| {
            if !layer.enabled {
                return None;
            }
            let texture = layer.texture.clone()?;
            let values = layer.sample(time)?;
            let alpha = values.alpha * weight;
            (alpha > 0.0).then(|| CloudLayerDraw {
                layer: index as u8,
                texture,
                color: values.color,
                alpha,
                scroll_speed: Vec2::new(layer.x_speed, layer.y_speed),
            })
        })
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
    use crate::rendering::lighting::time_of_day::TimeSlot;
    use crate::rendering::weather::clouds::fixtures::{clouds, layer};
    use crate::rendering::weather::definition::WeatherDef;
    use crate::rendering::weather::frame::fixtures::{
        A, B, DAY, DUSK, NIGHT, library, settled, transition, weather, with_clouds, without_clouds,
    };
    use crate::rendering::weather::library::fixtures::library_of;
    use crate::rendering::weather::{CLOUD_TEXTURE_COUNT, Clouds, WeatherLibrary};
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
        assert!(dusk.color.abs_diff_eq(Vec3::new(0.75, 0.0, 0.25), 1e-6));
        assert!((dusk.alpha - 0.625).abs() < 1e-6);
    }

    #[test]
    fn while_a_weather_changes_the_one_that_goes_out_is_drawn_first_and_the_one_that_comes_in_over_it()
     {
        let layers = layers_of(&red_and_blue(), &transition(B, A, 0.25), DAY);
        assert_eq!(layers.len(), 2 * CLOUD_TEXTURE_COUNT);
        let (leaving, coming) = layers.split_at(CLOUD_TEXTURE_COUNT);
        assert!(leaving.iter().all(|layer| layer.color == Vec3::X));
        assert!(coming.iter().all(|layer| layer.color == Vec3::Z));
        assert_eq!(layer_indices(leaving), textured());
        assert_eq!(layer_indices(coming), textured());
    }

    #[test]
    fn a_weather_is_as_opaque_as_it_is_much_of_the_frame() {
        let layers = layers_of(&red_and_blue(), &transition(B, A, 0.25), DAY);
        let (leaving, coming) = layers.split_at(CLOUD_TEXTURE_COUNT);
        assert!(leaving.iter().all(|layer| layer.alpha == 0.375));
        assert!(coming.iter().all(|layer| layer.alpha == 0.125));
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
