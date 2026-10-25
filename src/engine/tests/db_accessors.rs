//! `DbClient` accessors against the installer's real schema (`esm_schema.sql`,
//! the single source of truth) loaded into an in-memory `SQLite` database.
//! Rows are seeded with plain SQL; nothing here mocks the database.

#![allow(
    clippy::float_cmp,
    reason = "values are stored and read back bit-exactly; no arithmetic is involved"
)]

use openfo4::assets::db::DbClient;
use openfo4::assets::db::schema::DbSchema;
use openfo4::assets::db::teleports::{DoorLocation, WalkInDoors};
use openfo4::assets::db::water_placement::CellWaterHeight;
use rusqlite::Connection;
use rusqlite::types::Value;

const SCHEMA: &str = include_str!("../../installer/file_converters/esm_schema.sql");

fn text(s: &str) -> Value {
    Value::Text(s.to_string())
}

fn int(n: i64) -> Value {
    Value::Integer(n)
}

fn real(x: f64) -> Value {
    Value::Real(x)
}

fn blob(bytes: &[u8]) -> Value {
    Value::Blob(bytes.to_vec())
}

fn schema_connection() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(SCHEMA).unwrap();
    // The installer inserts parents first; these tests seed only the tables
    // under test, so referential integrity is not what is being exercised.
    conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
    conn
}

/// The real schema with columns dropped, as in a database converted by an
/// older installer.
fn legacy_connection(dropped: &[(&str, &str)]) -> Connection {
    let conn = schema_connection();
    for (table, column) in dropped {
        conn.execute_batch(&format!("ALTER TABLE {table} DROP COLUMN {column};"))
            .unwrap();
    }
    conn
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

fn client(conn: Connection) -> DbClient {
    DbClient::from_connection(conn).unwrap()
}

fn is_no_rows(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<rusqlite::Error>(),
        Some(rusqlite::Error::QueryReturnedNoRows)
    )
}

fn cell_row(
    form_key: &str,
    worldspace: Option<&str>,
    grid: Option<(i64, i64)>,
) -> Vec<(&'static str, Value)> {
    let mut row = vec![("form_key", text(form_key))];
    if let Some(ws) = worldspace {
        row.push(("worldspace", text(ws)));
    }
    if let Some((x, y)) = grid {
        row.push(("grid_x", int(x)));
        row.push(("grid_y", int(y)));
    }
    row
}

fn placed(form_key: &str, cell: &str, position: (f64, f64, f64)) -> Vec<(&'static str, Value)> {
    vec![
        ("form_key", text(form_key)),
        ("cell_form_key", text(cell)),
        ("base_object", text("base")),
        ("object_type", text("PlacedObject")),
        ("position_x", real(position.0)),
        ("position_y", real(position.1)),
        ("position_z", real(position.2)),
        ("rotation_x", real(0.0)),
        ("rotation_y", real(0.0)),
        ("rotation_z", real(0.0)),
    ]
}

// ---------------------------------------------------------------- schema

#[test]
fn every_accessor_query_runs_against_the_real_schema() {
    let db = client(schema_connection());
    assert!(db.get_statics().unwrap().is_empty());
    assert!(db.get_furniture().unwrap().is_empty());
    assert!(db.get_doors().unwrap().is_empty());
    assert!(db.get_containers().unwrap().is_empty());
    assert!(db.get_movable_statics().unwrap().is_empty());
    assert!(db.get_activators().unwrap().is_empty());
    assert!(db.get_flora().unwrap().is_empty());
    assert!(db.get_trees().unwrap().is_empty());
    assert!(db.get_lights().unwrap().is_empty());
    assert!(db.get_lighting_templates().unwrap().is_empty());
    assert!(db.get_static_collections().unwrap().is_empty());
    assert!(db.get_worldspaces().unwrap().is_empty());
    assert_eq!(db.get_perks().unwrap(), []);
    assert_eq!(db.get_perk_chart().unwrap(), []);
    assert_eq!(db.get_special_attributes().unwrap(), []);
    assert_eq!(db.get_items().unwrap(), []);
    assert_eq!(db.get_map_markers("ws").unwrap(), []);
    assert_eq!(db.get_worldspace_map_image("ws").unwrap(), None);
    assert_eq!(db.get_waters().unwrap(), []);
    assert_eq!(db.get_placed_water_refs("c").unwrap(), []);
    assert_eq!(db.get_cells_with_placed_water(5).unwrap(), []);
    assert!(db.get_exterior_cell_waters("w").unwrap().is_empty());
    assert_eq!(db.get_lod_water_quads("w").unwrap(), []);
    assert!(db.get_landscape_textures().unwrap().is_empty());
    assert!(db.get_base_material_swaps().unwrap().is_empty());
    assert!(db.get_material_swap_entries().unwrap().is_empty());
    assert!(db.get_weapons().unwrap().unwrap().is_empty());
    assert!(db.get_mod_models(&[]).unwrap().unwrap().is_empty());
    assert!(db.get_interior_cells().unwrap().is_empty());
    assert!(db.get_interior_lighting().unwrap().rows.is_empty());
    assert_eq!(db.get_interior_anchors().unwrap(), []);
    assert!(db.get_placed_teleports().unwrap().is_empty());
    assert!(db.get_cells("ws").unwrap().is_empty());
    assert!(
        db.get_cells_in_range("ws", -1, 1, -1, 1)
            .unwrap()
            .is_empty()
    );
    assert!(db.get_placed_objects("c").unwrap().is_empty());
    assert!(
        db.get_worldspace_persistent_objects("ws")
            .unwrap()
            .is_empty()
    );
    assert_eq!(db.count_placed_teleports().unwrap(), 0);
}

#[test]
fn detect_reports_every_optional_group_present_on_the_real_schema() {
    let schema = DbSchema::detect(&schema_connection()).unwrap();
    assert!(schema.light_columns_present);
    assert!(schema.placed_object_light_columns_present);
    assert!(schema.placed_object_ragdoll_pose_column_present);
    assert!(schema.material_swap_name_column_present);
    assert!(schema.weather_columns_present);
    assert!(schema.weather_fog_column_present);
    assert!(schema.cell_water_columns_present);
    assert!(schema.worldspace_water_columns_present);
    assert!(schema.activator_water_column_present);
    assert!(schema.material_type_columns_present);
    assert!(schema.perk_pipboy_columns_present);
    assert!(schema.perk_chart_columns_present);
    assert!(schema.special_attribute_columns_present);
    assert!(schema.map_marker_columns_present);
    assert!(schema.worldspace_map_image_column_present);
    assert!(schema.weapon_assembly_available());
}

#[test]
fn detect_reports_a_fully_missing_group_as_absent() {
    let conn = legacy_connection(&[("placed_objects", "ragdoll_pose")]);
    let schema = DbSchema::detect(&conn).unwrap();
    assert!(!schema.placed_object_ragdoll_pose_column_present);
    assert!(schema.placed_object_light_columns_present);
}

#[test]
fn material_buoyancies_are_read_by_hash_skipping_unnamed_materials() {
    let conn = schema_connection();
    insert(
        &conn,
        "material_types",
        &[
            ("form_key", text("A")),
            ("material_hash", int(2_052_480_620)),
            ("buoyancy", real(-1.0)),
        ],
    );
    insert(
        &conn,
        "material_types",
        &[("form_key", text("B")), ("buoyancy", real(1.0))],
    );
    let rows = client(conn).get_material_buoyancies().unwrap().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].material_hash, rows[0].buoyancy),
        (2_052_480_620, -1.0)
    );
}

#[test]
fn material_buoyancies_are_absent_on_a_database_from_an_older_installer() {
    let conn = legacy_connection(&[
        ("material_types", "material_hash"),
        ("material_types", "buoyancy"),
    ]);
    assert!(client(conn).get_material_buoyancies().unwrap().is_none());
}

fn seed_weapon(conn: &Connection, form_key: &str, model: Option<&str>, default_mods: Option<&str>) {
    insert(
        conn,
        "items",
        &[
            ("form_key", text(form_key)),
            ("item_type", text("WEAP")),
            ("editor_id", text(&format!("edid_{form_key}"))),
            ("model_path", model.map_or(Value::Null, text)),
        ],
    );
    insert(
        conn,
        "weapons",
        &[
            ("form_key", text(form_key)),
            ("default_mods", default_mods.map_or(Value::Null, text)),
        ],
    );
}

#[test]
fn weapons_are_read_with_their_default_mod_lists() {
    let conn = schema_connection();
    seed_weapon(
        &conn,
        "004822:Fallout4.esm",
        Some(r"weapons\10mmpistol\10mmrecieverdummy.nif"),
        Some(r#"["091FC4:Fallout4.esm","0231AB:Fallout4.esm"]"#),
    );
    seed_weapon(&conn, "000001:Fallout4.esm", Some("a.nif"), Some("[]"));
    seed_weapon(&conn, "000002:Fallout4.esm", None, None);
    let mut weapons = client(conn).get_weapons().unwrap().unwrap();
    weapons.sort_by(|a, b| a.form_key.cmp(&b.form_key));
    assert_eq!(weapons.len(), 3);
    assert_eq!(weapons[0].default_mods.as_deref(), Some(&[][..]));
    assert_eq!(weapons[1].model_path, None);
    assert_eq!(weapons[1].default_mods, None);
    let ten_mm = &weapons[2];
    assert_eq!(
        ten_mm.editor_id.as_deref(),
        Some("edid_004822:Fallout4.esm")
    );
    assert_eq!(
        ten_mm.model_path.as_deref(),
        Some(r"weapons\10mmpistol\10mmrecieverdummy.nif")
    );
    assert_eq!(
        ten_mm.default_mods.as_deref().unwrap(),
        ["091FC4:Fallout4.esm", "0231AB:Fallout4.esm"]
    );
}

#[test]
fn malformed_default_mods_name_the_weapon() {
    let conn = schema_connection();
    seed_weapon(
        &conn,
        "004822:Fallout4.esm",
        Some("a.nif"),
        Some("not json"),
    );
    let error = client(conn).get_weapons().err().unwrap();
    assert!(
        format!("{error:#}").contains("004822:Fallout4.esm"),
        "{error:#}"
    );
}

#[test]
fn mod_models_are_read_for_the_requested_keys_only() {
    let conn = schema_connection();
    insert(
        &conn,
        "object_modifications",
        &[
            ("form_key", text("0231AB:Fallout4.esm")),
            ("model_path", text(r"weapons\10mmpistol\10mmgrip.nif")),
            ("attach_point_name", text("ap_gun_Grip")),
        ],
    );
    insert(
        &conn,
        "object_modifications",
        &[
            ("form_key", text("04F21D:Fallout4.esm")),
            ("attach_point_name", text("ap_gun_Muzzle")),
        ],
    );
    insert(
        &conn,
        "object_modifications",
        &[("form_key", text("000009:Fallout4.esm"))],
    );
    let keys = ["0231AB:Fallout4.esm", "04F21D:Fallout4.esm", "missing"].map(String::from);
    let mut rows = client(conn).get_mod_models(&keys).unwrap().unwrap();
    rows.sort_by(|a, b| a.form_key.cmp(&b.form_key));
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].form_key, "0231AB:Fallout4.esm");
    assert_eq!(
        rows[0].row.model_path.as_deref(),
        Some(r"weapons\10mmpistol\10mmgrip.nif")
    );
    assert_eq!(rows[1].form_key, "04F21D:Fallout4.esm");
    assert_eq!(rows[1].row.model_path, None);
    assert_eq!(
        rows[1].row.attach_point_name.as_deref(),
        Some("ap_gun_Muzzle")
    );
}

#[test]
fn weapon_data_is_unavailable_on_a_database_from_an_older_installer() {
    let conn = legacy_connection(&[("weapons", "default_mods")]);
    let db = client(conn);
    assert!(db.get_weapons().unwrap().is_none());
    assert!(db.get_mod_models(&[]).unwrap().is_none());
    assert!(!db.schema().weapon_assembly_available());
}

#[test]
fn detect_rejects_a_partial_object_mod_model_group() {
    let conn = legacy_connection(&[("object_modifications", "attach_point_name")]);
    let error = DbSchema::detect(&conn).unwrap_err().to_string();
    assert!(error.contains("attach_point_name"), "{error}");
}

#[test]
fn detect_requires_both_weather_tables_for_weather_columns() {
    let conn = legacy_connection(&[("worldspaces", "climate")]);
    let schema = DbSchema::detect(&conn).unwrap();
    assert!(!schema.weather_columns_present);
}

#[test]
fn detect_rejects_a_partially_present_column_group_naming_the_missing_columns() {
    let conn = legacy_connection(&[("lights", "gobo"), ("lights", "lens")]);
    let error = DbSchema::detect(&conn).unwrap_err().to_string();
    assert!(error.contains("lights"), "{error}");
    assert!(error.contains("gobo") && error.contains("lens"), "{error}");
}

#[test]
fn from_connection_clients_cannot_be_cloned() {
    let db = client(schema_connection());
    let error = db.try_clone().err().expect("in-memory client cannot clone");
    assert!(error.to_string().contains("cannot clone"));
}

#[test]
fn file_backed_clients_clone_to_a_second_working_connection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(SCHEMA).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
    insert(
        &conn,
        "statics",
        &[("form_key", text("S1")), ("editor_id", text("Rock"))],
    );
    drop(conn);

    let db = DbClient::new(path.to_str().unwrap()).unwrap();
    let clone = db.try_clone().unwrap();
    assert_eq!(
        clone.get_static("S1").unwrap().editor_id.as_deref(),
        Some("Rock")
    );
}

#[test]
fn opening_a_missing_database_file_fails() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.db");
    assert!(DbClient::new(missing.to_str().unwrap()).is_err());
}

// ----------------------------------------------------------------- cells

#[test]
fn get_cell_maps_every_column_including_nulls() {
    let conn = schema_connection();
    insert(
        &conn,
        "cells",
        &[
            ("form_key", text("C1")),
            ("editor_id", text("Cell1")),
            ("worldspace", text("WS")),
            ("grid_x", int(-3)),
            ("grid_y", int(7)),
            ("is_interior", int(0)),
            ("flags", text("{}")),
        ],
    );
    let cell = client(conn).get_cell(-3, 7, "WS").unwrap();
    assert_eq!(cell.form_key.as_deref(), Some("C1"));
    assert_eq!(cell.editor_id.as_deref(), Some("Cell1"));
    assert_eq!((cell.grid_x, cell.grid_y), (Some(-3), Some(7)));
    assert!(!cell.is_interior);
    assert_eq!(cell.flags.as_deref(), Some("{}"));
    assert_eq!(cell.lighting, None);
    assert_eq!(cell.encounter_zone, None);
}

