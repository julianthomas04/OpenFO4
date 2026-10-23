//! Ambient music tables as plain data: music types, tracks, and the four
//! places that name a music type (cell, location, worldspace, region).
//! Values are raw Bethesda units; interpretation belongs to the audio logic.

use std::collections::{HashMap, HashSet};

use anyhow::Context;
use rusqlite::{Connection, Row};

use crate::assets::db::schema::require_music_tables;

#[derive(Debug, Clone, PartialEq)]
pub struct MusicType {
    pub form_key: String,
    pub editor_id: Option<String>,
    pub flags: u32,
    /// Lower number wins.
    pub priority: u8,
    pub ducking_centidb: u16,
    pub fade_seconds: f32,
    /// Track form keys in record order; a track may repeat.
    pub tracks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MusicTrack {
    pub form_key: String,
    pub editor_id: Option<String>,
    pub kind: MusicTrackKind,
    pub conditions: Vec<Condition>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MusicTrackKind {
    Single {
        file_key: String,
        duration: Option<f32>,
        fade_out: Option<f32>,
    },
    Silent {
        duration: f32,
    },
    Palette {
        /// Sub-track form keys per layer, each in record order.
        layers: Vec<Vec<String>>,
        duration: f32,
        fade_out: Option<f32>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Equal,
    NotEqual,
    Greater,
    GreaterOrEqual,
    Less,
    LessOrEqual,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Comparand {
    Value(f32),
    Global(String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConditionParam {
    pub num: Option<i64>,
    pub record: Option<String>,
    pub string: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    pub function: u16,
    pub op: CompareOp,
    /// True when this condition is OR-ed with the next one.
    pub or_flag: bool,
    pub comparand: Comparand,
    pub run_on: u8,
    pub reference: Option<String>,
    pub param1: ConditionParam,
    pub param2: ConditionParam,
    pub param3: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CellPlace {
    pub cell: String,
    pub music_type: Option<String>,
    pub location: Option<String>,
    pub worldspace: Option<String>,
    pub grid: Option<(i32, i32)>,
    pub is_interior: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegionMusic {
    pub region: String,
    /// `regions.worldspace`; a region without one can never match a place.
    pub worldspace: Option<String>,
    pub music_type: String,
    pub priority: u8,
    pub override_flag: bool,
    /// 0-based position of the REGN record among the plugin's REGN records.
    pub record_order: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegionArea {
    pub region: String,
    pub area: u32,
    /// Polygon vertices in raw Bethesda units.
    pub points: Vec<(f32, f32)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SoundCategory {
    pub form_key: String,
    pub editor_id: Option<String>,
    pub parent: Option<String>,
    /// The static volume multiplier, 0..1; `None` when the record has none.
    pub static_multiplier: Option<f32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MusicCatalog {
    pub music_types: Vec<MusicType>,
    pub tracks: Vec<MusicTrack>,
    /// Only the cells that name a music type or a location.
    pub cell_places: Vec<CellPlace>,
    pub location_music: HashMap<String, String>,
    pub worldspace_music: HashMap<String, String>,
    pub region_music: Vec<RegionMusic>,
    pub region_areas: Vec<RegionArea>,
    pub sound_categories: Vec<SoundCategory>,
    pub globals: HashMap<String, f32>,
    /// Global editor id to form key.
    pub global_form_keys: HashMap<String, String>,
}

impl MusicCatalog {
    /// Loads everything once; errors when the installer has not extracted the music tables or a row is malformed.
    pub(crate) fn from_connection(conn: &Connection) -> anyhow::Result<Self> {
        require_music_tables(conn)?;
        let (globals, global_form_keys) = load_globals(conn)?;
        let catalog = Self {
            music_types: load_music_types(conn)?,
            tracks: load_tracks(conn)?,
            cell_places: load_cell_places(conn)?,
            location_music: load_key_to_music(conn, "location_music", "location")?,
            worldspace_music: load_key_to_music(conn, "worldspace_music", "worldspace")?,
            region_music: load_region_music(conn)?,
            region_areas: load_region_areas(conn)?,
            sound_categories: load_sound_categories(conn)?,
            globals,
            global_form_keys,
        };
        catalog.check_references()?;
        Ok(catalog)
    }

    fn check_references(&self) -> anyhow::Result<()> {
        let types: HashSet<&str> = self
            .music_types
            .iter()
            .map(|t| t.form_key.as_str())
            .collect();
        let tracks: HashSet<&str> = self.tracks.iter().map(|t| t.form_key.as_str()).collect();
        let known_type = |key: &str, owner: &str| {
            anyhow::ensure!(
                types.contains(key),
                "{owner} names unknown music type {key}"
            );
            Ok(())
        };
        let known_track = |key: &str, owner: &str| {
            anyhow::ensure!(
                tracks.contains(key),
                "{owner} names unknown music track {key}"
            );
            Ok(())
        };
        for music_type in &self.music_types {
            for track in &music_type.tracks {
                known_track(track, &format!("music type {}", music_type.form_key))?;
            }
        }
        for track in &self.tracks {
            if let MusicTrackKind::Palette { layers, .. } = &track.kind {
                for sub_track in layers.iter().flatten() {
                    known_track(sub_track, &format!("palette track {}", track.form_key))?;
                }
            }
        }
        for place in &self.cell_places {
            if let Some(key) = &place.music_type {
                known_type(key, &format!("cell {}", place.cell))?;
            }
        }
        for (location, key) in &self.location_music {
            known_type(key, &format!("location {location}"))?;
        }
        for (worldspace, key) in &self.worldspace_music {
            known_type(key, &format!("worldspace {worldspace}"))?;
        }
        for entry in &self.region_music {
            known_type(&entry.music_type, &format!("region {}", entry.region))?;
        }
        Ok(())
    }
}

pub(super) fn narrow<T: TryFrom<i64>>(value: i64, column: &str, row: &str) -> anyhow::Result<T> {
    T::try_from(value).map_err(|_| anyhow::anyhow!("{row}: {column} {value} is out of range"))
}

pub(super) fn required<T>(value: Option<T>, column: &str, row: &str) -> anyhow::Result<T> {
    value.ok_or_else(|| anyhow::anyhow!("{row}: {column} is NULL"))
}

pub(super) fn flag(value: i64, column: &str, row: &str) -> anyhow::Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        other => anyhow::bail!("{row}: {column} {other} is not 0 or 1"),
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the database stores f32 values widened to REAL"
)]
pub(super) fn real32(value: f64) -> f32 {
    value as f32
}

pub(super) fn collect_rows<T>(
    conn: &Connection,
    sql: &str,
    map: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
) -> anyhow::Result<Vec<T>> {
    let mut stmt = conn
        .prepare(sql)
        .with_context(|| format!("preparing `{sql}`"))?;
    let rows = stmt
        .query_map([], map)?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("reading `{sql}`"))?;
    Ok(rows)
}

fn load_music_types(conn: &Connection) -> anyhow::Result<Vec<MusicType>> {
    let mut tracks: HashMap<String, Vec<String>> = HashMap::new();
    for (music_type, track) in collect_rows(
        conn,
        "SELECT music_type, track FROM music_type_tracks ORDER BY music_type, ord",
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )? {
        tracks.entry(music_type).or_default().push(track);
    }
    let rows = collect_rows(
        conn,
        "SELECT form_key, editor_id, flags, priority, ducking_centidb, fade_seconds \
         FROM music_types ORDER BY form_key",
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<f64>>(5)?,
            ))
        },
    )?;
    rows.into_iter()
        .map(|(form_key, editor_id, flags, priority, ducking, fade)| {
            let row = format!("music type {form_key}");
            Ok(MusicType {
                flags: narrow(required(flags, "flags", &row)?, "flags", &row)?,
                priority: narrow(required(priority, "priority", &row)?, "priority", &row)?,
                ducking_centidb: narrow(
                    required(ducking, "ducking_centidb", &row)?,
                    "ducking_centidb",
                    &row,
                )?,
                fade_seconds: real32(required(fade, "fade_seconds", &row)?),
                tracks: tracks.remove(&form_key).unwrap_or_default(),
                form_key,
                editor_id,
            })
        })
        .collect()
}

struct TrackRow {
    form_key: String,
    editor_id: Option<String>,
    track_type: Option<String>,
    duration: Option<f64>,
    fade_out: Option<f64>,
    file_key: Option<String>,
}

fn load_tracks(conn: &Connection) -> anyhow::Result<Vec<MusicTrack>> {
    let mut layers = load_layers(conn)?;
    let mut conditions = load_conditions(conn, "music_track_conditions", "track", "music track")?;
    let rows = collect_rows(
        conn,
        "SELECT form_key, editor_id, track_type, duration, fade_out, file_key \
         FROM music_tracks ORDER BY form_key",
        |row| {
            Ok(TrackRow {
                form_key: row.get(0)?,
                editor_id: row.get(1)?,
                track_type: row.get(2)?,
                duration: row.get(3)?,
                fade_out: row.get(4)?,
                file_key: row.get(5)?,
            })
        },
    )?;
    rows.into_iter()
        .map(|row| {
            let name = format!("music track {}", row.form_key);
            let duration = row.duration.map(real32);
            let fade_out = row.fade_out.map(real32);
            let kind = match row.track_type.as_deref() {
                Some("Single") => MusicTrackKind::Single {
                    file_key: required(row.file_key, "file_key", &name)?,
                    duration,
                    fade_out,
                },
                Some("Silent") => MusicTrackKind::Silent {
                    duration: required(duration, "duration", &name)?,
                },
                Some("Palette") => MusicTrackKind::Palette {
                    layers: layers.remove(&row.form_key).unwrap_or_default(),
                    duration: required(duration, "duration", &name)?,
                    fade_out,
                },
                Some(other) => anyhow::bail!("{name}: unknown track_type `{other}`"),
                None => anyhow::bail!("{name}: track_type is NULL"),
            };
            Ok(MusicTrack {
                conditions: conditions.remove(&row.form_key).unwrap_or_default(),
                form_key: row.form_key,
                editor_id: row.editor_id,
                kind,
            })
        })
        .collect()
}

/// Layers per palette track, each in record order; a gap in the layer numbering is bad data.
fn load_layers(conn: &Connection) -> anyhow::Result<HashMap<String, Vec<Vec<String>>>> {
    let rows = collect_rows(
        conn,
        "SELECT track, layer, sub_track FROM music_track_layers ORDER BY track, layer, ord",
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        },
    )?;
    let mut layers: HashMap<String, Vec<Vec<String>>> = HashMap::new();
    for (track, layer, sub_track) in rows {
        let row = format!("music track {track} layers");
        let layer: usize = narrow(layer, "layer", &row)?;
        let track_layers = layers.entry(track).or_default();
        anyhow::ensure!(
            layer <= track_layers.len(),
            "{row}: layer {layer} leaves a gap after {} layers",
            track_layers.len()
        );
        if layer == track_layers.len() {
            track_layers.push(Vec::new());
        }
        track_layers[layer].push(sub_track);
    }
    Ok(layers)
}

struct ConditionRow {
    track: String,
    ord: i64,
    function: i64,
    compare_op: i64,
    or_flag: i64,
    compare_value: Option<f64>,
    compare_global: Option<String>,
    run_on: i64,
    reference: Option<String>,
    param1: ConditionParam,
    param2: ConditionParam,
    param3: Option<i64>,
}

pub(super) fn load_conditions(
    conn: &Connection,
    table: &str,
    owner_column: &str,
    owner_label: &str,
) -> anyhow::Result<HashMap<String, Vec<Condition>>> {
    let rows = collect_rows(
        conn,
        &format!(
            "SELECT {owner_column}, ord, function, compare_op, or_flag, compare_value, compare_global, run_on, \
         reference, param1_num, param1_record, param1_string, param2_num, param2_record, \
         param2_string, param3 FROM {table} ORDER BY {owner_column}, ord"
        ),
        |row| {
            Ok(ConditionRow {
                track: row.get(0)?,
                ord: row.get(1)?,
                function: row.get(2)?,
                compare_op: row.get(3)?,
                or_flag: row.get(4)?,
                compare_value: row.get(5)?,
                compare_global: row.get(6)?,
                run_on: row.get(7)?,
                reference: row.get(8)?,
                param1: ConditionParam {
                    num: row.get(9)?,
                    record: row.get(10)?,
                    string: row.get(11)?,
                },
                param2: ConditionParam {
                    num: row.get(12)?,
                    record: row.get(13)?,
                    string: row.get(14)?,
                },
                param3: row.get(15)?,
            })
        },
    )?;
    let mut grouped: HashMap<String, Vec<Condition>> = HashMap::new();
    for row in rows {
        let name = format!("{owner_label} {} condition {}", row.track, row.ord);
        let comparand = match (row.compare_value, row.compare_global) {
            (Some(value), None) => Comparand::Value(real32(value)),
            (None, Some(global)) => Comparand::Global(global),
            (Some(_), Some(_)) => {
                anyhow::bail!("{name}: both compare_value and compare_global are set")
            }
            (None, None) => {
                anyhow::bail!("{name}: neither compare_value nor compare_global is set")
            }
        };
        let condition = Condition {
            function: narrow(row.function, "function", &name)?,
            op: compare_op(row.compare_op, &name)?,
            or_flag: flag(row.or_flag, "or_flag", &name)?,
            comparand,
            run_on: narrow(row.run_on, "run_on", &name)?,
            reference: row.reference,
            param1: row.param1,
            param2: row.param2,
            param3: row.param3,
        };
        grouped.entry(row.track).or_default().push(condition);
    }
    Ok(grouped)
}

fn compare_op(value: i64, row: &str) -> anyhow::Result<CompareOp> {
    Ok(match value {
        0 => CompareOp::Equal,
        1 => CompareOp::NotEqual,
        2 => CompareOp::Greater,
        3 => CompareOp::GreaterOrEqual,
        4 => CompareOp::Less,
        5 => CompareOp::LessOrEqual,
        other => anyhow::bail!("{row}: unknown compare_op {other}"),
    })
}

type CellPlaceRow = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    i64,
);

fn load_cell_places(conn: &Connection) -> anyhow::Result<Vec<CellPlace>> {
    let rows: Vec<CellPlaceRow> = collect_rows(
        conn,
        "SELECT p.cell, p.music_type, p.location, c.form_key, c.worldspace, c.grid_x, c.grid_y, \
         COALESCE(c.is_interior, 0) FROM cell_places p \
         LEFT JOIN cells c ON c.form_key = p.cell ORDER BY p.cell",
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
            ))
        },
    )?;
    rows.into_iter()
        .map(
            |(cell, music_type, location, joined, worldspace, grid_x, grid_y, interior)| {
                let name = format!("cell place {cell}");
                anyhow::ensure!(joined.is_some(), "{name}: no row in cells");
                let grid = match (grid_x, grid_y) {
                    (Some(x), Some(y)) => {
                        Some((narrow(x, "grid_x", &name)?, narrow(y, "grid_y", &name)?))
                    }
                    (None, None) => None,
                    _ => anyhow::bail!("{name}: only one of grid_x and grid_y is set"),
                };
                Ok(CellPlace {
                    is_interior: flag(interior, "is_interior", &name)?,
                    cell,
                    music_type,
                    location,
                    worldspace,
                    grid,
                })
            },
        )
        .collect()
}

