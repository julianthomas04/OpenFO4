//! `WeatherLibrary::load` against the installer's real schema (`esm_schema.sql`) loaded into an
//! in-memory `SQLite` database, seeded with plain SQL.

use openfo4::assets::db::DbClient;
use openfo4::assets::types::effect_material::BlendFactor;
use openfo4::rendering::lighting::exterior_weather::{ExteriorWeatherError, ExteriorWeatherSlots};
use openfo4::rendering::lighting::god_rays::{GodRaysData, SunInScatter};
use openfo4::rendering::weather::{
    StartupWeather, StartupWeatherError, WeatherDef, WeatherLibrary, WeatherLibraryError,
};
use openfo4::weather::{SoundKind, WeatherId, WeatherTableError};
use rusqlite::Connection;
use rusqlite::types::Value;
use serde_json::{Value as Json, json};

const SCHEMA: &str = include_str!("../../installer/file_converters/esm_schema.sql");
const SLOTS: [&str; 8] = [
    "Sunrise",
    "Day",
    "Sunset",
    "Night",
    "EarlySunrise",
    "LateSunrise",
    "EarlySunset",
    "LateSunset",
];
const WORLDSPACE: &str = "0000A1:Fallout4.esm";
const CLIMATE: &str = "0000C1:Fallout4.esm";
const CLEAR: &str = "0000D1:Fallout4.esm";
const RAIN: &str = "0000D2:Fallout4.esm";
const RAIN_GEOMETRY: &str = "0000E1:Fallout4.esm";
const IMAGE_SPACE: &str = "0000F1:Fallout4.esm";
const DAY_RAYS: &str = "0000B5:Fallout4.esm";

fn text(s: &str) -> Value {
    Value::Text(s.to_string())
}

fn per_slot(value: &Json) -> Json {
    SLOTS
        .iter()
        .map(|slot| ((*slot).to_owned(), value.clone()))
        .collect()
}

fn insert(conn: &Connection, table: &str, row: &[(&str, Value)]) {
    let columns = row.iter().map(|(c, _)| *c).collect::<Vec<_>>().join(", ");
    let placeholders = vec!["?"; row.len()].join(", ");
    conn.execute(
        &format!("INSERT INTO {table} ({columns}) VALUES ({placeholders})"),
        rusqlite::params_from_iter(row.iter().map(|(_, v)| v)),
    )
    .unwrap();
}

fn connection_without(dropped: &[(&str, &str)]) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(SCHEMA).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
    for (table, column) in dropped {
        conn.execute_batch(&format!("ALTER TABLE {table} DROP COLUMN {column};"))
            .unwrap();
    }
    conn
}

fn rgb() -> Json {
    json!({"R": 128, "G": 128, "B": 128})
}

fn colors() -> Json {
    let groups = [
        "Sunlight", "Sun", "SunGlare", "SkyUpper", "SkyLower", "Horizon", "Stars", "FogNear",
        "FogFar",
    ];
    groups
        .iter()
        .map(|group| ((*group).to_owned(), per_slot(&rgb())))
        .collect()
}

fn ambient() -> Json {
    per_slot(&json!({
        "XPlus": rgb(), "XMinus": rgb(), "YPlus": rgb(),
        "YMinus": rgb(), "ZPlus": rgb(), "ZMinus": rgb(),
        "Specular": rgb(), "Scale": 1.0,
    }))
}

fn fog() -> Json {
    let range = json!({"Near": 100.0, "Far": 1000.0, "Power": 1.0, "Max": 0.5});
    json!({"Day": range, "Night": range})
}

fn clouds() -> Json {
    let layer = json!({
        "enabled": true, "x_speed": 0.0, "y_speed": 0.0,
        "colors": per_slot(&json!([255, 255, 255, 255])),
        "alphas": per_slot(&json!(1.0)),
    });
    json!({
        "textures": vec![json!(null); 29],
        "layers": vec![layer; 32],
    })
}