#[test]
fn get_cell_for_an_empty_grid_square_is_a_no_rows_error() {
    let db = client(schema_connection());
    let error = db.get_cell(0, 0, "WS").err().unwrap();
    assert!(is_no_rows(&error));
}

#[test]
fn grid_queries_exclude_the_worldspace_persistent_cell_sharing_grid_zero_zero() {
    let conn = schema_connection();
    let mut persistent = cell_row("TOP", Some("WS"), Some((0, 0)));
    persistent.push(("is_worldspace_persistent_cell", int(1)));
    insert(&conn, "cells", &persistent);
    insert(&conn, "cells", &cell_row("REAL", Some("WS"), Some((0, 0))));
    let db = client(conn);
    assert_eq!(
        db.get_cell(0, 0, "WS").unwrap().form_key.as_deref(),
        Some("REAL")
    );
    let all = db.get_cells("WS").unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(db.get_cells_in_range("WS", -1, 1, -1, 1).unwrap().len(), 1);
}

#[test]
fn cells_in_range_includes_both_boundaries_and_filters_worldspace() {
    let conn = schema_connection();
    for (key, ws, x, y) in [
        ("in_low", "WS", -2, -2),
        ("in_high", "WS", 2, 2),
        ("out_x", "WS", 3, 0),
        ("out_y", "WS", 0, -3),
        ("other_ws", "OTHER", 0, 0),
    ] {
        insert(&conn, "cells", &cell_row(key, Some(ws), Some((x, y))));
    }
    let mut keys: Vec<String> = client(conn)
        .get_cells_in_range("WS", -2, 2, -2, 2)
        .unwrap()
        .into_iter()
        .filter_map(|c| c.form_key)
        .collect();
    keys.sort();
    assert_eq!(keys, ["in_high", "in_low"]);
}

#[test]
fn cell_by_form_key_reaches_interiors_without_grid_or_worldspace() {
    let conn = schema_connection();
    insert(
        &conn,
        "cells",
        &[("form_key", text("INT")), ("is_interior", int(1))],
    );
    let cell = client(conn).get_cell_by_form_key("INT").unwrap();
    assert!(cell.is_interior);
    assert_eq!((cell.worldspace, cell.grid_x), (None, None));
}

#[test]
fn cell_by_form_key_for_unknown_key_is_a_no_rows_error() {
    let error = client(schema_connection())
        .get_cell_by_form_key("nope")
        .err()
        .unwrap();
    assert!(is_no_rows(&error));
}

#[test]
fn cell_by_editor_id_is_case_insensitive_and_none_when_absent() {
    let conn = schema_connection();
    insert(
        &conn,
        "cells",
        &[
            ("form_key", text("C")),
            ("editor_id", text("SanctuaryHills")),
        ],
    );
    let db = client(conn);
    assert!(
        db.get_cell_by_editor_id("sanctuaryhills")
            .unwrap()
            .is_some()
    );
    assert!(db.get_cell_by_editor_id("Diamond").unwrap().is_none());
}

#[test]
fn interior_cells_are_listed_by_editor_id_ignoring_case_with_unnamed_first() {
    let conn = schema_connection();
    insert(
        &conn,
        "cells",
        &[
            ("form_key", text("b")),
            ("editor_id", text("bravo")),
            ("is_interior", int(1)),
        ],
    );
    insert(
        &conn,
        "cells",
        &[
            ("form_key", text("a")),
            ("editor_id", text("Alpha")),
            ("is_interior", int(1)),
        ],
    );
    insert(
        &conn,
        "cells",
        &[("form_key", text("n")), ("is_interior", int(1))],
    );
    insert(
        &conn,
        "cells",
        &[
            ("form_key", text("ext")),
            ("editor_id", text("aaa")),
            ("is_interior", int(0)),
        ],
    );
    let keys: Vec<_> = client(conn)
        .get_interior_cells()
        .unwrap()
        .into_iter()
        .filter_map(|c| c.form_key)
        .collect();
    assert_eq!(keys, ["n", "a", "b"]);
}

#[test]
fn interior_lighting_reads_the_template_link_when_the_column_exists() {
    let conn = schema_connection();
    insert(
        &conn,
        "cells",
        &[
            ("form_key", text("I")),
            ("is_interior", int(1)),
            ("lighting", text("{\"x\":1}")),
            ("lighting_template", text("T:Fallout4.esm")),
        ],
    );
    let rows = client(conn).get_interior_lighting().unwrap();
    assert!(rows.template_column_present);
    assert_eq!(
        rows.rows[0].lighting_template.as_deref(),
        Some("T:Fallout4.esm")
    );
    assert_eq!(rows.rows[0].lighting.as_deref(), Some("{\"x\":1}"));
}

#[test]
fn interior_lighting_without_the_template_column_flags_it_and_yields_none() {
    let conn = legacy_connection(&[("cells", "lighting_template")]);
    insert(
        &conn,
        "cells",
        &[("form_key", text("I")), ("is_interior", int(1))],
    );
    let rows = client(conn).get_interior_lighting().unwrap();
    assert!(!rows.template_column_present);
    assert_eq!(rows.rows.len(), 1);
    assert_eq!(rows.rows[0].lighting_template, None);
}

#[test]
fn populated_cell_is_the_grid_cell_with_the_most_placed_objects() {
    let conn = schema_connection();
    insert(
        &conn,
        "cells",
        &cell_row("EMPTYISH", Some("WS"), Some((0, 0))),
    );
    insert(&conn, "cells", &cell_row("BUSY", Some("WS"), Some((1, 0))));
    insert(
        &conn,
        "placed_objects",
        &placed("r1", "EMPTYISH", (0.0, 0.0, 0.0)),
    );
    for n in 0..3 {
        insert(
            &conn,
            "placed_objects",
            &placed(&format!("b{n}"), "BUSY", (0.0, 0.0, 0.0)),
        );
    }
    let cell = client(conn).get_populated_cell("WS").unwrap();
    assert_eq!(cell.form_key.as_deref(), Some("BUSY"));
}

// ------------------------------------------------------------- landscapes

fn landscape_row(
    form_key: &str,
    cell: &str,
    heights: &[u8],
    colors: Option<&[u8]>,
    layers: Option<&[u8]>,
) -> Vec<(&'static str, Value)> {
    let mut row = vec![
        ("form_key", text(form_key)),
        ("cell", text(cell)),
        ("vertex_height_map", blob(heights)),
    ];
    if let Some(c) = colors {
        row.push(("vertex_colors", blob(c)));
    }
    if let Some(l) = layers {
        row.push(("layers", blob(l)));
    }
    row
}

#[test]
fn landscape_blobs_round_trip_byte_for_byte_and_optional_blobs_are_none() {
    let conn = schema_connection();
    let heights: Vec<u8> = (0..=255).collect();
    insert(
        &conn,
        "landscapes",
        &landscape_row("L1", "C1", &heights, None, None),
    );
    insert(
        &conn,
        "landscapes",
        &landscape_row("L2", "C2", &[1, 0, 2], Some(&[9, 8, 7]), Some(&[0, 0])),
    );
    let db = client(conn);

    let plain = db.get_landscape("C1", 4, -5).unwrap();
    assert_eq!(plain.vertex_height_map, heights);
    assert_eq!((plain.vertex_colors, plain.layers), (None, None));
    assert_eq!((plain.grid_x, plain.grid_y), (4, -5));

    let full = db.get_landscape("C2", 0, 0).unwrap();
    assert_eq!(full.vertex_height_map, [1, 0, 2]);
    assert_eq!(full.vertex_colors.as_deref(), Some(&[9, 8, 7][..]));
    assert_eq!(full.layers.as_deref(), Some(&[0, 0][..]));
}

#[test]
fn landscape_for_a_cell_without_one_is_a_no_rows_error() {
    let error = client(schema_connection())
        .get_landscape("none", 0, 0)
        .err()
        .unwrap();
    assert!(is_no_rows(&error));
}

#[test]
fn batched_landscapes_take_their_grid_from_the_requesting_cells() {
    let conn = schema_connection();
    insert(
        &conn,
        "landscapes",
        &landscape_row("L1", "C1", &[1], None, None),
    );
    insert(
        &conn,
        "landscapes",
        &landscape_row("L2", "C2", &[2], None, None),
    );
    insert(&conn, "cells", &cell_row("C1", Some("WS"), Some((3, 4))));
    insert(&conn, "cells", &cell_row("C2", Some("WS"), Some((-1, -2))));
    let db = client(conn);
    let cells = db.get_cells("WS").unwrap();
    let mut landscapes = db.get_landscapes(&cells).unwrap();
    landscapes.sort_by(|a, b| a.cell.cmp(&b.cell));
    assert_eq!((landscapes[0].grid_x, landscapes[0].grid_y), (3, 4));
    assert_eq!((landscapes[1].grid_x, landscapes[1].grid_y), (-1, -2));
}

#[test]
fn batched_landscapes_skip_cells_without_a_form_key_or_grid_and_empty_input_is_empty() {
    let conn = schema_connection();
    insert(
        &conn,
        "landscapes",
        &landscape_row("L1", "INT", &[1], None, None),
    );
    insert(
        &conn,
        "cells",
        &[("form_key", text("INT")), ("is_interior", int(1))],
    );
    let db = client(conn);
    assert!(db.get_landscapes(&[]).unwrap().is_empty());
    let interior = vec![db.get_cell_by_form_key("INT").unwrap()];
    assert!(db.get_landscapes(&interior).unwrap().is_empty());
}

// ------------------------------------------------ worldspace LAND lineage

const LAND_DATA: i64 = 0x01;
const LOD_DATA: i64 = 0x02;

fn worldspace_row(
    form_key: &str,
    parent: Option<&str>,
    parent_flags: i64,
) -> Vec<(&'static str, Value)> {
    let mut row = vec![
        ("form_key", text(form_key)),
        ("editor_id", text(&format!("{form_key}Edid"))),
        ("parent_flags", int(parent_flags)),
    ];
    if let Some(parent) = parent {
        row.push(("parent_worldspace", text(parent)));
    }
    row
}

fn seed_worldspaces(conn: &Connection, rows: &[(&str, Option<&str>, i64)]) {
    for (form_key, parent, flags) in rows {
        insert(
            conn,
            "worldspaces",
            &worldspace_row(form_key, *parent, *flags),
        );
    }
}

/// A grid cell of `worldspace` at (1, 2) with a one-byte LAND whose height byte is `marker`.
fn seed_land(conn: &Connection, worldspace: &str, marker: u8) {
    let cell = format!("{worldspace}-cell");
    insert(
        conn,
        "cells",
        &cell_row(&cell, Some(worldspace), Some((1, 2))),
    );
    insert(
        conn,
        "landscapes",
        &landscape_row(&format!("{worldspace}-land"), &cell, &[marker], None, None),
    );
}

fn seed_cell_without_land(conn: &Connection, worldspace: &str) {
    insert(
        conn,
        "cells",
        &cell_row(
            &format!("{worldspace}-cell"),
            Some(worldspace),
            Some((1, 2)),
        ),
    );
}

fn marker_at(db: &DbClient, worldspace: &str) -> Option<u8> {
    db.get_landscape_at(worldspace, 1, 2)
        .unwrap()
        .map(|land| land.vertex_height_map[0])
}

#[test]
fn a_worldspace_with_its_own_land_reads_it_with_the_requested_grid() {
    let conn = schema_connection();
    seed_worldspaces(&conn, &[("WS", None, 0)]);
    seed_land(&conn, "WS", 7);
    let land = client(conn).get_landscape_at("WS", 1, 2).unwrap().unwrap();
    assert_eq!(land.vertex_height_map, [7]);
    assert_eq!((land.grid_x, land.grid_y), (1, 2));
    assert_eq!(land.cell, "WS-cell");
}

#[test]
fn a_cell_without_land_inherits_it_from_the_parent_when_the_land_bit_is_set() {
    let conn = schema_connection();
    seed_worldspaces(
        &conn,
        &[("CHILD", Some("PARENT"), LAND_DATA), ("PARENT", None, 0)],
    );
    seed_cell_without_land(&conn, "CHILD");
    seed_land(&conn, "PARENT", 9);
    let db = client(conn);
    let land = db.get_landscape_at("CHILD", 1, 2).unwrap().unwrap();
    assert_eq!(land.vertex_height_map, [9]);
    assert_eq!((land.grid_x, land.grid_y), (1, 2));
}

#[test]
fn a_child_without_any_cell_at_the_grid_inherits_the_parents_land() {
    let conn = schema_connection();
    seed_worldspaces(
        &conn,
        &[("CHILD", Some("PARENT"), LAND_DATA), ("PARENT", None, 0)],
    );
    seed_land(&conn, "PARENT", 9);
    assert_eq!(marker_at(&client(conn), "CHILD"), Some(9));
}

#[test]
fn the_childs_own_land_wins_over_the_parents() {
    let conn = schema_connection();
    seed_worldspaces(
        &conn,
        &[("CHILD", Some("PARENT"), LAND_DATA), ("PARENT", None, 0)],
    );
    seed_land(&conn, "CHILD", 1);
    seed_land(&conn, "PARENT", 9);
    assert_eq!(marker_at(&client(conn), "CHILD"), Some(1));
}

#[test]
fn a_cleared_land_bit_blocks_inheritance() {
    let conn = schema_connection();
    seed_worldspaces(
        &conn,
        &[
            ("CHILD", Some("PARENT"), 0xFF & !LAND_DATA),
            ("PARENT", None, 0),
        ],
    );
    seed_cell_without_land(&conn, "CHILD");
    seed_land(&conn, "PARENT", 9);
    assert_eq!(marker_at(&client(conn), "CHILD"), None);
}

#[test]
fn land_is_inherited_through_a_grandparent_while_every_link_defers() {
    let conn = schema_connection();
    seed_worldspaces(
        &conn,
        &[
            ("CHILD", Some("PARENT"), LAND_DATA),
            ("PARENT", Some("GRAND"), LAND_DATA),
            ("GRAND", None, 0),
        ],
    );
    seed_land(&conn, "GRAND", 5);
    assert_eq!(marker_at(&client(conn), "CHILD"), Some(5));
}

#[test]
fn the_nearest_ancestor_with_land_wins() {
    let conn = schema_connection();
    seed_worldspaces(
        &conn,
        &[
            ("CHILD", Some("PARENT"), LAND_DATA),
            ("PARENT", Some("GRAND"), LAND_DATA),
            ("GRAND", None, 0),
        ],
    );
    seed_land(&conn, "PARENT", 6);
    seed_land(&conn, "GRAND", 5);
    assert_eq!(marker_at(&client(conn), "CHILD"), Some(6));
}

