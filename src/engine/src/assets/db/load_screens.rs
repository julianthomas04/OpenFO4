//! Loading screens (LSCR) as plain data: the model to show, how to place it, the tip text and the
//! conditions that decide where the screen may appear, plus the location data those conditions read.
//! Values are raw Bethesda units; choosing a screen belongs to `crate::loading_screen`.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::assets::db::conditions::{Condition, load_conditions, load_globals};
use crate::assets::db::rows::{collect_rows, narrow, real32, required};

/// LSCR record flag bit 10.
const FLAG_DISPLAYS_IN_MAIN_MENU: u32 = 1 << 10;
/// LSCR record flag bit 15.
const FLAG_NO_ROTATION: u32 = 1 << 15;
/// TRNS flag index 16 as xEdit defines it. Mutagen's `Transform.MajorFlag.AroundOrigin` says 0x8000,
/// which matches neither xEdit nor the vanilla records.
const TRANSFORM_FLAG_AROUND_ORIGIN: u32 = 1 << 16;

/// A TRNS record: where the screen's model sits relative to the loading screen camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelTransform {
    pub position: [f32; 3],
    pub rotation_radians: [f32; 3],
    pub scale: f32,
    pub around_origin: bool,
    pub zoom_min: f32,
    pub zoom_max: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoadScreen {
    pub form_key: String,
    pub editor_id: Option<String>,
    pub tip: Option<String>,
    /// The model path of the STAT or SCOL the screen shows, as stored by the installer.
    pub model_path: String,
    pub displays_in_main_menu: bool,
    pub no_rotation: bool,
    pub transform: ModelTransform,
    /// The ONAM rotation limits in degrees.
    pub rotation_limits: Option<(i16, i16)>,
    /// The ZNAM zoom limits.
    pub zoom_limits: Option<(f32, f32)>,
    pub conditions: Vec<Condition>,
}