fn weather_data(flags: u32) -> Json {
    json!({
        "wind_speed": 0.1, "wind_turbulence": 0.0,
        "wind_direction_raw": 0, "wind_direction_range_raw": 0,
        "trans_delta_raw": 0, "sun_glare": 0.0, "sun_damage": 0.0,
        "precipitation_begin_fade_in": 0.0, "precipitation_end_fade_out": 1.0,
        "thunder_begin_fade_in": 0.0, "thunder_end_fade_out": 1.0,
        "thunder_frequency": 0.0, "lightning_color": [255, 255, 255],
        "flags": flags, "visual_effect_begin": 0.0, "visual_effect_end": 1.0,
        "volatility_mult": null, "visibility_mult": null,
    })
}

fn seed_weather(conn: &Connection, form_key: &str, precipitation: Option<&str>, flags: u32) {
    insert(
        conn,
        "weather",
        &[
            ("form_key", text(form_key)),
            ("editor_id", text(&format!("Weather{form_key}"))),
            ("colors", text(&colors().to_string())),
            ("directional_ambient", text(&ambient().to_string())),
            (
                "image_spaces",
                text(&per_slot(&json!(IMAGE_SPACE)).to_string()),
            ),
            ("fog", text(&fog().to_string())),
            ("clouds", text(&clouds().to_string())),
            ("data", text(&weather_data(flags).to_string())),
            ("precipitation", precipitation.map_or(Value::Null, text)),
            (
                "sounds",
                text(
                    &json!([{"kind": "precipitation", "sound": "0000B1:Fallout4.esm"}]).to_string(),
                ),
            ),
        ],
    );
}

fn seeded(climate_weathers: &Json) -> Connection {
    seeded_into(connection_without(&[]), climate_weathers)
}

fn seeded_into(conn: Connection, climate_weathers: &Json) -> Connection {
    insert(
        &conn,
        "worldspaces",
        &[("form_key", text(WORLDSPACE)), ("climate", text(CLIMATE))],
    );
    let climate_data = json!({
        "Weathers": climate_weathers,
        "SunriseBegin": "05:00:00", "SunriseEnd": "09:00:00",
        "SunsetBegin": "17:00:00", "SunsetEnd": "21:00:00",
    });
    insert(
        &conn,
        "climates",
        &[
            ("form_key", text(CLIMATE)),
            ("data", text(&climate_data.to_string())),
            ("volatility", Value::Integer(45)),
        ],
    );
    insert(
        &conn,
        "image_spaces",
        &[
            ("form_key", text(IMAGE_SPACE)),
            (
                "data",
                text(
                    &json!({"Hdr": {
                        "EyeAdaptSpeed": 3.0, "SunlightScale": 1.0, "SkyScale": 1.0,
                        "MiddleGray": 0.18, "AutoExposureMin": 1.6, "AutoExposureMax": 3.25,
                    }})
                    .to_string(),
                ),
            ),
        ],
    );
    insert(
        &conn,
        "shader_particle_geometries",
        &[
            ("form_key", text(RAIN_GEOMETRY)),
            ("particle_texture", text("textures\\rain.dds")),
            (
                "particle_material_data",
                text(
                    &json!({
                        "blend_enabled": true, "blend_src": "src_alpha", "blend_dst": "one",
                        "base_color": [205, 213, 220],
                        "depth_write": false, "effect_lighting": true, "lighting_influence": 1.0,
                    })
                    .to_string(),
                ),
            ),
            (
                "data",
                text(
                    &json!({
                        "type": 0, "gravity_velocity": 9.0, "rotation_velocity": 0.0,
                        "particle_size_x": 1.0, "particle_size_y": 2.0,
                        "center_offset_min": 0.0, "center_offset_max": 0.0,
                        "initial_rotation": 0.0, "subtextures_x": 1, "subtextures_y": 1,
                        "box_size": 100, "particle_density": 1.0,
                    })
                    .to_string(),
                ),
            ),
        ],
    );
    seed_weather(&conn, CLEAR, None, 1);
    seed_weather(&conn, RAIN, Some(RAIN_GEOMETRY), 4);
    conn
}