fn load_key_to_music(
    conn: &Connection,
    table: &str,
    key_column: &str,
) -> anyhow::Result<HashMap<String, String>> {
    Ok(collect_rows(
        conn,
        &format!("SELECT {key_column}, music_type FROM {table}"),
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?
    .into_iter()
    .collect())
}

fn load_region_music(conn: &Connection) -> anyhow::Result<Vec<RegionMusic>> {
    let rows = collect_rows(
        conn,
        "SELECT m.region, r.worldspace, m.music_type, m.priority, m.override, \
         m.record_order FROM region_music m LEFT JOIN regions r ON r.form_key = m.region ORDER BY m.region",
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        },
    )?;
    rows.into_iter()
        .map(
            |(region, worldspace, music_type, priority, override_flag, record_order)| {
                let name = format!("region music {region}");
                Ok(RegionMusic {
                    worldspace,
                    priority: narrow(priority, "priority", &name)?,
                    override_flag: flag(override_flag, "override", &name)?,
                    record_order: narrow(record_order, "record_order", &name)?,
                    region,
                    music_type,
                })
            },
        )
        .collect()
}

fn load_region_areas(conn: &Connection) -> anyhow::Result<Vec<RegionArea>> {
    let rows = collect_rows(
        conn,
        "SELECT region, area, points FROM region_areas ORDER BY region, area",
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Vec<u8>>(2)?,
            ))
        },
    )?;
    rows.into_iter()
        .map(|(region, area, blob)| {
            let name = format!("region area {region}/{area}");
            Ok(RegionArea {
                area: narrow(area, "area", &name)?,
                points: decode_points(&blob, &name)?,
                region,
            })
        })
        .collect()
}

