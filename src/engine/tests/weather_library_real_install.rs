//! The weather library of a converted installation: the Commonwealth climate loads every weather
//! with its clouds, precipitation, wind, lightning and sounds.
//!
//! Ignored by default (needs an installation converted with
//! `esm-only --tables weather,climates,shader_particle_geometries`). Run with
//! `cargo test --test weather_library_real_install -- --ignored --nocapture`; the database is
//! `OPENFO4_DB` when set, else the `data/data.db` of the installation in `OPENFO4_INSTALL_DIR`.

mod common;

use openfo4::assets::db::DbClient;
use openfo4::assets::types::effect_material::BlendFactor;
use openfo4::game_flow::spawn_locations::COMMONWEALTH;
use openfo4::rendering::lighting::exterior_weather::ExteriorWeatherSlots;
use openfo4::rendering::weather::{CLOUD_LAYER_COUNT, StartupWeather, WeatherDef, WeatherLibrary};
use openfo4::weather::{SoundKind, WeatherFlag};

const DEFAULT_CLIMATE_VOLATILITY: u8 = 175;
const COMMONWEALTH_CLIMATE_CHANCE_SUM: u32 = 102;
const COMMONWEALTH_CLEAR: &str = "02B52A:Fallout4.esm";
const COMMONWEALTH_RAIN: &str = "1CA7E4:Fallout4.esm";
const COMMONWEALTH_MISTY_RAINY: &str = "1CD096:Fallout4.esm";

fn database() -> DbClient {
    DbClient::new(common::database_path().to_str().unwrap()).unwrap()
}

fn library() -> WeatherLibrary {
    WeatherLibrary::load(&database(), COMMONWEALTH).unwrap_or_else(|error| panic!("{error}"))
}

fn weather<'a>(library: &'a WeatherLibrary, form_key: &str) -> &'a WeatherDef {
    library
        .weathers()
        .iter()
        .find(|weather| weather.form_key == form_key)
        .unwrap_or_else(|| panic!("the library lacks weather {form_key}"))
}

#[test]
#[ignore = "needs a converted installation"]
fn the_commonwealth_climate_loads_all_its_weathers() {
    let library = library();

    assert_eq!(library.climate_volatility(), DEFAULT_CLIMATE_VOLATILITY);
    assert_eq!(library.entries().len(), 8);
    assert_eq!(library.weathers().len(), 8);
    assert_eq!(
        library
            .entries()
            .iter()
            .map(|entry| entry.chance)
            .sum::<u32>(),
        COMMONWEALTH_CLIMATE_CHANCE_SUM
    );
    let initial = library.initial_weather();
    assert_eq!(initial.form_key, COMMONWEALTH_CLEAR);
    assert_eq!(initial.editor_id.as_deref(), Some("CommonwealthClear"));
    for weather in library.weathers() {
        let clouds = weather
            .clouds
            .as_ref()
            .unwrap_or_else(|| panic!("{} has no clouds", weather.form_key));
        assert_eq!(clouds.layers.len(), CLOUD_LAYER_COUNT);
        assert!(
            clouds
                .layers
                .iter()
                .any(|layer| layer.enabled && layer.texture.is_some()),
            "{} shows no textured cloud layer",
            weather.form_key
        );
        assert!(
            weather
                .sounds
                .iter()
                .any(|sound| sound.kind == SoundKind::Default),
            "{} has no default sound",
            weather.form_key
        );
    }
}

#[test]
#[ignore = "needs a converted installation"]
fn only_the_two_rainy_weathers_have_precipitation_a_resolved_texture_and_material() {
    let library = library();

    let mut rainy: Vec<&str> = library
        .weathers()
        .iter()
        .filter(|weather| weather.precipitation.is_some())
        .map(|weather| weather.form_key.as_str())
        .collect();
    rainy.sort_unstable();
    assert_eq!(rainy, [COMMONWEALTH_RAIN, COMMONWEALTH_MISTY_RAINY]);
    for key in rainy {
        let weather = weather(&library, key);
        let rain = weather.precipitation.as_ref().unwrap();
        assert!(rain.texture.is_some(), "{key} has no rain texture");
        assert!(rain.material.is_some(), "{key} has no rain material");
        let material = rain
            .material_data
            .unwrap_or_else(|| panic!("{key} has no rain material data"));
        assert_eq!(
            material.blend_factors(),
            Some((BlendFactor::SrcAlpha, BlendFactor::One)),
            "{key}"
        );
        assert_eq!(material.base_color, [205, 213, 220], "{key}");
        assert!(
            rain.box_size > 0 && rain.particle_density > 0.0,
            "{key}: {rain:?}"
        );
        for flag in [
            WeatherFlag::Rainy,
            WeatherFlag::RainOcclusion,
            WeatherFlag::HudRainEffects,
        ] {
            assert!(weather.flags.contains(flag), "{key} lacks {flag:?}");
        }
    }
}

#[test]
#[ignore = "needs a converted installation"]
fn the_rain_weather_has_thunder_and_a_lightning_colour() {
    let library = library();

    let rain = weather(&library, COMMONWEALTH_RAIN);
    assert!(
        rain.sounds
            .iter()
            .any(|sound| sound.kind == SoundKind::Thunder)
    );
    assert!(
        rain.sounds
            .iter()
            .any(|sound| sound.kind == SoundKind::Precipitation)
    );
    assert!(rain.has_thunder_sound());
    assert!(rain.lightning.frequency > 0.0);
    assert!(
        rain.lightning.color_srgb.iter().any(|channel| *channel > 0),
        "{:?}",
        rain.lightning
    );
}

#[test]
#[ignore = "needs a converted installation"]
fn the_static_start_and_the_library_begin_on_the_same_weather() {
    let db = database();
    let library = WeatherLibrary::load(&db, COMMONWEALTH).unwrap_or_else(|error| panic!("{error}"));
    let slots =
        ExteriorWeatherSlots::load(&db, COMMONWEALTH).unwrap_or_else(|error| panic!("{error}"));

    assert_eq!(slots.form_key(), library.initial_weather().form_key);
}

#[test]
#[ignore = "needs a converted installation"]
fn the_boot_path_builds_the_weather_cycle_from_the_installation() {
    let startup =
        StartupWeather::load(&database(), COMMONWEALTH).unwrap_or_else(|error| panic!("{error}"));

    assert_eq!(startup.table.initial(), startup.library.initial());
    assert_eq!(startup.table.entries(), startup.library.entries());
    assert_eq!(
        startup.table.climate_volatility(),
        DEFAULT_CLIMATE_VOLATILITY
    );
}