fn two_weathers() -> Json {
    json!([
        {"Weather": CLEAR, "Chance": 60, "Global": null},
        {"Weather": RAIN, "Chance": 40, "Global": null},
    ])
}

fn load(conn: Connection) -> Result<WeatherLibrary, WeatherLibraryError> {
    WeatherLibrary::load(&DbClient::from_connection(conn).unwrap(), WORLDSPACE)
}

fn start(conn: Connection) -> Result<StartupWeather, StartupWeatherError> {
    StartupWeather::load(&DbClient::from_connection(conn).unwrap(), WORLDSPACE)
}

#[test]
fn the_library_loads_every_climate_weather_from_the_database() {
    let library = load(seeded(&two_weathers())).unwrap();
    assert_eq!(library.climate_volatility(), 45);
    assert_eq!(library.weathers().len(), 2);
    let initial = library.initial_weather();
    assert_eq!(initial.form_key, CLEAR);
    assert!(initial.precipitation.is_none());
    let rain = library
        .weathers()
        .iter()
        .find(|weather| weather.form_key == RAIN)
        .unwrap();
    assert_eq!(
        rain.precipitation
            .as_ref()
            .and_then(|rain| rain.texture.as_deref()),
        Some("rain.dds")
    );
    assert!(rain.clouds.is_some());
    assert_eq!(rain.sounds[0].kind, SoundKind::Precipitation);
}

fn rain_weather(library: &WeatherLibrary) -> &WeatherDef {
    library
        .weathers()
        .iter()
        .find(|weather| weather.form_key == RAIN)
        .unwrap()
}

#[test]
fn the_rain_carries_the_material_data_of_its_particle_geometry() {
    let library = load(seeded(&two_weathers())).unwrap();

    let material = rain_weather(&library)
        .precipitation
        .as_ref()
        .and_then(|rain| rain.material_data)
        .unwrap();

    assert_eq!(
        material.blend_factors(),
        Some((BlendFactor::SrcAlpha, BlendFactor::One))
    );
    assert_eq!(material.base_color, [205, 213, 220]);
}

#[test]
fn a_database_without_the_material_data_column_loads_the_rain_without_it() {
    let conn = seeded(&two_weathers());
    conn.execute_batch(
        "ALTER TABLE shader_particle_geometries DROP COLUMN particle_material_data;",
    )
    .unwrap();

    let library = load(conn).unwrap();

    let rain = rain_weather(&library).precipitation.as_ref().unwrap();
    assert_eq!(rain.material_data, None);
    assert!(rain.texture.is_some());
}

#[test]
fn malformed_material_data_fails_the_library_naming_the_weather() {
    let conn = seeded(&two_weathers());
    conn.execute(
        "UPDATE shader_particle_geometries SET particle_material_data = '{ not json'",
        [],
    )
    .unwrap();

    let error = load(conn).unwrap_err();

    assert!(
        matches!(
            &error,
            WeatherLibraryError::Lighting(ExteriorWeatherError::Malformed {
                weather,
                field: "particle material data",
                ..
            }) if weather == RAIN
        ),
        "{error:?}"
    );
}

#[test]
fn a_global_gated_entry_is_left_out_of_the_library_and_of_the_static_start_alike() {
    let weathers = json!([
        {"Weather": CLEAR, "Chance": 10, "Global": null},
        {"Weather": RAIN, "Chance": 90, "Global": "0000A9:Fallout4.esm"},
    ]);
    let db = DbClient::from_connection(seeded(&weathers)).unwrap();
    let library = WeatherLibrary::load(&db, WORLDSPACE).unwrap();
    assert_eq!(library.entries().len(), 1);
    assert_eq!(library.weathers().len(), 1);
    assert_eq!(library.initial_weather().form_key, CLEAR);
    let slots = ExteriorWeatherSlots::load(&db, WORLDSPACE).unwrap();
    assert_eq!(slots.form_key(), CLEAR);
}