fn load_sound_categories(conn: &Connection) -> anyhow::Result<Vec<SoundCategory>> {
    collect_rows(
        conn,
        "SELECT form_key, editor_id, parent, static_attenuation FROM sound_categories \
         ORDER BY form_key",
        |row| {
            Ok(SoundCategory {
                form_key: row.get(0)?,
                editor_id: row.get(1)?,
                parent: row.get(2)?,
                static_multiplier: row.get::<_, Option<f64>>(3)?.map(real32),
            })
        },
    )
}

const COORDINATE_BYTES: usize = 4;
const POINT_BYTES: usize = 2 * COORDINATE_BYTES;

fn decode_points(blob: &[u8], row: &str) -> anyhow::Result<Vec<(f32, f32)>> {
    anyhow::ensure!(
        blob.len().is_multiple_of(POINT_BYTES),
        "{row}: points blob of {} bytes is not a whole number of f32 pairs",
        blob.len()
    );
    let coordinate = |bytes: &[u8]| f32::from_le_bytes(bytes.try_into().expect("4-byte slice"));
    Ok(blob
        .as_chunks::<POINT_BYTES>()
        .0
        .iter()
        .map(|pair| {
            let (x, y) = pair.split_at(COORDINATE_BYTES);
            (coordinate(x), coordinate(y))
        })
        .collect())
}