#[test]
fn a_link_that_keeps_its_land_cuts_the_chain_to_the_grandparent() {
    let conn = schema_connection();
    seed_worldspaces(
        &conn,
        &[
            ("CHILD", Some("PARENT"), LAND_DATA),
            ("PARENT", Some("GRAND"), 0),
            ("GRAND", None, 0),
        ],
    );
    seed_land(&conn, "GRAND", 5);
    assert_eq!(marker_at(&client(conn), "CHILD"), None);
}

#[test]
fn the_worldspace_persistent_cell_never_provides_land() {
    let conn = schema_connection();
    seed_worldspaces(&conn, &[("WS", None, 0)]);
    let mut persistent = cell_row("WS-top", Some("WS"), Some((1, 2)));
    persistent.push(("is_worldspace_persistent_cell", int(1)));
    insert(&conn, "cells", &persistent);
    insert(
        &conn,
        "landscapes",
        &landscape_row("TOP-land", "WS-top", &[3], None, None),
    );
    assert_eq!(marker_at(&client(conn), "WS"), None);
}

#[test]
fn a_database_without_the_pnam_column_inherits_nothing() {
    let conn = legacy_connection(&[
        ("worldspaces", "water_type"),
        ("worldspaces", "lod_water_type"),
        ("worldspaces", "lod_water_height"),
        ("worldspaces", "default_land_height"),
        ("worldspaces", "default_water_height"),
        ("worldspaces", "water_environment_map"),
        ("worldspaces", "parent_flags"),
    ]);
    insert(
        &conn,
        "worldspaces",
        &[
            ("form_key", text("CHILD")),
            ("parent_worldspace", text("PARENT")),
        ],
    );
    insert(&conn, "worldspaces", &[("form_key", text("PARENT"))]);
    seed_land(&conn, "PARENT", 9);
    let db = client(conn);
    assert_eq!(marker_at(&db, "CHILD"), None);
    assert_eq!(db.lod_data_owner("CHILD").unwrap(), "CHILD");
}

#[test]
fn a_parent_loop_is_an_error_not_a_missing_land() {
    let conn = schema_connection();
    seed_worldspaces(
        &conn,
        &[("A", Some("B"), LAND_DATA), ("B", Some("A"), LAND_DATA)],
    );
    let error = client(conn).get_landscape_at("A", 1, 2).err().unwrap();
    assert!(error.to_string().contains("loops at"), "{error}");
}

#[test]
fn an_unknown_worldspace_is_an_error_not_a_missing_land() {
    let error = client(schema_connection())
        .get_landscape_at("NOPE", 1, 2)
        .err()
        .unwrap();
    assert!(is_no_rows(&error));
}

#[test]
fn a_missing_parent_row_ends_the_chain_without_land() {
    let conn = schema_connection();
    seed_worldspaces(&conn, &[("CHILD", Some("GONE"), LAND_DATA)]);
    assert_eq!(marker_at(&client(conn), "CHILD"), None);
}

#[test]
fn a_worldspace_that_keeps_its_lod_data_owns_it() {
    let conn = schema_connection();
    seed_worldspaces(
        &conn,
        &[
            ("CHILD", Some("PARENT"), 0xFF & !LOD_DATA),
            ("PARENT", None, 0),
        ],
    );
    assert_eq!(client(conn).lod_data_owner("CHILD").unwrap(), "CHILD");
}

#[test]
fn lod_data_is_owned_by_the_nearest_ancestor_that_keeps_its_own() {
    let conn = schema_connection();
    seed_worldspaces(
        &conn,
        &[
            ("CHILD", Some("PARENT"), LOD_DATA),
            ("PARENT", Some("GRAND"), LOD_DATA),
            ("GRAND", None, 0),
        ],
    );
    let db = client(conn);
    assert_eq!(db.lod_data_owner("CHILD").unwrap(), "GRAND");
    assert_eq!(db.lod_data_owner("PARENT").unwrap(), "GRAND");
}

#[test]
fn lod_data_ignores_the_land_bit() {
    let conn = schema_connection();
    seed_worldspaces(
        &conn,
        &[("CHILD", Some("PARENT"), LAND_DATA), ("PARENT", None, 0)],
    );
    assert_eq!(client(conn).lod_data_owner("CHILD").unwrap(), "CHILD");
}

// --------------------------------------------------------- placed objects

#[test]
fn placed_object_maps_all_columns_and_decodes_the_ragdoll_pose_blob() {
    let conn = schema_connection();
    let pose: Vec<u8> = (0..56).collect();
    let mut row = placed("R1", "C1", (1.5, -2.5, 3.5));
    row.retain(|(column, _)| !column.starts_with("rotation"));
    row.extend([
        ("is_persistent", int(1)),
        ("rotation_x", real(0.1)),
        ("rotation_y", real(0.2)),
        ("rotation_z", real(0.3)),
        ("scale", real(2.0)),
        ("is_initially_disabled", int(1)),
        ("enable_parent", text("P")),
        ("enable_parent_opposite", int(1)),
        ("initially_hidden", int(1)),
        ("radius_override", real(-12.0)),
        ("light_fov_pm", real(5.0)),
        ("record_flags", int(0x200)),
        ("ragdoll_pose", blob(&pose)),
        ("material_swap", text("MSWP")),
    ]);
    insert(&conn, "placed_objects", &row);
    let object = client(conn).get_placed_objects("C1").unwrap().remove(0);
    assert_eq!(object.form_key.as_deref(), Some("R1"));
    assert_eq!(object.object_type.as_deref(), Some("PlacedObject"));
    assert_eq!(
        (object.position_x, object.position_y, object.position_z),
        (1.5, -2.5, 3.5)
    );
    assert_eq!(
        (object.rotation_x, object.rotation_y, object.rotation_z),
        (0.1, 0.2, 0.3)
    );
    assert_eq!(object.scale, 2.0);
    assert!(object.is_persistent && object.is_initially_disabled);
    assert!(object.enable_parent_opposite && object.initially_hidden);
    assert_eq!(object.enable_parent.as_deref(), Some("P"));
    assert_eq!(object.radius_delta, Some(-12.0));
    assert_eq!(object.light_fov_pm, Some(5.0));
    assert_eq!(object.light_fade_pm, None);
    assert_eq!(object.record_flags, Some(0x200));
    assert_eq!(object.ragdoll_pose.as_deref(), Some(&pose[..]));
    assert_eq!(object.material_swap.as_deref(), Some("MSWP"));
}

#[test]
fn placed_object_defaults_apply_when_optional_columns_are_unset() {
    let conn = schema_connection();
    insert(
        &conn,
        "placed_objects",
        &placed("R1", "C1", (0.0, 0.0, 0.0)),
    );
    let object = client(conn).get_placed_objects("C1").unwrap().remove(0);
    assert_eq!(object.scale, 1.0);
    assert!(!object.is_persistent && !object.initially_hidden && !object.has_primitive);
    assert_eq!(object.ragdoll_pose, None);
    assert_eq!(object.radius_delta, None);
}

#[test]
fn placed_objects_from_a_database_without_light_and_ragdoll_columns_read_them_as_none() {
    let conn = legacy_connection(&[
        ("placed_objects", "radius_override"),
        ("placed_objects", "light_fov_pm"),
        ("placed_objects", "light_fade_pm"),
        ("placed_objects", "light_end_distance_cap"),
        ("placed_objects", "light_shadow_depth_bias"),
        ("placed_objects", "light_near_clip"),
        ("placed_objects", "light_volumetric_intensity"),
        ("placed_objects", "record_flags"),
        ("placed_objects", "ragdoll_pose"),
    ]);
    insert(
        &conn,
        "placed_objects",
        &placed("R1", "C1", (7.0, 8.0, 9.0)),
    );
    let object = client(conn).get_placed_objects("C1").unwrap().remove(0);
    assert_eq!(object.position_z, 9.0);
    assert_eq!(object.radius_delta, None);
    assert_eq!(object.record_flags, None);
    assert_eq!(object.ragdoll_pose, None);
}

#[test]
fn batched_placed_objects_return_only_the_requested_cells_and_skip_keyless_cells() {
    let conn = schema_connection();
    insert(&conn, "cells", &cell_row("A", Some("WS"), Some((0, 0))));
    insert(&conn, "cells", &cell_row("B", Some("WS"), Some((1, 0))));
    insert(&conn, "placed_objects", &placed("ra", "A", (0.0, 0.0, 0.0)));
    insert(&conn, "placed_objects", &placed("rb", "B", (0.0, 0.0, 0.0)));
    let db = client(conn);
    let a = vec![db.get_cell_by_form_key("A").unwrap()];
    let objects = db.get_placed_objects_batched(&a).unwrap();
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].form_key.as_deref(), Some("ra"));
    assert!(db.get_placed_objects_batched(&[]).unwrap().is_empty());
}

#[test]
fn worldspace_persistent_objects_come_only_from_the_top_cell_and_only_when_persistent() {
    let conn = schema_connection();
    let mut top = cell_row("TOP", Some("WS"), Some((0, 0)));
    top.push(("is_worldspace_persistent_cell", int(1)));
    insert(&conn, "cells", &top);
    insert(&conn, "cells", &cell_row("REAL", Some("WS"), Some((0, 0))));
    let mut persistent = placed("keep", "TOP", (0.0, 0.0, 0.0));
    persistent.push(("is_persistent", int(1)));
    insert(&conn, "placed_objects", &persistent);
    insert(
        &conn,
        "placed_objects",
        &placed("not_persistent", "TOP", (0.0, 0.0, 0.0)),
    );
    let mut elsewhere = placed("grid_cell", "REAL", (0.0, 0.0, 0.0));
    elsewhere.push(("is_persistent", int(1)));
    insert(&conn, "placed_objects", &elsewhere);
    let objects = client(conn)
        .get_worldspace_persistent_objects("WS")
        .unwrap();
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].form_key.as_deref(), Some("keep"));
}

fn anchors(conn: Connection) -> std::collections::HashMap<String, (f64, f64, f64)> {
    client(conn)
        .get_interior_anchors()
        .unwrap()
        .into_iter()
        .map(|(cell, x, y, z)| (cell, (x, y, z)))
        .collect()
}

#[test]
fn interior_anchor_is_the_per_axis_median_so_outliers_do_not_drag_it() {
    let conn = schema_connection();
    insert(
        &conn,
        "cells",
        &[("form_key", text("I")), ("is_interior", int(1))],
    );
    for (n, p) in [
        (1.0, 10.0, 100.0),
        (2.0, 20.0, 200.0),
        (1000.0, 30.0, 300.0),
    ]
    .into_iter()
    .enumerate()
    {
        insert(&conn, "placed_objects", &placed(&format!("r{n}"), "I", p));
    }
    assert_eq!(anchors(conn)["I"], (2.0, 20.0, 200.0));
}

#[test]
fn interior_anchor_of_an_even_count_is_the_mean_of_the_two_middle_values() {
    let conn = schema_connection();
    insert(
        &conn,
        "cells",
        &[("form_key", text("I")), ("is_interior", int(1))],
    );
    for (n, x) in [0.0, 10.0, 20.0, 100.0].into_iter().enumerate() {
        insert(
            &conn,
            "placed_objects",
            &placed(&format!("r{n}"), "I", (x, 0.0, 0.0)),
        );
    }
    assert_eq!(anchors(conn)["I"].0, 15.0);
}

#[test]
fn interior_anchor_ignores_hidden_objects_incomplete_positions_and_exteriors() {
    let conn = schema_connection();
    insert(
        &conn,
        "cells",
        &[("form_key", text("I")), ("is_interior", int(1))],
    );
    insert(
        &conn,
        "cells",
        &[("form_key", text("EXT")), ("is_interior", int(0))],
    );
    insert(
        &conn,
        "cells",
        &[("form_key", text("EMPTY")), ("is_interior", int(1))],
    );
    insert(&conn, "placed_objects", &placed("ok", "I", (5.0, 5.0, 5.0)));
    let mut hidden = placed("hidden", "I", (999.0, 999.0, 999.0));
    hidden.push(("initially_hidden", int(1)));
    insert(&conn, "placed_objects", &hidden);
    insert(
        &conn,
        "placed_objects",
        &[
            ("form_key", text("no_z")),
            ("cell_form_key", text("I")),
            ("base_object", text("b")),
            ("object_type", text("t")),
            ("position_x", real(500.0)),
            ("position_y", real(500.0)),
        ],
    );
    insert(
        &conn,
        "placed_objects",
        &placed("outside", "EXT", (1.0, 1.0, 1.0)),
    );
    let anchors = anchors(conn);
    assert_eq!(anchors["I"], (5.0, 5.0, 5.0));
    assert!(!anchors.contains_key("EXT"));
    assert!(!anchors.contains_key("EMPTY"));
}

// -------------------------------------------------------------- teleports

fn teleport_world() -> Connection {
    let conn = schema_connection();
    insert(
        &conn,
        "cells",
        &[("form_key", text("INT")), ("is_interior", int(1))],
    );
    insert(&conn, "cells", &cell_row("EXT", Some("WS"), Some((0, 0))));
    insert(
        &conn,
        "placed_objects",
        &placed("dest_int", "INT", (0.0, 0.0, 0.0)),
    );
    insert(
        &conn,
        "placed_objects",
        &placed("dest_ext", "EXT", (0.0, 0.0, 0.0)),
    );
    conn
}

fn teleport(conn: &Connection, source: &str, destination: &str) {
    insert(
        conn,
        "placed_teleports",
        &[
            ("source_ref", text(source)),
            ("destination_ref", text(destination)),
            ("dest_position_x", real(1.0)),
            ("dest_position_y", real(2.0)),
            ("dest_position_z", real(3.0)),
            ("dest_rotation_x", real(0.0)),
            ("dest_rotation_y", real(0.0)),
            ("dest_rotation_z", real(1.5)),
        ],
    );
}

#[test]
fn teleports_resolve_the_destination_cell_and_classify_interior_versus_exterior() {
    let conn = teleport_world();
    teleport(&conn, "door_in", "dest_int");
    teleport(&conn, "door_out", "dest_ext");
    let db = client(conn);
    let mut all = db.get_placed_teleports().unwrap();
    all.sort_by(|a, b| a.source_ref.cmp(&b.source_ref));

    let outside = &all[0];
    assert_eq!(outside.source_ref, "door_in");
    assert!(outside.is_interior);
    assert_eq!(outside.worldspace, None);
    assert_eq!(outside.dest_cell, "INT");
    assert_eq!(
        (
            outside.dest_x,
            outside.dest_y,
            outside.dest_z,
            outside.dest_rot_z
        ),
        (1.0, 2.0, 3.0, 1.5)
    );

    assert!(!all[1].is_interior);
    assert_eq!(all[1].worldspace.as_deref(), Some("WS"));
}