#[test]
fn a_climate_with_only_gated_entries_loads_neither_the_library_nor_the_static_start() {
    let weathers = json!([{"Weather": RAIN, "Chance": 90, "Global": "0000A9:Fallout4.esm"}]);
    let db = DbClient::from_connection(seeded(&weathers)).unwrap();
    let library_error = WeatherLibrary::load(&db, WORLDSPACE).unwrap_err();
    assert!(
        matches!(
            &library_error,
            WeatherLibraryError::Lighting(ExteriorWeatherError::NoUngatedWeather { climate })
                if climate == CLIMATE
        ),
        "{library_error:?}"
    );
    let static_error = ExteriorWeatherSlots::load(&db, WORLDSPACE).unwrap_err();
    assert!(
        matches!(&static_error, ExteriorWeatherError::NoUngatedWeather { climate } if climate == CLIMATE),
        "{static_error:?}"
    );
}

#[test]
fn a_database_without_the_library_columns_asks_for_the_installer_rerun() {
    let conn = connection_without(&[("weather", "clouds"), ("weather", "data")]);
    let error = load(conn).unwrap_err();
    assert!(
        matches!(error, WeatherLibraryError::SchemaOutdated),
        "{error:?}"
    );
    assert!(
        error
            .to_string()
            .contains("esm-only --tables weather,climates,shader_particle_geometries")
    );
}

#[test]
fn a_database_with_only_some_of_a_column_group_is_a_schema_error_naming_the_missing_column() {
    let conn = connection_without(&[("shader_particle_geometries", "particle_texture")]);
    let Err(error) = DbClient::from_connection(conn) else {
        panic!("a partly converted column group must not open");
    };
    let message = error.to_string();
    assert!(message.contains("shader_particle_geometries"), "{message}");
    assert!(message.contains("particle_texture"), "{message}");
}

#[test]
fn a_climate_without_volatility_is_an_error_naming_it() {
    let conn = seeded(&two_weathers());
    conn.execute("UPDATE climates SET volatility = NULL", [])
        .unwrap();
    let error = load(conn).unwrap_err();
    assert!(
        matches!(&error, WeatherLibraryError::VolatilityMissing { climate } if climate == CLIMATE),
        "{error:?}"
    );
}

#[test]
fn malformed_weather_data_fails_the_library_naming_weather_and_field() {
    let conn = seeded(&two_weathers());
    conn.execute(
        "UPDATE weather SET data = '{ not json' WHERE form_key = ?",
        [RAIN],
    )
    .unwrap();
    let error = load(conn).unwrap_err();
    assert!(
        matches!(
            &error,
            WeatherLibraryError::Lighting(ExteriorWeatherError::Malformed { weather, field: "data", .. })
                if weather == RAIN
        ),
        "{error:?}"
    );
}

#[test]
fn a_malformed_image_space_list_fails_the_library_naming_the_weather() {
    let conn = seeded(&two_weathers());
    conn.execute(
        "UPDATE weather SET image_spaces = '{ not json' WHERE form_key = ?",
        [CLEAR],
    )
    .unwrap();
    let error = load(conn).unwrap_err();
    assert!(
        matches!(
            &error,
            WeatherLibraryError::Lighting(ExteriorWeatherError::Malformed { weather, field: "image spaces", .. })
                if weather == CLEAR
        ),
        "{error:?}"
    );
}

#[test]
fn malformed_image_space_data_fails_the_library_naming_the_image_space() {
    let conn = seeded(&two_weathers());
    conn.execute("UPDATE image_spaces SET data = '{ not json'", [])
        .unwrap();
    let error = load(conn).unwrap_err();
    assert!(
        matches!(
            &error,
            WeatherLibraryError::Lighting(ExteriorWeatherError::ImageSpaceMalformed { image_space, .. })
                if image_space == IMAGE_SPACE
        ),
        "{error:?}"
    );
}