type Globals = (HashMap<String, f32>, HashMap<String, String>);

fn load_globals(conn: &Connection) -> anyhow::Result<Globals> {
    let rows = collect_rows(
        conn,
        "SELECT form_key, editor_id, value FROM globals",
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<f64>>(2)?,
            ))
        },
    )?;
    let mut values = HashMap::new();
    let mut form_keys = HashMap::new();
    for (form_key, editor_id, value) in rows {
        let value = required(value, "value", &format!("global {form_key}"))?;
        if let Some(editor_id) = editor_id {
            form_keys.insert(editor_id, form_key.clone());
        }
        values.insert(form_key, real32(value));
    }
    Ok((values, form_keys))
}

#[cfg(test)]
mod tests {
    use rusqlite::types::Value;

    use super::*;

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

    fn music_type(conn: &Connection, key: &str, editor_id: &str, priority: i64, fade: f64) {
        insert(
            conn,
            "music_types",
            &[
                ("form_key", text(key)),
                ("editor_id", text(editor_id)),
                ("flags", int(0)),
                ("priority", int(priority)),
                ("ducking_centidb", int(0)),
                ("fade_seconds", real(fade)),
            ],
        );
    }

    fn single_track(conn: &Connection, key: &str, file_key: &str) {
        insert(
            conn,
            "music_tracks",
            &[
                ("form_key", text(key)),
                ("track_type", text("Single")),
                ("duration", real(120.0)),
                ("fade_out", real(2.0)),
                ("file_key", text(file_key)),
            ],
        );
    }