impl LoadScreen {
    /// The editor id where the screen has one, else its form key; for messages.
    pub fn label(&self) -> &str {
        self.editor_id.as_deref().unwrap_or(&self.form_key)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoadScreenCatalog {
    pub screens: Vec<LoadScreen>,
    /// Cell form key to the location (XLCN) it names; cells naming none are absent.
    pub cell_locations: HashMap<String, String>,
    /// Location form key to its parent location; root locations are absent.
    pub location_parents: HashMap<String, String>,
    pub globals: HashMap<String, f32>,
    /// Global editor id to form key.
    pub global_form_keys: HashMap<String, String>,
}

impl LoadScreenCatalog {
    /// Loads everything once; errors when a row is malformed or incomplete, such as a stub screen
    /// the installer migrated but has not filled yet. Callers check
    /// `DbSchema::load_screen_columns_present` first.
    pub(crate) fn from_connection(conn: &Connection) -> anyhow::Result<Self> {
        let (globals, global_form_keys) = load_globals(conn)?;
        Ok(Self {
            screens: load_screens(conn)?,
            cell_locations: load_cell_locations(conn)?,
            location_parents: load_location_parents(conn)?,
            globals,
            global_form_keys,
        })
    }
}

/// What the joins found for the screen's model form key.
enum ModelSource {
    Static(Option<String>),
    Collection(Option<String>),
    Both,
    Neither,
}

struct ScreenRow {
    form_key: String,
    editor_id: Option<String>,
    tip: Option<String>,
    model: String,
    flags: i64,
    transform: Option<String>,
    rotation_min: Option<i64>,
    rotation_max: Option<i64>,
    zoom_min: Option<f64>,
    zoom_max: Option<f64>,
    source: ModelSource,
}

impl ScreenRow {
    fn name(&self) -> String {
        match &self.editor_id {
            Some(editor_id) => format!("load screen {} ({editor_id})", self.form_key),
            None => format!("load screen {}", self.form_key),
        }
    }
}

fn load_screens(conn: &Connection) -> anyhow::Result<Vec<LoadScreen>> {
    let transforms = load_transforms(conn)?;
    let mut conditions = load_conditions(conn, "load_screen_conditions", "screen", "load screen")?;
    let rows = collect_rows(
        conn,
        "SELECT l.form_key, l.editor_id, l.tip, l.model, l.flags, l.transform, l.rotation_min, \
         l.rotation_max, l.zoom_min, l.zoom_max, s.form_key IS NOT NULL, s.model_path, \
         c.form_key IS NOT NULL, c.model_path \
         FROM load_screens l \
         LEFT JOIN statics s ON s.form_key = l.model \
         LEFT JOIN static_collections c ON c.form_key = l.model \
         ORDER BY l.form_key",
        |row| {
            let is_static: bool = row.get(10)?;
            let is_collection: bool = row.get(12)?;
            let source = match (is_static, is_collection) {
                (true, false) => ModelSource::Static(row.get(11)?),
                (false, true) => ModelSource::Collection(row.get(13)?),
                (true, true) => ModelSource::Both,
                (false, false) => ModelSource::Neither,
            };
            Ok(ScreenRow {
                form_key: row.get(0)?,
                editor_id: row.get(1)?,
                tip: row.get(2)?,
                model: row.get(3)?,
                flags: row.get(4)?,
                transform: row.get(5)?,
                rotation_min: row.get(6)?,
                rotation_max: row.get(7)?,
                zoom_min: row.get(8)?,
                zoom_max: row.get(9)?,
                source,
            })
        },
    )?;
    rows.into_iter()
        .map(|row| {
            let screen_conditions = conditions.remove(&row.form_key).unwrap_or_default();
            build_screen(row, &transforms, screen_conditions)
        })
        .collect()
}

fn build_screen(
    row: ScreenRow,
    transforms: &HashMap<String, ModelTransform>,
    conditions: Vec<Condition>,
) -> anyhow::Result<LoadScreen> {
    let name = row.name();
    anyhow::ensure!(!row.model.is_empty(), "{name}: model is empty");
    let transform_key = required(row.transform.as_deref(), "transform", &name)?;
    let transform = *transforms.get(transform_key).ok_or_else(|| {
        anyhow::anyhow!("{name}: transform {transform_key} has no row in transforms")
    })?;
    let model_path = match row.source {
        ModelSource::Static(path) | ModelSource::Collection(path) => path
            .filter(|path| !path.is_empty())
            .ok_or_else(|| anyhow::anyhow!("{name}: model {} has no model path", row.model))?,
        ModelSource::Neither => anyhow::bail!(
            "{name}: model {} is neither a static nor a static collection",
            row.model
        ),
        ModelSource::Both => anyhow::bail!(
            "{name}: model {} is both a static and a static collection",
            row.model
        ),
    };
    let flags: u32 = narrow(row.flags, "flags", &name)?;
    let rotation_limits = both_or_neither(
        row.rotation_min,
        row.rotation_max,
        ("rotation_min", "rotation_max"),
        &name,
    )?
    .map(|(min, max)| {
        Ok::<_, anyhow::Error>((
            narrow(min, "rotation_min", &name)?,
            narrow(max, "rotation_max", &name)?,
        ))
    })
    .transpose()?;
    let zoom_limits = both_or_neither(row.zoom_min, row.zoom_max, ("zoom_min", "zoom_max"), &name)?
        .map(|(min, max)| (real32(min), real32(max)));
    Ok(LoadScreen {
        form_key: row.form_key,
        editor_id: row.editor_id,
        tip: row.tip,
        model_path,
        displays_in_main_menu: flags & FLAG_DISPLAYS_IN_MAIN_MENU != 0,
        no_rotation: flags & FLAG_NO_ROTATION != 0,
        transform,
        rotation_limits,
        zoom_limits,
        conditions,
    })
}

fn both_or_neither<T>(
    first: Option<T>,
    second: Option<T>,
    columns: (&str, &str),
    row: &str,
) -> anyhow::Result<Option<(T, T)>> {
    match (first, second) {
        (Some(first), Some(second)) => Ok(Some((first, second))),
        (None, None) => Ok(None),
        _ => anyhow::bail!("{row}: only one of {} and {} is set", columns.0, columns.1),
    }
}

struct TransformRow {
    form_key: String,
    position: [Option<f64>; 3],
    rotation: [Option<f64>; 3],
    scale: Option<f64>,
    flags: Option<i64>,
    zoom_min: Option<f64>,
    zoom_max: Option<f64>,
}

fn load_transforms(conn: &Connection) -> anyhow::Result<HashMap<String, ModelTransform>> {
    let rows = collect_rows(
        conn,
        "SELECT form_key, pos_x, pos_y, pos_z, rot_x, rot_y, rot_z, scale, flags, zoom_min, zoom_max \
         FROM transforms",
        |row| {
            Ok(TransformRow {
                form_key: row.get(0)?,
                position: [row.get(1)?, row.get(2)?, row.get(3)?],
                rotation: [row.get(4)?, row.get(5)?, row.get(6)?],
                scale: row.get(7)?,
                flags: row.get(8)?,
                zoom_min: row.get(9)?,
                zoom_max: row.get(10)?,
            })
        },
    )?;
    rows.into_iter()
        .map(|row| {
            let name = format!("transform {}", row.form_key);
            let flags: u32 = narrow(required(row.flags, "flags", &name)?, "flags", &name)?;
            let transform = ModelTransform {
                position: vector(row.position, ["pos_x", "pos_y", "pos_z"], &name)?,
                rotation_radians: vector(row.rotation, ["rot_x", "rot_y", "rot_z"], &name)?,
                scale: real32(required(row.scale, "scale", &name)?),
                around_origin: flags & TRANSFORM_FLAG_AROUND_ORIGIN != 0,
                zoom_min: real32(required(row.zoom_min, "zoom_min", &name)?),
                zoom_max: real32(required(row.zoom_max, "zoom_max", &name)?),
            };
            Ok((row.form_key, transform))
        })
        .collect()
}

fn vector(values: [Option<f64>; 3], columns: [&str; 3], row: &str) -> anyhow::Result<[f32; 3]> {
    let [x, y, z] = values;
    let [x_column, y_column, z_column] = columns;
    Ok([
        real32(required(x, x_column, row)?),
        real32(required(y, y_column, row)?),
        real32(required(z, z_column, row)?),
    ])
}

/// The one source of where a cell stands: the location its record names. Exterior cells that only
/// the location records list will join here.
fn load_cell_locations(conn: &Connection) -> anyhow::Result<HashMap<String, String>> {
    Ok(collect_rows(
        conn,
        "SELECT cell, location FROM cell_places WHERE location IS NOT NULL",
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?
    .into_iter()
    .collect())
}

fn load_location_parents(conn: &Connection) -> anyhow::Result<HashMap<String, String>> {
    Ok(collect_rows(
        conn,
        "SELECT form_key, parent_location FROM locations WHERE parent_location IS NOT NULL",
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?
    .into_iter()
    .collect())
}

#[cfg(test)]
mod tests {
    use rusqlite::types::Value;

    use super::*;
    use crate::assets::db::conditions::{Comparand, CompareOp};

    const SCHEMA: &str = include_str!("../../../../installer/file_converters/esm_schema.sql");

    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
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

    fn text(s: &str) -> Value {
        Value::Text(s.to_string())
    }

    fn int(n: i64) -> Value {
        Value::Integer(n)
    }

    fn real(x: f64) -> Value {
        Value::Real(x)
    }

    fn transform_row(key: &str, flags: i64) -> Vec<(&'static str, Value)> {
        vec![
            ("form_key", text(key)),
            ("pos_x", real(1.0)),
            ("pos_y", real(2.0)),
            ("pos_z", real(3.0)),
            ("rot_x", real(0.5)),
            ("rot_y", real(0.25)),
            ("rot_z", real(1.5)),
            ("scale", real(2.0)),
            ("flags", int(flags)),
            ("zoom_min", real(-0.5)),
            ("zoom_max", real(1.5)),
        ]
    }

    fn screen_row(key: &str, model: &str, transform: Option<&str>) -> Vec<(&'static str, Value)> {
        vec![
            ("form_key", text(key)),
            ("editor_id", text(&format!("LS_{key}"))),
            ("model", text(model)),
            ("flags", int(0)),
            ("transform", transform.map_or(Value::Null, text)),
        ]
    }

    fn static_model(conn: &Connection, key: &str, path: Option<&str>) {
        insert(
            conn,
            "statics",
            &[
                ("form_key", text(key)),
                ("model_path", path.map_or(Value::Null, text)),
            ],
        );
    }

    fn collection_model(conn: &Connection, key: &str, path: &str) {
        insert(
            conn,
            "static_collections",
            &[("form_key", text(key)), ("model_path", text(path))],
        );
    }

    fn world_with_one_screen() -> Connection {
        let conn = connection();
        static_model(&conn, "STAT", Some(r"Interface\LoadScreen\Truck.nif"));
        insert(&conn, "transforms", &transform_row("TRNS", 0));
        insert(
            &conn,
            "load_screens",
            &screen_row("LSCR", "STAT", Some("TRNS")),
        );
        conn
    }

    fn only_screen(conn: &Connection) -> LoadScreen {
        let mut screens = LoadScreenCatalog::from_connection(conn).unwrap().screens;
        assert_eq!(screens.len(), 1);
        screens.remove(0)
    }

    fn error_text(conn: &Connection) -> String {
        format!(
            "{:#}",
            LoadScreenCatalog::from_connection(conn).unwrap_err()
        )
    }

    #[test]
    fn a_screen_with_a_static_model_loads_every_field() {
        let conn = world_with_one_screen();
        conn.execute_batch(
            "UPDATE load_screens SET tip = 'Mind the gap', rotation_min = -90, rotation_max = 45, \
             zoom_min = 0.5, zoom_max = 2.5",
        )
        .unwrap();

        let screen = only_screen(&conn);

        assert_eq!(screen.form_key, "LSCR");
        assert_eq!(screen.editor_id.as_deref(), Some("LS_LSCR"));
        assert_eq!(screen.tip.as_deref(), Some("Mind the gap"));
        assert_eq!(screen.model_path, r"Interface\LoadScreen\Truck.nif");
        assert_eq!(screen.rotation_limits, Some((-90, 45)));
        assert_eq!(screen.zoom_limits, Some((0.5, 2.5)));
        assert_eq!(
            screen.transform,
            ModelTransform {
                position: [1.0, 2.0, 3.0],
                rotation_radians: [0.5, 0.25, 1.5],
                scale: 2.0,
                around_origin: false,
                zoom_min: -0.5,
                zoom_max: 1.5,
            }
        );
        assert_eq!(screen.conditions, vec![]);
    }

    #[test]
    fn absent_tip_and_limits_stay_absent() {
        let screen = only_screen(&world_with_one_screen());

        assert_eq!(
            (screen.tip, screen.rotation_limits, screen.zoom_limits),
            (None, None, None)
        );
    }

    #[test]
    fn a_screen_can_take_its_model_from_a_static_collection() {
        let conn = connection();
        collection_model(&conn, "SCOL", r"SCOL\Fallout4.esm\CM00001234.NIF");
        insert(&conn, "transforms", &transform_row("TRNS", 0));
        insert(
            &conn,
            "load_screens",
            &screen_row("LSCR", "SCOL", Some("TRNS")),
        );

        assert_eq!(
            only_screen(&conn).model_path,
            r"SCOL\Fallout4.esm\CM00001234.NIF"
        );
    }

    #[test]
    fn record_flags_are_split_into_main_menu_and_no_rotation() {
        let cases = [
            (0, false, false),
            (0x400, true, false),
            (0x8000, false, true),
            (0x8400, true, true),
            (0x4001_0000, false, false),
        ];
        for (flags, main_menu, no_rotation) in cases {
            let conn = world_with_one_screen();
            conn.execute(
                "UPDATE load_screens SET flags = ?1",
                rusqlite::params![flags],
            )
            .unwrap();

            let screen = only_screen(&conn);

            assert_eq!(
                (screen.displays_in_main_menu, screen.no_rotation),
                (main_menu, no_rotation),
                "flags {flags:#x}"
            );
        }
    }

    #[test]
    fn around_origin_is_bit_16_of_the_transform_flags_not_bit_15() {
        for (flags, around_origin) in [
            (0, false),
            (0x8000, false),
            (0x10000, true),
            (0x18000, true),
        ] {
            let conn = connection();
            static_model(&conn, "STAT", Some("a.nif"));
            insert(&conn, "transforms", &transform_row("TRNS", flags));
            insert(
                &conn,
                "load_screens",
                &screen_row("LSCR", "STAT", Some("TRNS")),
            );

            assert_eq!(
                only_screen(&conn).transform.around_origin,
                around_origin,
                "flags {flags:#x}"
            );
        }
    }

    #[test]
    fn conditions_are_attached_to_their_screen_in_order() {
        let conn = world_with_one_screen();
        for (ord, function, value) in [(1, 77, 20.0), (0, 310, 1.0)] {
            insert(
                &conn,
                "load_screen_conditions",
                &[
                    ("screen", text("LSCR")),
                    ("ord", int(ord)),
                    ("function", int(function)),
                    ("compare_op", int(5)),
                    ("or_flag", int(0)),
                    ("compare_value", real(value)),
                    ("run_on", int(0)),
                ],
            );
        }

        let conditions = only_screen(&conn).conditions;

        let loaded: Vec<(u16, CompareOp, Comparand)> = conditions
            .into_iter()
            .map(|c| (c.function, c.op, c.comparand))
            .collect();
        assert_eq!(
            loaded,
            vec![
                (310, CompareOp::LessOrEqual, Comparand::Value(1.0)),
                (77, CompareOp::LessOrEqual, Comparand::Value(20.0)),
            ]
        );
    }

    #[test]
    fn a_stub_screen_with_an_empty_model_is_rejected_by_naming_it() {
        let conn = connection();
        insert(&conn, "load_screens", &screen_row("LSCR", "", None));

        let message = error_text(&conn);

        assert!(
            message.contains("load screen LSCR (LS_LSCR)") && message.contains("model is empty"),
            "{message}"
        );
    }

    #[test]
    fn a_screen_without_a_transform_is_rejected_by_naming_it() {
        let conn = connection();
        static_model(&conn, "STAT", Some("a.nif"));
        insert(&conn, "load_screens", &screen_row("LSCR", "STAT", None));

        let message = error_text(&conn);

        assert!(
            message.contains("load screen LSCR (LS_LSCR)") && message.contains("transform is NULL"),
            "{message}"
        );
    }

    #[test]
    fn a_screen_whose_transform_row_is_missing_is_rejected() {
        let conn = connection();
        static_model(&conn, "STAT", Some("a.nif"));
        insert(
            &conn,
            "load_screens",
            &screen_row("LSCR", "STAT", Some("GHOST")),
        );

        let message = error_text(&conn);

        assert!(
            message.contains("LSCR") && message.contains("transform GHOST has no row"),
            "{message}"
        );
    }

    #[test]
    fn a_model_that_is_neither_static_nor_collection_is_rejected() {
        let conn = connection();
        insert(&conn, "transforms", &transform_row("TRNS", 0));
        insert(
            &conn,
            "load_screens",
            &screen_row("LSCR", "GHOST", Some("TRNS")),
        );

        let message = error_text(&conn);

        assert!(
            message.contains("neither a static nor a static collection"),
            "{message}"
        );
    }

    #[test]
    fn a_model_that_is_both_static_and_collection_is_rejected() {
        let conn = connection();
        static_model(&conn, "BOTH", Some("a.nif"));
        collection_model(&conn, "BOTH", "b.nif");
        insert(&conn, "transforms", &transform_row("TRNS", 0));
        insert(
            &conn,
            "load_screens",
            &screen_row("LSCR", "BOTH", Some("TRNS")),
        );

        let message = error_text(&conn);

        assert!(
            message.contains("both a static and a static collection"),
            "{message}"
        );
    }

    #[test]
    fn a_static_without_a_model_path_is_rejected() {
        let conn = connection();
        static_model(&conn, "STAT", None);
        insert(&conn, "transforms", &transform_row("TRNS", 0));
        insert(
            &conn,
            "load_screens",
            &screen_row("LSCR", "STAT", Some("TRNS")),
        );

        let message = error_text(&conn);

        assert!(
            message.contains("model STAT has no model path"),
            "{message}"
        );
    }

    #[test]
    fn a_transform_with_a_missing_value_is_rejected_by_naming_the_column() {
        let conn = connection();
        static_model(&conn, "STAT", Some("a.nif"));
        let mut row = transform_row("TRNS", 0);
        row.retain(|(name, _)| *name != "zoom_max");
        insert(&conn, "transforms", &row);
        insert(
            &conn,
            "load_screens",
            &screen_row("LSCR", "STAT", Some("TRNS")),
        );

        let message = error_text(&conn);

        assert!(
            message.contains("transform TRNS: zoom_max is NULL"),
            "{message}"
        );
    }

    #[test]
    fn half_a_limit_pair_is_rejected() {
        for (column, expected) in [
            ("rotation_min", "only one of rotation_min and rotation_max"),
            ("zoom_max", "only one of zoom_min and zoom_max"),
        ] {
            let conn = world_with_one_screen();
            conn.execute_batch(&format!("UPDATE load_screens SET {column} = 1"))
                .unwrap();

            let message = error_text(&conn);

            assert!(message.contains(expected), "{column}: {message}");
        }
    }

    #[test]
    fn a_rotation_limit_outside_sixteen_bits_is_rejected() {
        let conn = world_with_one_screen();
        conn.execute_batch("UPDATE load_screens SET rotation_min = 40000, rotation_max = 1")
            .unwrap();

        let message = error_text(&conn);

        assert!(
            message.contains("rotation_min 40000 is out of range"),
            "{message}"
        );
    }

    #[test]
    fn cell_locations_and_location_parents_are_loaded() {
        let conn = connection();
        for (cell, location) in [("CELL_A", Some("LOC_CHILD")), ("CELL_B", None)] {
            insert(&conn, "cells", &[("form_key", text(cell))]);
            insert(
                &conn,
                "cell_places",
                &[
                    ("cell", text(cell)),
                    ("music_type", text("MUS")),
                    ("location", location.map_or(Value::Null, text)),
                ],
            );
        }
        for (location, parent) in [("LOC_CHILD", Some("LOC_ROOT")), ("LOC_ROOT", None)] {
            insert(
                &conn,
                "locations",
                &[
                    ("form_key", text(location)),
                    ("parent_location", parent.map_or(Value::Null, text)),
                ],
            );
        }

        let catalog = LoadScreenCatalog::from_connection(&conn).unwrap();

        assert_eq!(
            catalog.cell_locations,
            HashMap::from([("CELL_A".to_string(), "LOC_CHILD".to_string())])
        );
        assert_eq!(
            catalog.location_parents,
            HashMap::from([("LOC_CHILD".to_string(), "LOC_ROOT".to_string())])
        );
    }

    #[test]
    fn cell_locations_do_not_require_the_music_tables_to_be_consistent() {
        let conn = connection();
        insert(
            &conn,
            "cell_places",
            &[("cell", text("NO_CELL_ROW")), ("location", text("LOC"))],
        );

        let catalog = LoadScreenCatalog::from_connection(&conn).unwrap();

        assert_eq!(
            catalog
                .cell_locations
                .get("NO_CELL_ROW")
                .map(String::as_str),
            Some("LOC")
        );
    }

    #[test]
    fn globals_are_loaded_with_their_editor_ids() {
        let conn = connection();
        insert(
            &conn,
            "globals",
            &[
                ("form_key", text("GLOB")),
                ("editor_id", text("LoadScreenPerkPinPercent01")),
                ("value", real(1.0)),
            ],
        );

        let catalog = LoadScreenCatalog::from_connection(&conn).unwrap();

        assert_eq!(catalog.globals.get("GLOB"), Some(&1.0));
        assert_eq!(
            catalog
                .global_form_keys
                .get("LoadScreenPerkPinPercent01")
                .map(String::as_str),
            Some("GLOB")
        );
    }
}