#[test]
fn teleports_with_an_unresolvable_destination_are_dropped_but_still_counted() {
    let conn = teleport_world();
    teleport(&conn, "good", "dest_int");
    teleport(&conn, "dangling", "ghost_ref");
    let db = client(conn);
    assert_eq!(db.get_placed_teleports().unwrap().len(), 1);
    assert_eq!(db.count_placed_teleports().unwrap(), 2);
}

#[test]
fn teleport_into_a_cell_picks_the_lowest_source_ref() {
    let conn = teleport_world();
    teleport(&conn, "door_b", "dest_int");
    teleport(&conn, "door_a", "dest_int");
    let db = client(conn);
    assert_eq!(
        db.get_teleport_into_cell("INT")
            .unwrap()
            .unwrap()
            .source_ref,
        "door_a"
    );
    assert!(db.get_teleport_into_cell("EXT").unwrap().is_none());
}

// ---------------------------------------------------------- walk-in doors

const BOUNDS_JSON: &str = r#"{"Min":{"X":-64,"Y":-8,"Z":0},"Max":{"X":64,"Y":8,"Z":192}}"#;

/// The schema file gains `doors.trigger_bounds` with the installer; this keeps the tests valid
/// either side of that change.
fn with_trigger_bounds_column(conn: Connection, present: bool) -> Connection {
    let has_column: bool = conn
        .query_row(
            "SELECT count(*) > 0 FROM pragma_table_info('doors') WHERE name = 'trigger_bounds'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    match (has_column, present) {
        (false, true) => conn
            .execute_batch("ALTER TABLE doors ADD COLUMN trigger_bounds TEXT;")
            .unwrap(),
        (true, false) => conn
            .execute_batch("ALTER TABLE doors DROP COLUMN trigger_bounds;")
            .unwrap(),
        _ => {}
    }
    conn
}

fn walk_in_world() -> Connection {
    let conn = with_trigger_bounds_column(teleport_world(), true);
    insert(
        &conn,
        "doors",
        &[
            ("form_key", text("AUTO")),
            ("flags", text("2")),
            ("trigger_bounds", text(BOUNDS_JSON)),
        ],
    );
    insert(
        &conn,
        "doors",
        &[
            ("form_key", text("AUTO_NO_TEXT")),
            ("flags", text("66")),
            ("trigger_bounds", text(BOUNDS_JSON)),
        ],
    );
    insert(
        &conn,
        "doors",
        &[
            ("form_key", text("PLAIN")),
            ("flags", text("0")),
            ("trigger_bounds", text(BOUNDS_JSON)),
        ],
    );
    conn
}

fn placed_door(
    conn: &Connection,
    form_key: &str,
    cell: &str,
    base: &str,
    extra: &[(&'static str, Value)],
) {
    let mut row = placed(form_key, cell, (10.0, 20.0, 30.0));
    let overridden =
        |column: &str| column == "base_object" || extra.iter().any(|(name, _)| *name == column);
    row.retain(|(column, _)| !overridden(column));
    row.push(("base_object", text(base)));
    row.extend(extra.iter().cloned());
    insert(conn, "placed_objects", &row);
    teleport(conn, form_key, "dest_int");
}

#[test]
fn walk_in_doors_are_the_automatic_doors_with_location_pose_and_bounds() {
    let conn = walk_in_world();
    placed_door(
        &conn,
        "outdoor",
        "EXT",
        "AUTO",
        &[("scale", real(1.5)), ("rotation_z", real(0.5))],
    );
    placed_door(&conn, "indoor", "INT", "AUTO_NO_TEXT", &[]);
    placed_door(&conn, "ordinary", "EXT", "PLAIN", &[]);
    let doors = client(conn).get_walk_in_doors().unwrap();

    assert_eq!(doors.without_bounds, Vec::<String>::new());
    assert!(doors.rejected.is_empty(), "{:?}", doors.rejected);
    assert_eq!(doors.doors.len(), 2);
    let indoor = &doors.doors[0];
    assert_eq!(indoor.source_ref, "indoor");
    assert_eq!(
        indoor.location,
        DoorLocation::Interior { cell: "INT".into() }
    );
    let outdoor = &doors.doors[1];
    assert_eq!(outdoor.source_ref, "outdoor");
    assert_eq!(
        outdoor.location,
        DoorLocation::Exterior {
            worldspace: "WS".into()
        }
    );
    assert_eq!(outdoor.position, [10.0, 20.0, 30.0]);
    assert_eq!(outdoor.rotation, [0.0, 0.0, 0.5]);
    assert_eq!(outdoor.scale, 1.5);
    assert_eq!(outdoor.bounds.min, [-64.0, -8.0, 0.0]);
    assert_eq!(outdoor.bounds.max, [64.0, 8.0, 192.0]);
}

#[test]
fn walk_in_doors_exclude_refs_that_start_disabled_or_hidden() {
    let conn = walk_in_world();
    placed_door(&conn, "on", "EXT", "AUTO", &[]);
    placed_door(
        &conn,
        "disabled",
        "EXT",
        "AUTO",
        &[("is_initially_disabled", int(1))],
    );
    placed_door(
        &conn,
        "hidden",
        "EXT",
        "AUTO",
        &[("initially_hidden", int(1))],
    );
    let doors = client(conn).get_walk_in_doors().unwrap();
    let refs: Vec<_> = doors.doors.iter().map(|d| d.source_ref.as_str()).collect();
    assert_eq!(refs, ["on"]);
}

#[test]
fn walk_in_doors_without_trigger_bounds_are_reported_not_defaulted() {
    let conn = walk_in_world();
    conn.execute(
        "UPDATE doors SET trigger_bounds = NULL WHERE form_key = 'AUTO'",
        [],
    )
    .unwrap();
    placed_door(&conn, "nobounds", "EXT", "AUTO", &[]);
    placed_door(&conn, "fine", "EXT", "AUTO_NO_TEXT", &[]);
    let doors = client(conn).get_walk_in_doors().unwrap();
    assert_eq!(doors.without_bounds, ["nobounds"]);
    assert_eq!(doors.doors.len(), 1);
    assert_eq!(doors.doors[0].source_ref, "fine");
}

#[test]
fn walk_in_doors_from_a_database_without_the_trigger_bounds_column_report_every_ref() {
    let conn = with_trigger_bounds_column(schema_connection(), false);
    insert(
        &conn,
        "cells",
        &[("form_key", text("INT")), ("is_interior", int(1))],
    );
    insert(
        &conn,
        "placed_objects",
        &placed("dest_int", "INT", (0.0, 0.0, 0.0)),
    );
    insert(
        &conn,
        "doors",
        &[("form_key", text("AUTO")), ("flags", text("2"))],
    );
    placed_door(&conn, "legacy", "INT", "AUTO", &[]);
    let doors = client(conn).get_walk_in_doors().unwrap();
    assert_eq!(doors.doors.len(), 0);
    assert_eq!(doors.without_bounds, ["legacy"]);
}

fn rejected_refs(doors: &WalkInDoors) -> Vec<String> {
    doors.rejected.iter().map(ToString::to_string).collect()
}

fn walk_in_refs(doors: &WalkInDoors) -> Vec<&str> {
    doors.doors.iter().map(|d| d.source_ref.as_str()).collect()
}

#[test]
fn walk_in_doors_with_malformed_flags_or_bounds_are_rejected_alone_naming_the_ref() {
    let conn = walk_in_world();
    conn.execute("UPDATE doors SET flags = 'x' WHERE form_key = 'PLAIN'", [])
        .unwrap();
    placed_door(&conn, "badflags", "EXT", "PLAIN", &[]);
    placed_door(&conn, "fine", "EXT", "AUTO", &[]);
    let doors = client(conn).get_walk_in_doors().unwrap();
    let rejected = rejected_refs(&doors);
    assert!(
        rejected.len() == 1 && rejected[0].contains("badflags"),
        "{rejected:?}"
    );
    assert_eq!(walk_in_refs(&doors), ["fine"]);

    let conn = walk_in_world();
    conn.execute(
        "UPDATE doors SET trigger_bounds = '{\"Min\":1}' WHERE form_key = 'AUTO'",
        [],
    )
    .unwrap();
    placed_door(&conn, "badbounds", "EXT", "AUTO", &[]);
    placed_door(&conn, "fine", "EXT", "AUTO_NO_TEXT", &[]);
    let doors = client(conn).get_walk_in_doors().unwrap();
    let rejected = rejected_refs(&doors);
    assert!(
        rejected.len() == 1 && rejected[0].contains("badbounds"),
        "{rejected:?}"
    );
    assert_eq!(walk_in_refs(&doors), ["fine"]);
}

#[test]
fn walk_in_doors_standing_in_an_exterior_cell_without_a_worldspace_are_rejected_naming_the_ref() {
    let conn = walk_in_world();
    insert(&conn, "cells", &cell_row("ORPHAN", None, Some((0, 0))));
    placed_door(&conn, "lost", "ORPHAN", "AUTO", &[]);
    placed_door(&conn, "fine", "EXT", "AUTO", &[]);
    let doors = client(conn).get_walk_in_doors().unwrap();
    let rejected = rejected_refs(&doors);
    assert!(
        rejected.len() == 1 && rejected[0].contains("lost"),
        "{rejected:?}"
    );
    assert_eq!(walk_in_refs(&doors), ["fine"]);
}

// ---------------------------------------------------------- material swaps

#[test]
fn base_material_swaps_skip_rows_without_a_swap() {
    let conn = schema_connection();
    insert(
        &conn,
        "base_material_swaps",
        &[("form_key", text("A")), ("material_swap", text("M1"))],
    );
    insert(&conn, "base_material_swaps", &[("form_key", text("B"))]);
    let swaps = client(conn).get_base_material_swaps().unwrap();
    assert_eq!(swaps.len(), 1);
    assert_eq!(swaps["A"], "M1");
}

#[test]
fn material_swap_entries_group_by_swap_and_normalise_empty_and_out_of_range_values() {
    let conn = schema_connection();
    insert(
        &conn,
        "material_swap_entries",
        &[
            ("swap_form_key", text("S1")),
            ("original_material", text("a.bgsm")),
            ("diffuse", text("")),
            ("normal", text("n.dds")),
            ("is_blend", int(1)),
            ("alpha_mult", real(0.5)),
            ("alpha_test", int(300)),
            ("two_sided", int(1)),
            ("material_name", text("")),
        ],
    );
    insert(
        &conn,
        "material_swap_entries",
        &[
            ("swap_form_key", text("S1")),
            ("original_material", text("b.bgsm")),
            ("alpha_test", int(-5)),
            ("material_name", text("d.dds|n.dds")),
        ],
    );
    insert(
        &conn,
        "material_swap_entries",
        &[
            ("swap_form_key", text("S2")),
            ("original_material", text("c.bgsm")),
        ],
    );
    let entries = client(conn).get_material_swap_entries().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries["S1"].len(), 2);

    let first = entries["S1"]
        .iter()
        .find(|e| e.original_material == "a.bgsm")
        .unwrap();
    assert_eq!(first.diffuse, None, "empty string means no override");
    assert_eq!(first.normal.as_deref(), Some("n.dds"));
    assert!(first.is_blend && first.two_sided);
    assert_eq!(first.alpha_mult, 0.5);
    assert_eq!(first.alpha_test, Some(255), "clamped from 300");
    assert_eq!(first.material_name, None);

    let second = entries["S1"]
        .iter()
        .find(|e| e.original_material == "b.bgsm")
        .unwrap();
    assert_eq!(second.alpha_test, Some(0), "clamped from -5");
    assert_eq!(second.alpha_mult, 1.0, "schema default");
    assert!(!second.is_blend && !second.two_sided);
    assert_eq!(second.material_name.as_deref(), Some("d.dds|n.dds"));

    assert_eq!(entries["S2"][0].alpha_test, None);
}

#[test]
fn material_swap_entries_without_the_material_name_column_read_it_as_none() {
    let conn = legacy_connection(&[("material_swap_entries", "material_name")]);
    insert(
        &conn,
        "material_swap_entries",
        &[
            ("swap_form_key", text("S")),
            ("original_material", text("a.bgsm")),
        ],
    );
    let entries = client(conn).get_material_swap_entries().unwrap();
    assert_eq!(entries["S"][0].material_name, None);
}

// ----------------------------------------------------------------- lights

#[test]
fn light_flags_stored_as_legacy_text_are_read_as_integers() {
    let conn = schema_connection();
    insert(
        &conn,
        "lights",
        &[
            ("form_key", text("L")),
            ("flags", text("2049")),
            ("radius", real(256.0)),
        ],
    );
    let light = client(conn).get_lights().unwrap().remove(0);
    assert_eq!(light.flags, Some(2049));
    assert_eq!(light.radius, Some(256.0));
    assert!(!light.is_marker);
    assert_eq!(light.gobo, None);
}

#[test]
fn lights_without_the_light_data_columns_read_them_as_none() {
    let conn = legacy_connection(&[
        ("lights", "constant"),
        ("lights", "scalar"),
        ("lights", "exponent"),
        ("lights", "fov"),
        ("lights", "near_clip"),
        ("lights", "flicker_period"),
        ("lights", "flicker_intensity_amplitude"),
        ("lights", "flicker_movement_amplitude"),
        ("lights", "gobo"),
        ("lights", "lens"),
        ("lights", "god_rays"),
        ("lights", "god_rays_near_clip"),
    ]);
    insert(
        &conn,
        "lights",
        &[("form_key", text("L")), ("radius", real(10.0))],
    );
    let light = client(conn).get_lights().unwrap().remove(0);
    assert_eq!(light.radius, Some(10.0));
    assert_eq!((light.constant, light.fov, light.gobo), (None, None, None));
}

// ---------------------------------------------------------------- weather

#[test]
fn weather_row_maps_columns_and_keeps_null_json_as_none() {
    let conn = schema_connection();
    insert(
        &conn,
        "weather",
        &[
            ("form_key", text("W")),
            ("editor_id", text("Clear")),
            ("colors", text("{}")),
            ("fog", text("{\"Day\":1}")),
        ],
    );
    let weather = client(conn).get_weather("W").unwrap();
    assert_eq!(weather.editor_id.as_deref(), Some("Clear"));
    assert_eq!(weather.colors.as_deref(), Some("{}"));
    assert_eq!(weather.fog.as_deref(), Some("{\"Day\":1}"));
    assert_eq!(weather.directional_ambient, None);
    assert_eq!(weather.image_spaces, None);
}

#[test]
fn weather_without_the_fog_column_reads_fog_as_none() {
    let conn = legacy_connection(&[("weather", "fog")]);
    insert(
        &conn,
        "weather",
        &[("form_key", text("W")), ("colors", text("{}"))],
    );
    let db = client(conn);
    assert!(!db.schema().weather_fog_column_present);
    assert_eq!(db.get_weather("W").unwrap().fog, None);
}

#[test]
fn unknown_weather_is_a_no_rows_error() {
    let error = client(schema_connection())
        .get_weather("nope")
        .err()
        .unwrap();
    assert!(matches!(error, rusqlite::Error::QueryReturnedNoRows));
}

#[test]
fn worldspace_climate_distinguishes_no_climate_from_unknown_worldspace() {
    let conn = schema_connection();
    insert(
        &conn,
        "worldspaces",
        &[("form_key", text("WS1")), ("climate", text("CLIM"))],
    );
    insert(&conn, "worldspaces", &[("form_key", text("WS2"))]);
    let db = client(conn);
    assert_eq!(
        db.get_worldspace_climate("WS1").unwrap().as_deref(),
        Some("CLIM")
    );
    assert_eq!(db.get_worldspace_climate("WS2").unwrap(), None);
    assert!(matches!(
        db.get_worldspace_climate("WS3").unwrap_err(),
        rusqlite::Error::QueryReturnedNoRows
    ));
}

#[test]
fn climate_and_image_space_data_are_returned_verbatim() {
    let conn = schema_connection();
    insert(
        &conn,
        "climates",
        &[
            ("form_key", text("CL")),
            ("data", text("{\"Weathers\":[]}")),
        ],
    );
    insert(&conn, "image_spaces", &[("form_key", text("IS"))]);
    let db = client(conn);
    assert_eq!(
        db.get_climate_data("CL").unwrap().as_deref(),
        Some("{\"Weathers\":[]}")
    );
    assert_eq!(db.get_image_space_data("IS").unwrap(), None);
    assert!(matches!(
        db.get_climate_data("missing").unwrap_err(),
        rusqlite::Error::QueryReturnedNoRows
    ));
}

// ------------------------------------------------- statics / misc records

#[test]
fn static_maps_flags_and_json_columns() {
    let conn = schema_connection();
    insert(
        &conn,
        "statics",
        &[
            ("form_key", text("S")),
            ("model_path", text("Meshes\\a.nif")),
            ("has_distant_lod", int(1)),
            ("is_marker", int(0)),
            ("lod_model_path", text("Meshes\\LOD\\a_lod.nif")),
        ],
    );
    let db = client(conn);
    let one = db.get_static("S").unwrap();
    assert!(one.has_distant_lod && !one.is_marker);
    assert_eq!(
        one.lod_model_path.as_deref(),
        Some("Meshes\\LOD\\a_lod.nif")
    );
    assert_eq!(one.alternate_textures, None);
    assert_eq!(db.get_statics().unwrap().len(), 1);
    assert!(is_no_rows(&db.get_static("missing").err().unwrap()));
}

#[test]
fn movement_type_reads_the_speeds_from_the_json_payload() {
    let conn = schema_connection();
    insert(
        &conn,
        "movement_types",
        &[
            ("form_key", text("M")),
            ("editor_id", text("NPCDefault")),
            (
                "data",
                text("{\"ForwardWalk\":100.5,\"ForwardRun\":250,\"ForwardSprint\":400.25}"),
            ),
        ],
    );
    let movement = client(conn).get_movement_type("NPCDefault").unwrap();
    assert_eq!(
        (
            movement.forward_walk,
            movement.forward_run,
            movement.forward_sprint
        ),
        (100.5, 250.0, 400.25)
    );
}

#[test]
fn movement_type_names_the_missing_field() {
    let conn = schema_connection();
    insert(
        &conn,
        "movement_types",
        &[
            ("form_key", text("M")),
            ("editor_id", text("Broken")),
            ("data", text("{\"ForwardWalk\":1,\"ForwardRun\":2}")),
        ],
    );
    let error = client(conn)
        .get_movement_type("Broken")
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("ForwardSprint") && error.contains("Broken"),
        "{error}"
    );
}