#[test]
fn a_precipitation_pointing_at_no_geometry_row_is_an_error() {
    let conn = seeded(&two_weathers());
    conn.execute("DELETE FROM shader_particle_geometries", [])
        .unwrap();
    let error = load(conn).unwrap_err();
    assert!(
        matches!(&error, WeatherLibraryError::ParticleGeometryNotFound { weather, .. } if weather == RAIN),
        "{error:?}"
    );
}

#[test]
fn a_climate_entry_naming_an_unknown_weather_is_reported_by_key() {
    let weathers = json!([{"Weather": "0000FF:Fallout4.esm", "Chance": 1, "Global": null}]);
    let error = load(seeded(&weathers)).unwrap_err();
    assert!(
        matches!(
            &error,
            WeatherLibraryError::Lighting(ExteriorWeatherError::WeatherNotFound { weather })
                if weather == "0000FF:Fallout4.esm"
        ),
        "{error:?}"
    );
}

#[test]
fn the_library_hands_the_weather_cycle_its_roll_table() {
    let library = load(seeded(&two_weathers())).unwrap();
    let table = library.roll_table().unwrap();
    assert_eq!(table.climate_volatility(), 45);
    assert_eq!(table.total_chance(), 100);
    assert_eq!(table.entries().len(), 2);
    assert_eq!(library.get(table.initial()).unwrap().form_key, CLEAR);
    let rain = table.traits(WeatherId::new(1)).unwrap();
    assert_eq!(rain.trans_delta_raw, 0);
    assert!((rain.volatility_mult - 1.0).abs() < 1e-6);
    assert!(!rain.has_thunder_sound);
}

#[test]
fn a_weather_with_a_thunder_sound_is_marked_in_the_roll_table() {
    let conn = seeded(&two_weathers());
    let sounds = json!([{"kind": "thunder", "sound": "0000B2:Fallout4.esm"}]).to_string();
    conn.execute(
        "UPDATE weather SET sounds = ? WHERE form_key = ?",
        rusqlite::params![sounds, RAIN],
    )
    .unwrap();
    let library = load(conn).unwrap();
    let table = library.roll_table().unwrap();
    assert!(table.traits(WeatherId::new(1)).unwrap().has_thunder_sound);
    assert!(!table.traits(WeatherId::new(0)).unwrap().has_thunder_sound);
}

#[test]
fn the_world_starts_its_weather_cycle_on_the_weathers_of_the_library() {
    let startup = start(seeded(&two_weathers())).unwrap();
    assert_eq!(startup.table.initial(), startup.library.initial());
    assert_eq!(
        startup
            .library
            .get(startup.table.initial())
            .unwrap()
            .form_key,
        CLEAR
    );
    assert_eq!(startup.table.total_chance(), 100);
}

#[test]
fn a_database_without_the_library_columns_starts_no_cycle_and_names_the_fix() {
    let conn = connection_without(&[("weather", "clouds"), ("weather", "data")]);
    let error = start(conn).unwrap_err();
    assert!(
        matches!(
            error,
            StartupWeatherError::Library(WeatherLibraryError::SchemaOutdated)
        ),
        "{error:?}"
    );
    assert!(
        error
            .to_string()
            .contains("esm-only --tables weather,climates,shader_particle_geometries")
    );
}

#[test]
fn a_climate_where_no_weather_has_a_chance_starts_no_cycle() {
    let weathers = json!([
        {"Weather": CLEAR, "Chance": 0, "Global": null},
        {"Weather": RAIN, "Chance": 0, "Global": null},
    ]);
    let error = start(seeded(&weathers)).unwrap_err();
    assert!(
        matches!(
            error,
            StartupWeatherError::RollTable(WeatherTableError::NoChance)
        ),
        "{error:?}"
    );
}