    fn condition_row(track: &str, ord: i64) -> Vec<(&'static str, Value)> {
        vec![
            ("track", text(track)),
            ("ord", int(ord)),
            ("function", int(1)),
            ("compare_op", int(3)),
            ("or_flag", int(0)),
            ("compare_value", real(1.0)),
            ("run_on", int(0)),
        ]
    }

    fn error_text(conn: &Connection) -> String {
        format!("{:#}", MusicCatalog::from_connection(conn).unwrap_err())
    }

    #[test]
    fn loads_music_types_with_ordered_tracks_and_duplicates() {
        let conn = connection();
        music_type(&conn, "00AA:Fallout4.esm", "MUSzExplore", 90, 12.0);
        music_type(&conn, "00AB:Fallout4.esm", "MUSzPublic", 70, 6.0);
        single_track(&conn, "01:Fallout4.esm", "music/a.flac");
        single_track(&conn, "02:Fallout4.esm", "music/b.flac");
        for (ord, track) in [
            (0, "02:Fallout4.esm"),
            (1, "01:Fallout4.esm"),
            (2, "02:Fallout4.esm"),
        ] {
            insert(
                &conn,
                "music_type_tracks",
                &[
                    ("music_type", text("00AA:Fallout4.esm")),
                    ("ord", int(ord)),
                    ("track", text(track)),
                ],
            );
        }

        let catalog = MusicCatalog::from_connection(&conn).unwrap();

        let explore = catalog
            .music_types
            .iter()
            .find(|t| t.editor_id.as_deref() == Some("MUSzExplore"))
            .unwrap();
        assert_eq!((explore.priority, explore.fade_seconds), (90, 12.0));
        assert_eq!(
            explore.tracks,
            ["02:Fallout4.esm", "01:Fallout4.esm", "02:Fallout4.esm"]
        );
        let public = &catalog.music_types[1];
        assert_eq!((public.priority, public.fade_seconds), (70, 6.0));
        assert_eq!(public.tracks, [] as [String; 0]);
    }