#[test]
fn movement_type_with_malformed_json_is_an_error_not_a_panic() {
    let conn = schema_connection();
    insert(
        &conn,
        "movement_types",
        &[
            ("form_key", text("M")),
            ("editor_id", text("Bad")),
            ("data", text("{nope")),
        ],
    );
    let error = client(conn).get_movement_type("Bad").err().unwrap();
    assert!(!is_no_rows(&error), "{error}");
    assert!(error.to_string().contains("Bad"), "{error}");
}

#[test]
fn unknown_movement_type_is_a_no_rows_error() {
    let error = client(schema_connection())
        .get_movement_type("x")
        .err()
        .unwrap();
    assert!(is_no_rows(&error));
}

#[test]
fn landscape_textures_left_join_their_texture_set_diffuse() {
    let conn = schema_connection();
    insert(
        &conn,
        "texture_sets",
        &[("form_key", text("TS")), ("diffuse", text("grass_d.dds"))],
    );
    insert(
        &conn,
        "landscape_textures",
        &[("form_key", text("LT1")), ("texture_set", text("TS"))],
    );
    insert(
        &conn,
        "landscape_textures",
        &[("form_key", text("LT2")), ("texture_set", text("missing"))],
    );
    let mut rows = client(conn).get_landscape_textures().unwrap();
    rows.sort_by(|a, b| a.form_key.cmp(&b.form_key));
    assert_eq!(rows[0].texture_set_diffuse.as_deref(), Some("grass_d.dds"));
    assert_eq!(rows[1].texture_set_diffuse, None);
}

#[test]
fn worldspaces_are_listed_by_editor_id_ignoring_case() {
    let conn = schema_connection();
    insert(
        &conn,
        "worldspaces",
        &[("form_key", text("2")), ("editor_id", text("zeta"))],
    );
    insert(
        &conn,
        "worldspaces",
        &[("form_key", text("1")), ("editor_id", text("Alpha"))],
    );
    let db = client(conn);
    let order: Vec<_> = db
        .get_worldspaces()
        .unwrap()
        .into_iter()
        .filter_map(|w| w.form_key)
        .collect();
    assert_eq!(order, ["1", "2"]);
    assert_eq!(
        db.get_worldspace("1").unwrap().editor_id.as_deref(),
        Some("Alpha")
    );
}

// ----------------------------------------------------------------- water

const WATER_JSON: &str = r#"{
    "Name": "Fixture Water", "Opacity": 77, "Flags": ["Dangerous"],
    "FogDepthAmount": 1.5, "FogShallowColor": [10, 20, 30, 40], "FogDeepColor": [11, 21, 31, 41],
    "FogColorShallowRange": 2.0, "FogColorDeepRange": 3.0, "FogShallowAlpha": 0.4, "FogDeepAlpha": 0.6,
    "FogAlphaShallowRange": 4.0, "FogAlphaDeepRange": 5.0, "FogUnderwaterColor": [12, 22, 32, 42],
    "FogUnderwaterAmount": 6.0, "FogUnderwaterNear": 7.0, "FogUnderwaterFar": 8.0,
    "PhysicalNormalMagnitude": 0.11, "PhysicalShallowNormalFalloff": 0.12, "PhysicalDeepNormalFalloff": 0.13,
    "PhysicalReflectivityAmount": 0.14, "PhysicalFresnelAmount": 0.15, "PhysicalSurfaceEffectFalloff": 0.16,
    "PhysicalDisplacementSimulatorForce": 0.17, "PhysicalDisplacementSimulatorDampener": 0.2, "PhysicalReflectionColor": [13, 23, 33, 43],
    "SpecularSunSpecularPower": 9.0, "SpecularSunSpecularMagnitude": 10.0, "SpecularSunSparklePower": 11.0,
    "SpecularSunSparkleMagnitude": 12.0, "SpecularInteriorSpecularRadius": 13.0,
    "SpecularInteriorSpecularBrightness": 14.0, "SpecularInteriorSpecularPower": 15.0,
    "SiltAmount": 0.5, "SiltLightColor": [14, 24, 34, 44], "SiltDarkColor": [15, 25, 35, 45],
    "ScreenSpaceReflections": true, "LinearVelocity": [0.25, -0.5, 0.0], "AngularVelocity": [0.0, 0.0, 0.125],
    "NoiseLayerOne": {"WindDirection": 0.125, "WindSpeed": 1.125, "AmplitudeScale": 2.125, "UvScale": 3.125, "NoiseFalloff": 4.125, "Texture": "data\\Textures\\Water\\One.dds"},
    "NoiseLayerTwo": {"WindDirection": 0.25, "WindSpeed": 1.25, "AmplitudeScale": 2.25, "UvScale": 3.25, "NoiseFalloff": 4.25, "Texture": null},
    "NoiseLayerThree": {"WindDirection": 0.5, "WindSpeed": 1.5, "AmplitudeScale": 2.5, "UvScale": 3.5, "NoiseFalloff": 4.5, "Texture": "data\\Textures\\Water\\Three.dds"}
}"#;

fn water_row(form_key: &str, data: Option<&str>) -> Vec<(&'static str, Value)> {
    let mut row = vec![
        ("form_key", text(form_key)),
        ("editor_id", text("FixtureWater")),
        ("material", text("0ABCE2:Fallout4.esm")),
    ];
    if let Some(data) = data {
        row.push(("data", text(data)));
    }
    row
}

#[test]
fn get_water_decodes_the_installer_json_under_mutagen_names() {
    let conn = schema_connection();
    insert(&conn, "waters", &water_row("W1", Some(WATER_JSON)));
    let water = client(conn).get_water("W1").unwrap();
    assert_eq!(water.editor_id.as_deref(), Some("FixtureWater"));
    assert_eq!(water.material.as_deref(), Some("0ABCE2:Fallout4.esm"));
    let data = water.data.unwrap();
    assert_eq!(data.opacity, 77);
    assert_eq!(data.fog_shallow_color, [10, 20, 30, 40]);
    assert_eq!(data.fog_alpha_deep_range, 5.0);
    assert_eq!(data.physical_fresnel_amount, 0.15);
    assert_eq!(data.specular_sun_sparkle_magnitude, 12.0);
    assert_eq!(data.silt_dark_color, [15, 25, 35, 45]);
    assert!(data.screen_space_reflections);
    assert_eq!(data.noise_layer_one.wind_speed, 1.125);
    assert_eq!(
        data.noise_layer_one.texture.as_deref(),
        Some("data\\Textures\\Water\\One.dds")
    );
    assert_eq!(data.noise_layer_two.texture, None);
}

#[test]
fn get_water_without_data_json_has_no_data() {
    let conn = schema_connection();
    insert(&conn, "waters", &water_row("W1", None));
    assert_eq!(client(conn).get_water("W1").unwrap().data, None);
}

#[test]
fn get_water_with_a_missing_field_names_the_record_and_the_field() {
    let conn = schema_connection();
    let without_fresnel = WATER_JSON.replace("\"PhysicalFresnelAmount\": 0.15,", "");
    insert(&conn, "waters", &water_row("W1", Some(&without_fresnel)));
    let error = client(conn).get_water("W1").unwrap_err().to_string();
    assert!(error.contains("W1"), "{error}");
    assert!(error.contains("PhysicalFresnelAmount"), "{error}");
}

#[test]
fn get_waters_fails_on_one_unusable_record_instead_of_skipping_it() {
    let conn = schema_connection();
    insert(&conn, "waters", &water_row("W1", Some(WATER_JSON)));
    insert(&conn, "waters", &water_row("W2", Some("{not json")));
    let error = client(conn).get_waters().unwrap_err().to_string();
    assert!(error.contains("W2"), "{error}");
}

#[test]
fn get_water_for_an_unknown_record_is_a_no_rows_error() {
    let error = client(schema_connection()).get_water("nope").unwrap_err();
    assert!(is_no_rows(&error));
}

#[test]
fn cell_water_reads_flags_height_type_and_currents() {
    let conn = schema_connection();
    insert(
        &conn,
        "cells",
        &[
            ("form_key", text("C1")),
            ("has_water", int(1)),
            ("no_lod_water", int(1)),
            ("water_height", real(1300.5)),
            ("water_type", text("000018:Fallout4.esm")),
            ("water_environment_map", text("data\\Textures\\cube.dds")),
            (
                "water_currents",
                text("[[1.0, 2.0, 3.0, 0.5], [0.0, 0.25, 0.0, 1.0]]"),
            ),
        ],
    );
    let water = client(conn).get_cell_water("C1").unwrap();
    assert!(water.has_water && water.no_lod_water);
    assert_eq!(water.height, CellWaterHeight::Level(1300.5));
    assert_eq!(water.water_type.as_deref(), Some("000018:Fallout4.esm"));
    assert_eq!(
        water.environment_map.as_deref(),
        Some("data\\Textures\\cube.dds")
    );
    assert_eq!(
        water.currents.unwrap(),
        [[1.0, 2.0, 3.0, 0.5], [0.0, 0.25, 0.0, 1.0]]
    );
}

#[test]
fn cell_water_height_distinguishes_unset_sentinel_from_absent() {
    let conn = schema_connection();
    insert(
        &conn,
        "cells",
        &[
            ("form_key", text("SENTINEL")),
            ("water_height", real(f64::from(f32::MAX))),
        ],
    );
    insert(&conn, "cells", &[("form_key", text("ABSENT"))]);
    insert(
        &conn,
        "cells",
        &[
            ("form_key", text("ZERO")),
            ("has_water", int(1)),
            ("water_height", real(0.0)),
        ],
    );
    let db = client(conn);
    assert_eq!(
        db.get_cell_water("SENTINEL").unwrap().height,
        CellWaterHeight::Unset
    );
    assert_eq!(
        db.get_cell_water("ABSENT").unwrap().height,
        CellWaterHeight::Absent
    );
    assert_eq!(
        db.get_cell_water("ZERO").unwrap().height,
        CellWaterHeight::Level(0.0)
    );
    assert!(!db.get_cell_water("SENTINEL").unwrap().has_water);
}

#[test]
fn cell_water_on_a_database_without_the_columns_reads_as_absent() {
    let conn = legacy_connection(&[
        ("cells", "has_water"),
        ("cells", "no_lod_water"),
        ("cells", "water_height"),
        ("cells", "water_type"),
        ("cells", "water_environment_map"),
        ("cells", "water_currents"),
    ]);
    insert(&conn, "cells", &[("form_key", text("C1"))]);
    let db = client(conn);
    assert!(!db.schema().cell_water_columns_present);
    let water = db.get_cell_water("C1").unwrap();
    assert!(!water.has_water);
    assert_eq!(water.height, CellWaterHeight::Absent);
    assert_eq!(water.water_type, None);
}