fn day_rays() -> Json {
    json!({
        "back_color": [197, 204, 218], "forward_color": [255, 154, 17], "air_color": [164, 172, 191],
        "back_color_scale": 5.0, "forward_color_scale": 1.0, "air_color_scale": 0.1,
        "intensity": 2.0, "back_phase": 0.552, "forward_phase": 0.801,
    })
}

/// The clear weather with `DAY_RAYS` in its Day slot and none in the others.
fn name_day_rays(conn: &Connection) {
    let slots: Json = SLOTS
        .iter()
        .map(|slot| {
            let rays = if *slot == "Day" {
                json!(DAY_RAYS)
            } else {
                Json::Null
            };
            ((*slot).to_owned(), rays)
        })
        .collect();
    conn.execute(
        "UPDATE weather SET god_rays = ? WHERE form_key = ?",
        [slots.to_string(), CLEAR.to_owned()],
    )
    .unwrap();
}

/// [`name_day_rays`] with the god rays record holding `data`.
fn with_day_rays(conn: &Connection, data: Option<&str>) {
    name_day_rays(conn);
    insert(
        conn,
        "god_rayses",
        &[
            ("form_key", text(DAY_RAYS)),
            ("data", data.map_or(Value::Null, text)),
        ],
    );
}

fn initial_in_scatter(conn: Connection) -> Result<SunInScatter, ExteriorWeatherError> {
    let db = DbClient::from_connection(conn).unwrap();
    ExteriorWeatherSlots::load(&db, WORLDSPACE).map(|slots| slots.initial().sun_in_scatter)
}

#[test]
fn the_day_slot_scatters_the_sun_by_the_god_rays_record_it_names() {
    let conn = seeded(&two_weathers());
    with_day_rays(&conn, Some(&day_rays().to_string()));
    let data: GodRaysData = serde_json::from_value(day_rays()).unwrap();
    assert_eq!(
        initial_in_scatter(conn).unwrap(),
        SunInScatter::from_god_rays(&data).unwrap()
    );
}

#[test]
fn a_weather_without_god_rays_scatters_nothing() {
    assert_eq!(
        initial_in_scatter(seeded(&two_weathers())).unwrap(),
        SunInScatter::NONE
    );
}

#[test]
fn an_unknown_god_rays_record_scatters_nothing_and_the_weather_still_loads() {
    let conn = seeded(&two_weathers());
    with_day_rays(&conn, Some(&day_rays().to_string()));
    conn.execute("DELETE FROM god_rayses", []).unwrap();
    assert_eq!(initial_in_scatter(conn).unwrap(), SunInScatter::NONE);
}

#[test]
fn a_god_rays_record_without_data_scatters_nothing() {
    let conn = seeded(&two_weathers());
    with_day_rays(&conn, None);
    assert_eq!(initial_in_scatter(conn).unwrap(), SunInScatter::NONE);
}

#[test]
fn a_database_from_before_the_god_rays_export_scatters_nothing() {
    let conn = seeded_into(
        connection_without(&[("god_rayses", "data")]),
        &two_weathers(),
    );
    name_day_rays(&conn);
    let db = DbClient::from_connection(conn).unwrap();
    assert!(!db.schema().weather_god_rays_columns_present);
    let slots = ExteriorWeatherSlots::load(&db, WORLDSPACE).unwrap();
    assert_eq!(slots.initial().sun_in_scatter, SunInScatter::NONE);
}

#[test]
fn malformed_god_rays_data_fails_the_weather_naming_the_record() {
    let conn = seeded(&two_weathers());
    with_day_rays(&conn, Some("{ not json"));
    let error = initial_in_scatter(conn).unwrap_err();
    assert!(
        matches!(&error, ExteriorWeatherError::GodRaysMalformed { god_rays, .. } if god_rays == DAY_RAYS),
        "{error:?}"
    );
}