    #[test]
    fn loads_track_kinds_layers_and_conditions() {
        let conn = connection();
        single_track(&conn, "01:Fallout4.esm", "music/a.flac");
        insert(
            &conn,
            "music_tracks",
            &[
                ("form_key", text("02:Fallout4.esm")),
                ("track_type", text("Silent")),
                ("duration", real(30.0)),
            ],
        );
        insert(
            &conn,
            "music_tracks",
            &[
                ("form_key", text("03:Fallout4.esm")),
                ("track_type", text("Palette")),
                ("duration", real(60.0)),
                ("fade_out", real(4.0)),
            ],
        );
        for (layer, ord, sub) in [
            (0, 0, "01:Fallout4.esm"),
            (0, 1, "02:Fallout4.esm"),
            (1, 0, "01:Fallout4.esm"),
        ] {
            insert(
                &conn,
                "music_track_layers",
                &[
                    ("track", text("03:Fallout4.esm")),
                    ("layer", int(layer)),
                    ("ord", int(ord)),
                    ("sub_track", text(sub)),
                ],
            );
        }
        let mut with_global = condition_row("01:Fallout4.esm", 1);
        with_global.retain(|(c, _)| *c != "compare_value" && *c != "or_flag");
        with_global.push(("compare_global", text("0F:Fallout4.esm")));
        with_global.push(("or_flag", int(1)));
        insert(
            &conn,
            "music_track_conditions",
            &condition_row("01:Fallout4.esm", 0),
        );
        insert(&conn, "music_track_conditions", &with_global);

        let catalog = MusicCatalog::from_connection(&conn).unwrap();

        let by_key = |key: &str| catalog.tracks.iter().find(|t| t.form_key == key).unwrap();
        assert_eq!(
            by_key("01:Fallout4.esm").kind,
            MusicTrackKind::Single {
                file_key: "music/a.flac".into(),
                duration: Some(120.0),
                fade_out: Some(2.0),
            }
        );
        assert_eq!(
            by_key("02:Fallout4.esm").kind,
            MusicTrackKind::Silent { duration: 30.0 }
        );
        assert_eq!(
            by_key("03:Fallout4.esm").kind,
            MusicTrackKind::Palette {
                layers: vec![
                    vec!["01:Fallout4.esm".into(), "02:Fallout4.esm".into()],
                    vec!["01:Fallout4.esm".into()],
                ],
                duration: 60.0,
                fade_out: Some(4.0),
            }
        );
        let conditions = &by_key("01:Fallout4.esm").conditions;
        assert_eq!(conditions.len(), 2);
        assert_eq!(conditions[0].op, CompareOp::GreaterOrEqual);
        assert_eq!(conditions[0].comparand, Comparand::Value(1.0));
        assert!(!conditions[0].or_flag);
        assert_eq!(
            conditions[1].comparand,
            Comparand::Global("0F:Fallout4.esm".into())
        );
        assert!(conditions[1].or_flag);
    }

    #[test]
    fn loads_places_only_for_cells_in_cell_places() {
        let conn = connection();
        music_type(&conn, "00AA:Fallout4.esm", "MUSzExplore", 90, 12.0);
        for (key, ws, gx, gy, interior) in [
            (
                "C1:Fallout4.esm",
                Some("00003C:Fallout4.esm"),
                Some(4),
                Some(-7),
                0,
            ),
            ("C2:Fallout4.esm", None, None, None, 1),
            (
                "C3:Fallout4.esm",
                Some("00003C:Fallout4.esm"),
                Some(0),
                Some(0),
                0,
            ),
        ] {
            insert(
                &conn,
                "cells",
                &[
                    ("form_key", text(key)),
                    ("worldspace", ws.map_or(Value::Null, text)),
                    ("grid_x", gx.map_or(Value::Null, int)),
                    ("grid_y", gy.map_or(Value::Null, int)),
                    ("is_interior", int(interior)),
                ],
            );
        }
        insert(
            &conn,
            "cell_places",
            &[
                ("cell", text("C1:Fallout4.esm")),
                ("music_type", text("00AA:Fallout4.esm")),
            ],
        );
        insert(
            &conn,
            "cell_places",
            &[
                ("cell", text("C2:Fallout4.esm")),
                ("location", text("L1:Fallout4.esm")),
            ],
        );
        insert(
            &conn,
            "location_music",
            &[
                ("location", text("L0:Fallout4.esm")),
                ("music_type", text("00AA:Fallout4.esm")),
            ],
        );
        insert(
            &conn,
            "worldspace_music",
            &[
                ("worldspace", text("00003C:Fallout4.esm")),
                ("music_type", text("00AA:Fallout4.esm")),
            ],
        );

        let catalog = MusicCatalog::from_connection(&conn).unwrap();

        assert_eq!(
            catalog.cell_places,
            vec![
                CellPlace {
                    cell: "C1:Fallout4.esm".into(),
                    music_type: Some("00AA:Fallout4.esm".into()),
                    location: None,
                    worldspace: Some("00003C:Fallout4.esm".into()),
                    grid: Some((4, -7)),
                    is_interior: false,
                },
                CellPlace {
                    cell: "C2:Fallout4.esm".into(),
                    music_type: None,
                    location: Some("L1:Fallout4.esm".into()),
                    worldspace: None,
                    grid: None,
                    is_interior: true,
                },
            ]
        );
        assert_eq!(
            catalog.location_music["L0:Fallout4.esm"],
            "00AA:Fallout4.esm"
        );
        assert_eq!(
            catalog.worldspace_music["00003C:Fallout4.esm"],
            "00AA:Fallout4.esm"
        );
    }