#[test]
fn a_cell_with_only_some_water_columns_is_rejected() {
    let conn = legacy_connection(&[("cells", "water_height")]);
    let error = DbSchema::detect(&conn).unwrap_err().to_string();
    assert!(
        error.contains("cells") && error.contains("water_height"),
        "{error}"
    );
}

#[test]
fn get_cell_water_for_an_unknown_cell_is_a_no_rows_error() {
    let error = client(schema_connection())
        .get_cell_water("nope")
        .unwrap_err();
    assert!(is_no_rows(&error));
}

#[test]
fn worldspace_water_reads_types_and_heights() {
    let conn = schema_connection();
    insert(
        &conn,
        "worldspaces",
        &[
            ("form_key", text("WS")),
            ("water_type", text("000018:Fallout4.esm")),
            ("lod_water_type", text("000019:Fallout4.esm")),
            ("lod_water_height", real(450.0)),
            ("default_land_height", real(-2000.0)),
            ("default_water_height", real(450.0)),
        ],
    );
    let water = client(conn).get_worldspace_water("WS").unwrap();
    assert_eq!(water.water_type.as_deref(), Some("000018:Fallout4.esm"));
    assert_eq!(water.lod_water_type.as_deref(), Some("000019:Fallout4.esm"));
    assert_eq!(water.lod_water_height, Some(450.0));
    assert_eq!(water.default_land_height, Some(-2000.0));
    assert_eq!(water.default_water_height, Some(450.0));
    assert_eq!(water.environment_map, None);
}

#[test]
fn worldspace_water_reads_parent_inheritance_and_flags() {
    let conn = schema_connection();
    insert(
        &conn,
        "worldspaces",
        &[
            ("form_key", text("CHILD")),
            ("parent_worldspace", text("PARENT")),
            ("parent_flags", int(0x1F)),
            ("flags", text("217")),
        ],
    );
    insert(&conn, "worldspaces", &[("form_key", text("ROOT"))]);
    let db = client(conn);
    let child = db.get_worldspace_water("CHILD").unwrap();
    assert_eq!(child.parent_worldspace.as_deref(), Some("PARENT"));
    assert_eq!(child.parent_flags, Some(0x1F));
    assert_eq!(child.flags, Some(217));
    let root = db.get_worldspace_water("ROOT").unwrap();
    assert_eq!(
        (root.parent_worldspace, root.parent_flags, root.flags),
        (None, None, None)
    );
}

#[test]
fn worldspace_flags_that_are_not_an_integer_name_the_worldspace() {
    let conn = schema_connection();
    insert(
        &conn,
        "worldspaces",
        &[("form_key", text("WS")), ("flags", text("{\"x\": 1}"))],
    );
    let error = client(conn)
        .get_worldspace_water("WS")
        .unwrap_err()
        .to_string();
    assert!(error.contains("WS") && error.contains("flags"), "{error}");
}

fn water_cell_row(
    form_key: &str,
    worldspace: &str,
    grid: Option<(i64, i64)>,
    interior: bool,
    has_water: bool,
    height: f64,
) -> Vec<(&'static str, Value)> {
    let mut row = cell_row(form_key, Some(worldspace), grid);
    row.push(("is_interior", int(i64::from(interior))));
    row.push(("has_water", int(i64::from(has_water))));
    row.push(("water_height", real(height)));
    row
}

#[test]
fn exterior_cell_waters_are_the_gridded_water_cells_of_the_worldspace_by_grid() {
    let conn = schema_connection();
    let rows = [
        water_cell_row("A", "WS", Some((1, -2)), false, true, 1300.0),
        water_cell_row("B", "WS", Some((0, 0)), false, true, f64::from(f32::MAX)),
        water_cell_row("DRY", "WS", Some((5, 5)), false, false, 0.0),
        water_cell_row("PERSISTENT", "WS", None, false, true, 0.0),
        water_cell_row("INTERIOR", "WS", Some((9, 9)), true, true, 0.0),
        water_cell_row("OTHER", "WS2", Some((1, -2)), false, true, 7.0),
    ];
    for row in &rows {
        insert(&conn, "cells", row);
    }
    let cells = client(conn).get_exterior_cell_waters("WS").unwrap();
    assert_eq!(cells.len(), 2);
    assert_eq!(cells[&(1, -2)].height, CellWaterHeight::Level(1300.0));
    assert_eq!(cells[&(0, 0)].height, CellWaterHeight::Unset);
    assert!(cells[&(1, -2)].has_water);
}

#[test]
fn lod_water_quads_are_the_rows_of_the_worldspace_ordered_by_cell() {
    let conn = schema_connection();
    for (worldspace, x, y, height) in [
        ("WS", 3, -1, 450.0),
        ("WS", -2, 4, 1300.0),
        ("WS", -2, 3, 450.0),
        ("SANCTUARY", 0, 0, 450.0),
    ] {
        insert(
            &conn,
            "lod_water_quads",
            &[
                ("worldspace", text(worldspace)),
                ("cell_x", int(x)),
                ("cell_y", int(y)),
                ("height", real(height)),
            ],
        );
    }
    let db = client(conn);
    let quads = db.get_lod_water_quads("WS").unwrap();
    let cells: Vec<_> = quads
        .iter()
        .map(|quad| (quad.cell_x, quad.cell_y, quad.height))
        .collect();
    assert_eq!(cells, [(-2, 3, 450.0), (-2, 4, 1300.0), (3, -1, 450.0)]);
    assert_eq!(db.get_lod_water_quads("NONE").unwrap(), []);
}

#[test]
fn lod_water_quads_of_a_database_without_the_table_is_an_error_naming_it() {
    let conn = schema_connection();
    conn.execute_batch("DROP TABLE lod_water_quads;").unwrap();
    let error = client(conn)
        .get_lod_water_quads("WS")
        .unwrap_err()
        .to_string();
    assert!(error.contains("lod_water_quads"), "{error}");
}

#[test]
fn worldspace_water_heights_are_none_when_the_columns_are_null() {
    let conn = schema_connection();
    insert(&conn, "worldspaces", &[("form_key", text("WS"))]);
    let water = client(conn).get_worldspace_water("WS").unwrap();
    assert_eq!(water.lod_water_height, None);
    assert_eq!(water.default_water_height, None);
    assert_eq!(water.water_type, None);
}

#[test]
fn activator_water_type_is_read_and_null_for_ordinary_activators() {
    let conn = schema_connection();
    insert(
        &conn,
        "activators",
        &[
            ("form_key", text("A1")),
            ("water_type", text("000018:Fallout4.esm")),
        ],
    );
    insert(&conn, "activators", &[("form_key", text("A2"))]);
    let mut activators = client(conn).get_activators().unwrap();
    activators.sort_by(|a, b| a.form_key.cmp(&b.form_key));
    assert_eq!(
        activators[0].water_type.as_deref(),
        Some("000018:Fallout4.esm")
    );
    assert_eq!(activators[1].water_type, None);
}

#[test]
fn activators_of_a_database_without_the_water_column_have_no_water_type() {
    let conn = legacy_connection(&[("activators", "water_type")]);
    insert(&conn, "activators", &[("form_key", text("A1"))]);
    let db = client(conn);
    assert!(!db.schema().activator_water_column_present);
    assert_eq!(db.get_activators().unwrap()[0].water_type, None);
}

fn placed_of(
    form_key: &str,
    cell: &str,
    base: &str,
    extra: &[(&'static str, Value)],
) -> Vec<(&'static str, Value)> {
    let mut row = placed(form_key, cell, (0.0, 0.0, 0.0));
    row.retain(|(column, _)| {
        *column != "base_object" && extra.iter().all(|(overridden, _)| overridden != column)
    });
    row.push(("base_object", text(base)));
    row.extend(extra.iter().cloned());
    row
}

fn seed_water_activator_refs(conn: &Connection) {
    insert(
        conn,
        "activators",
        &[("form_key", text("POOL")), ("water_type", text("W1"))],
    );
    insert(conn, "activators", &[("form_key", text("LEVER"))]);
    let pool = placed_of(
        "R1",
        "C1",
        "POOL",
        &[
            ("position_x", real(100.0)),
            ("position_y", real(200.0)),
            ("position_z", real(300.0)),
            ("scale", real(2.0)),
        ],
    );
    insert(conn, "placed_objects", &pool);
    insert(conn, "placed_objects", &placed_of("R2", "C1", "LEVER", &[]));
    insert(
        conn,
        "placed_objects",
        &placed_of("R3", "C1", "POOL", &[("initially_hidden", int(1))]),
    );
    insert(conn, "placed_objects", &placed_of("R4", "C2", "POOL", &[]));
}

#[test]
fn placed_water_refs_are_the_visible_refs_of_water_activators_in_the_cell() {
    let conn = schema_connection();
    seed_water_activator_refs(&conn);
    let refs = client(conn).get_placed_water_refs("C1").unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].form_key.as_deref(), Some("R1"));
    assert_eq!(refs[0].base_object, "POOL");
    assert_eq!(refs[0].water_type, "W1");
    assert_eq!(refs[0].position, [100.0, 200.0, 300.0]);
    assert_eq!(refs[0].scale, 2.0);
}

#[test]
fn cells_with_placed_water_rank_by_visible_water_ref_count() {
    let conn = schema_connection();
    seed_water_activator_refs(&conn);
    let ranked = client(conn).get_cells_with_placed_water(10).unwrap();
    assert_eq!(ranked, [("C1".to_string(), 1), ("C2".to_string(), 1)]);
}

/// A complete `grasses` row for `form_key`, with `overrides` replacing columns.
fn grass_columns(form_key: &str, overrides: &[(&str, Value)]) -> Vec<(&'static str, Value)> {
    let mut columns = vec![
        ("form_key", text(form_key)),
        ("editor_id", text("TestGrass")),
        ("model_path", text("landscape\\grass\\test.nif")),
        ("flags", int(5)),
        ("density", int(40)),
        ("min_slope", int(3)),
        ("max_slope", int(55)),
        ("units_from_water", int(64)),
        ("units_from_water_type", int(7)),
        ("position_range", real(32.0)),
        ("height_range", real(0.5)),
        ("color_range", real(0.25)),
        ("wave_period", real(10.0)),
    ];
    for (name, value) in overrides {
        let column = columns.iter_mut().find(|(c, _)| c == name).unwrap();
        column.1 = value.clone();
    }
    columns
}

fn grass_error(overrides: &[(&str, Value)]) -> rusqlite::Error {
    let conn = schema_connection();
    insert(&conn, "grasses", &grass_columns("G1", overrides));
    let error = client(conn).get_grasses().unwrap_err();
    error
        .downcast::<rusqlite::Error>()
        .expect("the row error stays a rusqlite error")
}

#[test]
fn a_null_numeric_grass_column_is_an_error_not_zero() {
    for column in [
        "flags",
        "density",
        "units_from_water_type",
        "position_range",
    ] {
        assert!(
            matches!(
                grass_error(&[(column, Value::Null)]),
                rusqlite::Error::InvalidColumnType(..)
            ),
            "{column}"
        );
    }
}

#[test]
fn an_out_of_range_grass_column_is_an_error_not_a_wrap() {
    for (column, value) in [
        ("flags", 256),
        ("density", -1),
        ("units_from_water", 65_536),
        ("units_from_water_type", 1 << 32),
    ] {
        assert!(
            matches!(
                grass_error(&[(column, int(value))]),
                rusqlite::Error::IntegralValueOutOfRange(..)
            ),
            "{column}"
        );
    }
}

#[test]
fn grasses_read_every_data_field() {
    let conn = schema_connection();
    insert(&conn, "grasses", &grass_columns("G1", &[]));

    let rows = client(conn).get_grasses().unwrap();
    assert_eq!(rows.len(), 1);

    let grass = &rows[0];
    assert_eq!(grass.editor_id.as_deref(), Some("TestGrass"));
    assert_eq!(
        grass.model_path.as_deref(),
        Some("landscape\\grass\\test.nif")
    );
    assert_eq!(
        (grass.flags, grass.density, grass.min_slope, grass.max_slope),
        (5, 40, 3, 55)
    );
    assert_eq!(
        (grass.units_from_water, grass.units_from_water_type),
        (64, 7)
    );
    assert_eq!(
        (
            grass.position_range,
            grass.height_range,
            grass.color_range,
            grass.wave_period
        ),
        (32.0, 0.5, 0.25, 10.0)
    );
}

#[test]
fn texture_grass_lists_come_back_grouped_by_texture_in_gnam_order() {
    let conn = schema_connection();
    for (ltex, grass, ord) in [("LB", "g1", 0), ("LA", "g3", 1), ("LA", "g2", 0)] {
        insert(
            &conn,
            "ltex_grasses",
            &[
                ("ltex_form_key", text(ltex)),
                ("grass_form_key", text(grass)),
                ("ord", int(ord)),
            ],
        );
    }
    let rows = client(conn).get_ltex_grasses().unwrap();
    let flattened: Vec<_> = rows
        .iter()
        .map(|r| (r.ltex_form_key.as_str(), r.grass_form_key.as_str(), r.ord))
        .collect();
    assert_eq!(
        flattened,
        [("LA", "g2", 0), ("LA", "g3", 1), ("LB", "g1", 0)]
    );
}

// ---------------------------------------------------------------- Pip-Boy

const COMMONWEALTH: &str = "00003C:Fallout4.esm";

#[allow(clippy::too_many_arguments)]
fn perk_row(
    form_key: &str,
    name: Option<&str>,
    next: Option<&str>,
    level: Option<i64>,
    first_rank: Option<&str>,
    playable: Option<i64>,
    hidden: Option<i64>,
    swf: Option<&str>,
) -> Vec<(&'static str, Value)> {
    let opt_text = |v: Option<&str>| v.map_or(Value::Null, text);
    let opt_int = |v: Option<i64>| v.map_or(Value::Null, int);
    vec![
        ("form_key", text(form_key)),
        ("editor_id", text(&format!("{form_key}Id"))),
        ("name", opt_text(name)),
        ("description", text("desc")),
        ("next_perk", opt_text(next)),
        ("level", opt_int(level)),
        ("playable", opt_int(playable)),
        ("hidden", opt_int(hidden)),
        ("swf_path", opt_text(swf)),
        ("first_rank", opt_text(first_rank)),
    ]
}