    #[test]
    fn loads_sound_categories_with_their_parent_and_multiplier() {
        let conn = connection();
        for (key, editor_id, parent, volume) in [
            ("A:Fallout4.esm", "Master", None, Some(0.5)),
            ("B:Fallout4.esm", "Music", Some("A:Fallout4.esm"), None),
        ] {
            insert(
                &conn,
                "sound_categories",
                &[
                    ("form_key", text(key)),
                    ("editor_id", text(editor_id)),
                    ("parent", parent.map_or(Value::Null, text)),
                    ("static_attenuation", volume.map_or(Value::Null, real)),
                ],
            );
        }

        let categories = MusicCatalog::from_connection(&conn)
            .unwrap()
            .sound_categories;

        assert_eq!(categories[0].static_multiplier, Some(0.5));
        assert_eq!(categories[1].parent.as_deref(), Some("A:Fallout4.esm"));
        assert_eq!(categories[1].static_multiplier, None);
    }

    #[test]
    fn loads_regions_with_decoded_polygons_and_globals() {
        let conn = connection();
        music_type(&conn, "00AA:Fallout4.esm", "MUSzExplore", 90, 12.0);
        insert(
            &conn,
            "regions",
            &[
                ("form_key", text("R1:Fallout4.esm")),
                ("worldspace", text("00003C:Fallout4.esm")),
            ],
        );
        insert(
            &conn,
            "region_music",
            &[
                ("region", text("R1:Fallout4.esm")),
                ("music_type", text("00AA:Fallout4.esm")),
                ("priority", int(55)),
                ("override", int(1)),
                ("record_order", int(7)),
            ],
        );
        let points: Vec<u8> = [-1.5f32, 2.0, 4096.0, -8192.25]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        insert(
            &conn,
            "region_areas",
            &[
                ("region", text("R1:Fallout4.esm")),
                ("area", int(0)),
                ("points", Value::Blob(points)),
            ],
        );
        insert(
            &conn,
            "globals",
            &[
                ("form_key", text("G1:Fallout4.esm")),
                ("editor_id", text("TimeScale")),
                ("value_type", text("Float")),
                ("value", real(20.0)),
            ],
        );

        let catalog = MusicCatalog::from_connection(&conn).unwrap();

        assert_eq!(
            catalog.region_music,
            vec![RegionMusic {
                region: "R1:Fallout4.esm".into(),
                worldspace: Some("00003C:Fallout4.esm".into()),
                music_type: "00AA:Fallout4.esm".into(),
                priority: 55,
                override_flag: true,
                record_order: 7,
            }]
        );
        assert_eq!(
            catalog.region_areas[0].points,
            [(-1.5, 2.0), (4096.0, -8192.25)]
        );
        let time_scale = &catalog.global_form_keys["TimeScale"];
        assert_eq!(catalog.globals[time_scale], 20.0);
    }

    #[test]
    fn unknown_track_type_names_the_row() {
        let conn = connection();
        insert(
            &conn,
            "music_tracks",
            &[
                ("form_key", text("01:Fallout4.esm")),
                ("track_type", text("Choir")),
            ],
        );
        let message = error_text(&conn);
        assert!(
            message.contains("01:Fallout4.esm") && message.contains("Choir"),
            "{message}"
        );
    }

    #[test]
    fn single_track_without_file_is_an_error() {
        let conn = connection();
        insert(
            &conn,
            "music_tracks",
            &[
                ("form_key", text("01:Fallout4.esm")),
                ("track_type", text("Single")),
            ],
        );
        assert!(error_text(&conn).contains("file_key is NULL"));
    }

    #[test]
    fn unknown_compare_op_names_the_condition() {
        let conn = connection();
        single_track(&conn, "01:Fallout4.esm", "music/a.flac");
        let mut row = condition_row("01:Fallout4.esm", 3);
        row.retain(|(c, _)| *c != "compare_op");
        row.push(("compare_op", int(9)));
        insert(&conn, "music_track_conditions", &row);
        let message = error_text(&conn);
        assert!(
            message.contains("condition 3") && message.contains("compare_op 9"),
            "{message}"
        );
    }

    #[test]
    fn condition_needs_exactly_one_comparand() {
        let conn = connection();
        single_track(&conn, "01:Fallout4.esm", "music/a.flac");
        let mut row = condition_row("01:Fallout4.esm", 0);
        row.retain(|(c, _)| *c != "compare_value");
        insert(&conn, "music_track_conditions", &row);
        assert!(error_text(&conn).contains("neither compare_value nor compare_global"));
    }

    #[test]
    fn odd_points_blob_names_the_region_area() {
        let conn = connection();
        insert(
            &conn,
            "region_areas",
            &[
                ("region", text("R1:Fallout4.esm")),
                ("area", int(2)),
                ("points", Value::Blob(vec![0; 12])),
            ],
        );
        let message = error_text(&conn);
        assert!(
            message.contains("R1:Fallout4.esm/2") && message.contains("12 bytes"),
            "{message}"
        );
    }

    #[test]
    fn layer_numbering_gap_is_an_error() {
        let conn = connection();
        single_track(&conn, "01:Fallout4.esm", "music/a.flac");
        insert(
            &conn,
            "music_tracks",
            &[
                ("form_key", text("03:Fallout4.esm")),
                ("track_type", text("Palette")),
                ("duration", real(60.0)),
            ],
        );
        insert(
            &conn,
            "music_track_layers",
            &[
                ("track", text("03:Fallout4.esm")),
                ("layer", int(1)),
                ("ord", int(0)),
                ("sub_track", text("01:Fallout4.esm")),
            ],
        );
        assert!(error_text(&conn).contains("music track 03:Fallout4.esm layers"));
    }

    #[test]
    fn unknown_music_type_reference_is_an_error() {
        let conn = connection();
        insert(
            &conn,
            "worldspace_music",
            &[
                ("worldspace", text("00003C:Fallout4.esm")),
                ("music_type", text("FFFF:Fallout4.esm")),
            ],
        );
        let message = error_text(&conn);
        assert!(
            message.contains("00003C:Fallout4.esm") && message.contains("FFFF:Fallout4.esm"),
            "{message}"
        );
    }

    #[test]
    fn cell_place_without_cell_row_is_an_error() {
        let conn = connection();
        insert(&conn, "cell_places", &[("cell", text("C9:Fallout4.esm"))]);
        assert!(error_text(&conn).contains("cell place C9:Fallout4.esm: no row in cells"));
    }

    #[test]
    fn global_without_value_is_an_error() {
        let conn = connection();
        insert(
            &conn,
            "globals",
            &[
                ("form_key", text("G1:Fallout4.esm")),
                ("editor_id", text("TimeScale")),
            ],
        );
        assert!(error_text(&conn).contains("global G1:Fallout4.esm: value is NULL"));
    }

    #[test]
    fn old_database_without_music_columns_asks_for_the_installer() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE music_types (form_key TEXT PRIMARY KEY, editor_id TEXT);")
            .unwrap();
        let message = error_text(&conn);
        assert!(
            message.contains("music_types") && message.contains("installer"),
            "{message}"
        );
    }

    #[test]
    fn database_without_the_region_record_order_asks_for_the_installer() {
        let conn = connection();
        conn.execute_batch("ALTER TABLE region_music DROP COLUMN record_order;")
            .unwrap();
        let message = error_text(&conn);
        assert!(
            message.contains("region_music")
                && message.contains("record_order")
                && message.contains("re-run the installer"),
            "{message}"
        );
    }
}