fn marker_row(
    form_key: &str,
    worldspace: &str,
    marker_type: i64,
    flags: i64,
    position: (f64, f64, f64),
) -> Vec<(&'static str, Value)> {
    vec![
        ("form_key", text(form_key)),
        ("worldspace", text(worldspace)),
        ("name", text(&format!("{form_key} name"))),
        ("marker_type", int(marker_type)),
        ("flags", int(flags)),
        ("position_x", real(position.0)),
        ("position_y", real(position.1)),
        ("position_z", real(position.2)),
    ]
}

#[test]
fn perks_are_read_with_every_pipboy_column_and_null_numbers_default_to_zero() {
    let conn = schema_connection();
    insert(
        &conn,
        "perks",
        &perk_row(
            "P2",
            Some("Second"),
            None,
            Some(9),
            Some("P1"),
            Some(1),
            Some(0),
            Some("A\\B.swf"),
        ),
    );
    insert(
        &conn,
        "perks",
        &perk_row(
            "P1",
            Some("First"),
            Some("P2"),
            Some(1),
            Some("P1"),
            Some(1),
            Some(0),
            None,
        ),
    );
    insert(
        &conn,
        "perks",
        &perk_row("P0", None, None, None, None, None, None, None),
    );
    let rows = client(conn).get_perks().unwrap();
    let keys: Vec<_> = rows.iter().map(|r| r.form_key.as_str()).collect();
    assert_eq!(keys, ["P0", "P1", "P2"], "ordered by form key");

    let first = &rows[1];
    assert_eq!(first.name.as_deref(), Some("First"));
    assert_eq!(first.editor_id.as_deref(), Some("P1Id"));
    assert_eq!(first.description.as_deref(), Some("desc"));
    assert_eq!(first.next_perk.as_deref(), Some("P2"));
    assert_eq!(first.level, 1);
    assert_eq!(first.first_rank.as_deref(), Some("P1"));
    assert_eq!(rows[2].first_rank.as_deref(), Some("P1"));
    assert!(first.playable && !first.hidden);
    assert_eq!(first.swf_path, None);
    assert_eq!(rows[2].swf_path.as_deref(), Some("A\\B.swf"));

    let unset = &rows[0];
    assert_eq!(unset.level, 0);
    assert_eq!(unset.first_rank, None);
    assert!(!unset.playable && !unset.hidden);
    assert_eq!(unset.name, None);
}

#[test]
fn perk_chart_rows_are_listed_by_perk() {
    let conn = schema_connection();
    for (perk, special, required) in [("P2", "luck", 10), ("P1", "strength", 1)] {
        insert(
            &conn,
            "perk_chart",
            &[
                ("perk", text(perk)),
                ("special", text(special)),
                ("required_special", int(required)),
            ],
        );
    }
    let rows = client(conn).get_perk_chart().unwrap();
    let flattened: Vec<_> = rows
        .iter()
        .map(|r| (r.perk.as_str(), r.special.as_str(), r.required_special))
        .collect();
    assert_eq!(flattened, [("P1", "strength", 1), ("P2", "luck", 10)]);
}

#[test]
fn special_attributes_are_listed_with_their_name_and_description() {
    let conn = schema_connection();
    for (special, form_key, name, description) in [
        ("luck", "AV2", Some("Luck"), None),
        ("strength", "AV1", Some("Strength"), Some("Raw power.")),
    ] {
        insert(
            &conn,
            "actor_value_informations",
            &[("form_key", text(form_key))],
        );
        insert(
            &conn,
            "special_attributes",
            &[
                ("special", text(special)),
                ("form_key", text(form_key)),
                ("name", name.map_or(Value::Null, text)),
                ("description", description.map_or(Value::Null, text)),
            ],
        );
    }
    let rows = client(conn).get_special_attributes().unwrap();
    let summary: Vec<_> = rows
        .iter()
        .map(|r| {
            (
                r.special.as_str(),
                r.name.as_deref(),
                r.description.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("luck", Some("Luck"), None),
            ("strength", Some("Strength"), Some("Raw power."))
        ]
    );
}

#[test]
fn items_join_their_weapon_and_armor_extension_rows() {
    let conn = schema_connection();
    insert(
        &conn,
        "items",
        &[
            ("form_key", text("W")),
            ("item_type", text("WEAP")),
            ("editor_id", text("Rifle")),
            ("name", text("Combat Rifle")),
            ("value", int(144)),
            ("weight", real(5.5)),
        ],
    );
    insert(
        &conn,
        "weapons",
        &[
            ("form_key", text("W")),
            ("damage", int(36)),
            ("fire_rate", real(7.5)),
            ("range", real(120.0)),
            ("accuracy", real(64.0)),
            ("ammo_type", text("AM")),
        ],
    );
    insert(
        &conn,
        "items",
        &[
            ("form_key", text("A")),
            ("item_type", text("ARMO")),
            ("name", text("Vest")),
            ("value", int(30)),
            ("weight", real(2.0)),
        ],
    );
    insert(
        &conn,
        "armors",
        &[
            ("form_key", text("A")),
            ("armor_rating", int(25)),
            ("body_slot", int(2048)),
        ],
    );
    insert(
        &conn,
        "items",
        &[("form_key", text("M")), ("item_type", text("MISC"))],
    );
    insert(
        &conn,
        "items",
        &[
            ("form_key", text("B")),
            ("item_type", text("WEAP")),
            ("name", text("Bare weapon row")),
        ],
    );
    insert(&conn, "weapons", &[("form_key", text("B"))]);

    let rows = client(conn).get_items().unwrap();
    let by_key = |key: &str| rows.iter().find(|r| r.form_key == key).unwrap();

    let rifle = by_key("W");
    assert_eq!(
        (rifle.item_type.as_str(), rifle.value, rifle.weight),
        ("WEAP", 144, 5.5)
    );
    let weapon = rifle.weapon.as_ref().unwrap();
    assert_eq!(weapon.damage, Some(36));
    assert_eq!(weapon.fire_rate, Some(7.5));
    assert_eq!((weapon.range, weapon.accuracy), (Some(120.0), Some(64.0)));
    assert_eq!(weapon.ammo_type.as_deref(), Some("AM"));
    assert!(rifle.armor.is_none());

    let vest = by_key("A");
    let armor = vest.armor.as_ref().unwrap();
    assert_eq!(
        (armor.armor_rating, armor.body_slot),
        (Some(25), Some(2048))
    );
    assert!(vest.weapon.is_none());

    let misc = by_key("M");
    assert!(misc.weapon.is_none() && misc.armor.is_none());
    assert_eq!((misc.name.as_deref(), misc.value), (None, 0));

    let bare = by_key("B").weapon.as_ref().unwrap();
    assert_eq!((bare.damage, bare.ammo_type.clone()), (None, None));
}

#[test]
fn map_markers_are_filtered_by_worldspace_and_carry_raw_type_flags_and_position() {
    let conn = schema_connection();
    insert(
        &conn,
        "map_markers",
        &marker_row("M2", COMMONWEALTH, 11, 0x03, (-80768.0, 88175.0, 12.5)),
    );
    insert(
        &conn,
        "map_markers",
        &marker_row("M1", COMMONWEALTH, 0, 0x00, (1.0, 2.0, 3.0)),
    );
    insert(
        &conn,
        "map_markers",
        &marker_row("X", "OTHER", 5, 0x01, (0.0, 0.0, 0.0)),
    );
    let db = client(conn);

    let rows = db.get_map_markers(COMMONWEALTH).unwrap();
    let keys: Vec<_> = rows.iter().map(|r| r.form_key.as_str()).collect();
    assert_eq!(keys, ["M1", "M2"]);
    let sanctuary = &rows[1];
    assert_eq!(sanctuary.name.as_deref(), Some("M2 name"));
    assert_eq!((sanctuary.marker_type, sanctuary.flags), (11, 0x03));
    assert_eq!(sanctuary.position, [-80768.0, 88175.0, 12.5]);
    assert_eq!(sanctuary.worldspace, COMMONWEALTH);
    assert_eq!(db.get_map_markers("NONE").unwrap(), []);
}

#[test]
fn a_marker_type_outside_the_u16_range_is_an_error_not_a_wrapped_value() {
    let conn = schema_connection();
    insert(
        &conn,
        "map_markers",
        &marker_row("M", COMMONWEALTH, 70000, 0, (0.0, 0.0, 0.0)),
    );
    assert!(client(conn).get_map_markers(COMMONWEALTH).is_err());
}

#[test]
fn the_worldspace_map_image_is_none_when_null_or_the_worldspace_is_unknown() {
    let conn = schema_connection();
    insert(
        &conn,
        "worldspaces",
        &[
            ("form_key", text(COMMONWEALTH)),
            ("map_image", text("Interface\\Pip-Boy\\WorldMap_d.dds")),
        ],
    );
    insert(&conn, "worldspaces", &[("form_key", text("NOMAP"))]);
    let db = client(conn);
    assert_eq!(
        db.get_worldspace_map_image(COMMONWEALTH)
            .unwrap()
            .as_deref(),
        Some("Interface\\Pip-Boy\\WorldMap_d.dds")
    );
    assert_eq!(db.get_worldspace_map_image("NOMAP").unwrap(), None);
    assert_eq!(db.get_worldspace_map_image("UNKNOWN").unwrap(), None);
}

fn stale_pipboy_connection() -> Connection {
    let conn = legacy_connection(&[
        ("perks", "level"),
        ("perks", "playable"),
        ("perks", "hidden"),
        ("perks", "swf_path"),
        ("perks", "first_rank"),
        ("worldspaces", "map_image"),
    ]);
    conn.execute_batch(
        "DROP TABLE perk_chart; DROP TABLE map_markers; DROP TABLE special_attributes;",
    )
    .unwrap();
    insert(
        &conn,
        "perks",
        &[("form_key", text("P")), ("name", text("Old perk"))],
    );
    insert(&conn, "worldspaces", &[("form_key", text(COMMONWEALTH))]);
    conn
}

#[test]
fn a_database_without_the_pipboy_tables_reports_the_groups_absent_and_reads_nothing() {
    let conn = stale_pipboy_connection();
    let schema = DbSchema::detect(&conn).unwrap();
    assert!(!schema.perk_pipboy_columns_present);
    assert!(!schema.perk_chart_columns_present);
    assert!(!schema.special_attribute_columns_present);
    assert!(!schema.map_marker_columns_present);
    assert!(!schema.worldspace_map_image_column_present);

    let db = client(conn);
    assert_eq!(db.get_special_attributes().unwrap(), []);
    assert_eq!(db.get_perks().unwrap(), []);
    assert_eq!(db.get_perk_chart().unwrap(), []);
    assert_eq!(db.get_map_markers(COMMONWEALTH).unwrap(), []);
    assert_eq!(db.get_worldspace_map_image(COMMONWEALTH).unwrap(), None);
    assert!(
        db.get_items().unwrap().is_empty(),
        "the item tables are not new"
    );
}

// -------------------------------------------------------- navigation meshes

/// The installer's `navigation_meshes` columns (see its data contract). Columns the loaded schema
/// already has are left alone, so this seed keeps working once the installer SQL declares them.
const NAVMESH_COLUMNS: &[(&str, &str)] = &[
    ("form_id", "INTEGER"),
    ("cell", "TEXT"),
    ("worldspace", "TEXT"),
    ("grid_x", "INTEGER"),
    ("grid_y", "INTEGER"),
    ("record_flags", "INTEGER"),
    ("format_version", "INTEGER"),
    ("vertex_count", "INTEGER"),
    ("triangle_count", "INTEGER"),
    ("edge_link_count", "INTEGER"),
    ("door_link_count", "INTEGER"),
    ("vertices", "BLOB"),
    ("triangles", "BLOB"),
    ("edge_links", "BLOB"),
    ("door_links", "BLOB"),
];

fn navmesh_connection() -> Connection {
    let conn = schema_connection();
    let existing: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('navigation_meshes')")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for (column, ty) in NAVMESH_COLUMNS {
        if !existing.iter().any(|name| name == column) {
            conn.execute_batch(&format!(
                "ALTER TABLE navigation_meshes ADD COLUMN {column} {ty};"
            ))
            .unwrap();
        }
    }
    conn
}

fn navmesh_row(form_key: &str, form_id: i64, cell: &str) -> Vec<(&'static str, Value)> {
    let vertices: Vec<u8> = [0.0f32, 0.0, 10.0, 100.0, 0.0, 10.0, 0.0, 100.0, 10.0]
        .iter()
        .flat_map(|c| c.to_le_bytes())
        .collect();
    let triangle: Vec<u8> = [0u16, 1, 2]
        .iter()
        .map(|i| i.to_le_bytes().to_vec())
        .chain([-1i16; 3].iter().map(|e| e.to_le_bytes().to_vec()))
        .chain(std::iter::once(0u16.to_le_bytes().to_vec()))
        .flatten()
        .collect();
    vec![
        ("form_key", text(form_key)),
        ("form_id", int(form_id)),
        ("cell", text(cell)),
        ("format_version", int(15)),
        ("vertex_count", int(3)),
        ("triangle_count", int(1)),
        ("edge_link_count", int(0)),
        ("door_link_count", int(0)),
        ("vertices", blob(&vertices)),
        ("triangles", blob(&triangle)),
        ("edge_links", blob(&[])),
        ("door_links", blob(&[])),
    ]
}

#[test]
fn navmeshes_of_a_cell_are_returned_with_their_raw_blobs() {
    let conn = navmesh_connection();
    insert(
        &conn,
        "navigation_meshes",
        &navmesh_row("N1", 0x1234, "CELL_A"),
    );
    insert(
        &conn,
        "navigation_meshes",
        &navmesh_row("N2", 0x1235, "CELL_A"),
    );
    insert(
        &conn,
        "navigation_meshes",
        &navmesh_row("N3", 0x1236, "CELL_B"),
    );

    let mut rows = client(conn).get_navmeshes_for_cell("CELL_A").unwrap();
    rows.sort_by_key(|row| row.form_id);

    assert_eq!(
        rows.iter().map(|r| r.form_id).collect::<Vec<_>>(),
        [0x1234, 0x1235]
    );
    let first = &rows[0];
    assert_eq!(first.form_key, "N1");
    assert_eq!(first.format_version, 15);
    assert_eq!((first.vertex_count, first.triangle_count), (3, 1));
    assert_eq!(first.vertices.len(), 36);
    assert_eq!(first.triangles.len(), 14);
    let source = openfo4::navmesh::decode(first).unwrap();
    assert_eq!(source.triangles, vec![[0, 1, 2]]);
}

#[test]
fn navmesh_stub_rows_without_geometry_are_skipped() {
    let conn = navmesh_connection();
    insert(&conn, "navigation_meshes", &navmesh_row("N1", 7, "CELL_A"));
    insert(
        &conn,
        "navigation_meshes",
        &[("form_key", text("STUB")), ("cell", text("CELL_A"))],
    );
    let db = client(conn);
    assert_eq!(db.get_navmeshes_for_cell("CELL_A").unwrap().len(), 1);
    assert_eq!(db.get_all_navmeshes().unwrap().len(), 1);
}

#[test]
fn an_unreadable_navmesh_row_is_skipped_without_losing_its_cell() {
    let conn = navmesh_connection();
    insert(
        &conn,
        "navigation_meshes",
        &navmesh_row("GOOD", 1, "CELL_A"),
    );
    insert(
        &conn,
        "navigation_meshes",
        &navmesh_row("BAD", -5, "CELL_A"),
    );
    insert(
        &conn,
        "navigation_meshes",
        &navmesh_row("ALSO_GOOD", 2, "CELL_A"),
    );
    let db = client(conn);
    let mut keys: Vec<String> = db
        .get_navmeshes_for_cell("CELL_A")
        .unwrap()
        .into_iter()
        .map(|row| row.form_key)
        .collect();
    keys.sort();
    assert_eq!(keys, ["ALSO_GOOD", "GOOD"]);
    assert_eq!(db.get_all_navmeshes().unwrap().len(), 2);
}

#[test]
fn all_navmeshes_span_every_cell() {
    let conn = navmesh_connection();
    insert(&conn, "navigation_meshes", &navmesh_row("N1", 1, "CELL_A"));
    insert(&conn, "navigation_meshes", &navmesh_row("N2", 2, "CELL_B"));
    assert_eq!(client(conn).get_all_navmeshes().unwrap().len(), 2);
}

#[test]
fn a_cell_without_navmeshes_yields_no_rows() {
    let conn = navmesh_connection();
    insert(&conn, "navigation_meshes", &navmesh_row("N1", 1, "CELL_A"));
    assert_eq!(client(conn).get_navmeshes_for_cell("CELL_Z").unwrap(), []);
}

#[test]
fn a_database_with_only_some_pipboy_perk_columns_is_a_schema_error() {
    let conn = legacy_connection(&[("perks", "swf_path")]);
    assert!(DbSchema::detect(&conn).is_err());
}

#[test]
fn the_catalog_of_a_stale_database_is_empty_and_not_an_error() {
    let catalog =
        openfo4::assets::pipboy_catalog::load_pipboy_catalog(&client(stale_pipboy_connection()))
            .unwrap();
    assert_eq!(catalog.perks, []);
    assert!(catalog.world_map.is_none());
    assert_eq!(catalog, openfo4::sim::pipboy::PipBoyCatalog::default());
}

#[test]
fn the_catalog_is_assembled_from_every_pipboy_table() {
    use openfo4::sim::pipboy::{InventoryCategory, MapMarkerId, PerkId, Special};

    let conn = schema_connection();
    insert(
        &conn,
        "perks",
        &perk_row(
            "P1",
            Some("Gun Nut"),
            Some("P2"),
            Some(5),
            Some("P1"),
            Some(1),
            Some(0),
            Some("G.swf"),
        ),
    );
    insert(
        &conn,
        "perks",
        &perk_row(
            "P2",
            Some("Gun Nut"),
            None,
            Some(13),
            Some("P1"),
            Some(1),
            Some(0),
            Some("G.swf"),
        ),
    );
    insert(
        &conn,
        "perk_chart",
        &[
            ("perk", text("P1")),
            ("special", text("intelligence")),
            ("required_special", int(3)),
        ],
    );
    insert(
        &conn,
        "items",
        &[
            ("form_key", text("S")),
            ("item_type", text("ALCH")),
            ("name", text("Stimpak")),
            ("value", int(48)),
            ("weight", real(0.5)),
        ],
    );
    insert(
        &conn,
        "worldspaces",
        &[
            ("form_key", text(COMMONWEALTH)),
            ("map_image", text("Interface\\Pip-Boy\\WorldMap_d.dds")),
        ],
    );
    insert(
        &conn,
        "map_markers",
        &marker_row("MK", COMMONWEALTH, 11, 0x03, (-80768.0, 88175.0, 0.0)),
    );

    insert(
        &conn,
        "actor_value_informations",
        &[("form_key", text("AV"))],
    );
    insert(
        &conn,
        "special_attributes",
        &[
            ("special", text("intelligence")),
            ("form_key", text("AV")),
            ("name", text("Intelligence")),
            ("description", text("Smarts.")),
        ],
    );

    let catalog = openfo4::assets::pipboy_catalog::load_pipboy_catalog(&client(conn)).unwrap();

    assert_eq!(
        catalog.special_texts.get(Special::Intelligence).description,
        "Smarts."
    );
    assert_eq!(catalog.perks.len(), 1);
    let gun_nut = &catalog.perks[0];
    assert_eq!(gun_nut.id, PerkId("P1".to_owned()));
    assert_eq!(gun_nut.ranks.len(), 2);
    assert_eq!(gun_nut.ranks[1].required_level, 13);
    assert_eq!(gun_nut.vault_boy_swf.as_deref(), Some("G.swf"));
    let slot = gun_nut.chart.unwrap();
    assert_eq!(
        (slot.special, slot.required_special),
        (Special::Intelligence, 3)
    );

    assert_eq!(catalog.items.len(), 1);
    assert_eq!(catalog.items[0].category, InventoryCategory::Aid);
    assert_eq!(catalog.items[0].value, 48);

    let map = catalog.world_map.unwrap();
    assert_eq!(
        map.texture_key,
        "textures\\Interface\\Pip-Boy\\WorldMap_d.dds"
    );
    assert_eq!(map.markers.len(), 1);
    let marker = &map.markers[0];
    assert_eq!(marker.id, MapMarkerId("MK".to_owned()));
    assert!(marker.discovered && marker.can_fast_travel);
    assert!((marker.position.x - -8076.8).abs() < 0.01);
    assert!((marker.position.z - -8817.5).abs() < 0.01);
}

#[test]
fn a_commonwealth_without_a_map_image_yields_no_world_map() {
    let conn = schema_connection();
    insert(&conn, "worldspaces", &[("form_key", text(COMMONWEALTH))]);
    insert(
        &conn,
        "map_markers",
        &marker_row("MK", COMMONWEALTH, 11, 0x03, (0.0, 0.0, 0.0)),
    );
    let catalog = openfo4::assets::pipboy_catalog::load_pipboy_catalog(&client(conn)).unwrap();
    assert!(catalog.world_map.is_none());
}

#[test]
fn databases_without_navmesh_geometry_columns_load_no_navmeshes() {
    let conn = schema_connection();
    conn.execute_batch("DROP TABLE navigation_meshes;").unwrap();
    conn.execute_batch(
        "CREATE TABLE navigation_meshes (form_key TEXT PRIMARY KEY, editor_id TEXT);",
    )
    .unwrap();
    insert(&conn, "navigation_meshes", &[("form_key", text("OLD"))]);
    let db = client(conn);
    assert!(!db.schema().navmesh_columns_present);
    assert_eq!(db.get_navmeshes_for_cell("CELL_A").unwrap(), []);
    assert_eq!(db.get_all_navmeshes().unwrap(), []);
}

#[test]
fn a_database_with_only_some_navmesh_columns_is_rejected() {
    let conn = schema_connection();
    conn.execute_batch("DROP TABLE navigation_meshes;").unwrap();
    conn.execute_batch(
        "CREATE TABLE navigation_meshes (form_key TEXT PRIMARY KEY, editor_id TEXT, form_id INTEGER);",
    )
    .unwrap();
    assert!(DbClient::from_connection(conn).is_err());
}

fn seed_collision_layer(
    conn: &Connection,
    form_key: &str,
    editor_id: &str,
    index: i64,
    flags: i64,
    collides_with: Value,
) {
    insert(
        conn,
        "collision_layers",
        &[
            ("form_key", text(form_key)),
            ("editor_id", text(editor_id)),
            ("layer_index", int(index)),
            ("flags", int(flags)),
            ("collides_with", collides_with),
        ],
    );
}

fn seed_character_layers(conn: &Connection) {
    seed_collision_layer(
        conn,
        "08877F:Fallout4.esm",
        "L_CHARCONTROLLER",
        30,
        0,
        text(r#"["0D8121:Fallout4.esm"]"#),
    );
    seed_collision_layer(
        conn,
        "0D8121:Fallout4.esm",
        "L_CHARBUMPER",
        56,
        0,
        text(r#"["08877F:Fallout4.esm"]"#),
    );
}

#[test]
fn player_collision_layers_resolve_from_the_stored_rows() {
    use openfo4::assets::db::collision_layers::LayerSolidity;
    let conn = schema_connection();
    seed_character_layers(&conn);
    seed_collision_layer(
        &conn,
        "000001:Fallout4.esm",
        "L_STATIC",
        1,
        0,
        text(r#"["08877F:Fallout4.esm"]"#),
    );
    seed_collision_layer(&conn, "000002:Fallout4.esm", "L_NAVCUT", 49, 5, Value::Null);
    seed_collision_layer(
        &conn,
        "000003:Fallout4.esm",
        "L_NONCOLLIDABLE",
        15,
        0,
        text(r#"["000001:Fallout4.esm"]"#),
    );
    let layers = client(conn).get_player_collision_layers().unwrap();
    assert_eq!(layers.solidity(Some(1)), LayerSolidity::Solid);
    assert_eq!(layers.solidity(Some(49)), LayerSolidity::NeverTouchesPlayer);
    assert_eq!(layers.solidity(Some(15)), LayerSolidity::NeverTouchesPlayer);
    assert_eq!(layers.solidity(Some(99)), LayerSolidity::SolidUnknown);
}

#[test]
fn player_collision_layers_keep_everything_solid_on_a_database_from_an_older_installer() {
    use openfo4::assets::db::collision_layers::LayerSolidity;
    let conn = legacy_connection(&[
        ("collision_layers", "layer_index"),
        ("collision_layers", "flags"),
        ("collision_layers", "collides_with"),
    ]);
    let layers = client(conn).get_player_collision_layers().unwrap();
    assert_eq!(layers.solidity(Some(49)), LayerSolidity::Solid);
}

#[test]
fn player_collision_layers_keep_everything_solid_without_the_character_layers() {
    use openfo4::assets::db::collision_layers::LayerSolidity;
    let conn = schema_connection();
    seed_collision_layer(&conn, "000002:Fallout4.esm", "L_NAVCUT", 49, 5, Value::Null);
    let layers = client(conn).get_player_collision_layers().unwrap();
    assert_eq!(layers.solidity(Some(49)), LayerSolidity::Solid);
}

#[test]
fn a_malformed_collides_with_list_is_an_error() {
    let conn = schema_connection();
    seed_character_layers(&conn);
    seed_collision_layer(
        &conn,
        "000004:Fallout4.esm",
        "L_BAD",
        7,
        0,
        text("not json"),
    );
    let error = client(conn).get_player_collision_layers().unwrap_err();
    assert!(
        format!("{error:#}").contains("000004:Fallout4.esm"),
        "{error:#}"
    );
}

// ---------------------------------------------------- globals and game settings

fn seed_global(conn: &Connection, editor_id: &str, value: Value) {
    insert(
        conn,
        "globals",
        &[
            ("form_key", text(&format!("{editor_id}:Fallout4.esm"))),
            ("editor_id", text(editor_id)),
            ("value_type", text("float")),
            ("value", value),
        ],
    );
}

fn seed_game_setting(
    conn: &Connection,
    editor_id: &str,
    value_type: &str,
    (value_int, value_float): (Value, Value),
) {
    insert(
        conn,
        "game_settings",
        &[
            ("form_key", text(&format!("{editor_id}:Fallout4.esm"))),
            ("editor_id", text(editor_id)),
            ("value_type", text(value_type)),
            ("value_int", value_int),
            ("value_float", value_float),
        ],
    );
}

#[test]
fn a_global_is_read_by_editor_id() {
    let conn = schema_connection();
    seed_global(&conn, "TimeScale", real(20.0));
    seed_global(&conn, "GameHour", real(11.5));
    let db = client(conn);
    assert_eq!(db.get_global("TimeScale").unwrap(), Some(20.0));
    assert_eq!(db.get_global("GameHour").unwrap(), Some(11.5));
}

#[test]
fn an_unknown_global_is_absent_rather_than_an_error() {
    let db = client(schema_connection());
    assert_eq!(db.get_global("TimeScale").unwrap(), None);
}

#[test]
fn a_global_without_a_value_is_an_error() {
    let conn = schema_connection();
    seed_global(&conn, "TimeScale", Value::Null);
    assert!(client(conn).get_global("TimeScale").is_err());
}

#[test]
fn a_global_query_fails_when_the_table_is_missing() {
    let conn = schema_connection();
    conn.execute_batch("DROP TABLE globals;").unwrap();
    assert!(client(conn).get_global("TimeScale").is_err());
}

#[test]
fn float_and_int_game_settings_read_as_numbers() {
    let conn = schema_connection();
    seed_game_setting(&conn, "fSunXExtreme", "float", (Value::Null, real(600.0)));
    seed_game_setting(&conn, "iMasserSize", "int", (int(90), Value::Null));
    let db = client(conn);
    assert_eq!(
        db.get_game_setting_number("fSunXExtreme").unwrap(),
        Some(600.0)
    );
    assert_eq!(
        db.get_game_setting_number("iMasserSize").unwrap(),
        Some(90.0)
    );
}

#[test]
fn an_unknown_game_setting_is_absent_rather_than_an_error() {
    let db = client(schema_connection());
    assert_eq!(db.get_game_setting_number("fSunXExtreme").unwrap(), None);
}

#[test]
fn a_non_numeric_game_setting_is_an_error_naming_it() {
    let conn = schema_connection();
    seed_game_setting(
        &conn,
        "sMasserTexture",
        "string",
        (Value::Null, Value::Null),
    );
    seed_game_setting(&conn, "bFlag", "bool", (int(1), Value::Null));
    let db = client(conn);
    for name in ["sMasserTexture", "bFlag"] {
        let error = db.get_game_setting_number(name).unwrap_err();
        assert!(format!("{error:#}").contains(name), "{error:#}");
    }
}

#[test]
fn a_numeric_game_setting_without_a_value_is_an_error_naming_it() {
    let conn = schema_connection();
    seed_game_setting(&conn, "fSunXExtreme", "float", (Value::Null, Value::Null));
    let error = client(conn)
        .get_game_setting_number("fSunXExtreme")
        .unwrap_err();
    assert!(format!("{error:#}").contains("fSunXExtreme"), "{error:#}");
}
