//! Where the spawn picker can put the player: a fixed table of named places, each resolved from a
//! map marker in the database or given explicitly as where the player body stands.

use std::collections::HashMap;

use glam::Vec3;

use crate::assets::db::{DbClient, placed_objects::MarkerRef};
use crate::bethesda_space::{position_from_engine, rotation_z_from_compass_heading_degrees};
use crate::sim::STANDING_CENTER_ABOVE_FEET;

/// The Commonwealth worldspace.
pub const COMMONWEALTH: &str = "00003C:Fallout4.esm";

/// Bethesda units the spawn sits above its marker, so the ground snap starts above terrain a
/// marker may be sunk into.
pub const SPAWN_LIFT: f64 = 100.0;

/// What the player enters: an exterior worldspace or one interior cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnPlace {
    Exterior { worldspace: String },
    Interior { cell_form_key: String },
}

/// A resolved spawn: where the player enters the world, ready for the streamer.
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnLocation {
    pub name: &'static str,
    pub place: SpawnPlace,
    /// Bethesda world units, including [`SPAWN_LIFT`].
    pub position: [f64; 3],
    /// Bethesda Z rotation in radians the player faces; `None` keeps the default look.
    pub heading: Option<f64>,
}

/// Where the player body stands, as the debug Player panel reports it.
#[derive(Debug, Clone, Copy)]
struct BodyPose {
    /// Engine units, the centre of the standing capsule.
    engine_position: [f32; 3],
    /// Compass degrees as the debug panel shows them: north 0, clockwise.
    compass_heading: f32,
}

#[derive(Debug, Clone, Copy)]
enum SpawnSource {
    /// A map or heading marker ref; the player faces the ref's own heading.
    Marker {
        ref_editor_id: &'static str,
    },
    Exterior {
        body: BodyPose,
    },
    Interior {
        cell_editor_id: &'static str,
        body: BodyPose,
    },
}

struct SpawnEntry {
    name: &'static str,
    source: SpawnSource,
}

const fn marker(name: &'static str, ref_editor_id: &'static str) -> SpawnEntry {
    SpawnEntry {
        name,
        source: SpawnSource::Marker { ref_editor_id },
    }
}

const fn exterior(
    name: &'static str,
    engine_position: [f32; 3],
    compass_heading: f32,
) -> SpawnEntry {
    SpawnEntry {
        name,
        source: SpawnSource::Exterior {
            body: BodyPose {
                engine_position,
                compass_heading,
            },
        },
    }
}

const fn interior(
    name: &'static str,
    cell_editor_id: &'static str,
    engine_position: [f32; 3],
    compass_heading: f32,
) -> SpawnEntry {
    SpawnEntry {
        name,
        source: SpawnSource::Interior {
            cell_editor_id,
            body: BodyPose {
                engine_position,
                compass_heading,
            },
        },
    }
}

const SPAWN_TABLE: [SpawnEntry; 7] = [
    marker("RED ROCKET", "RedRocketMapMarkerHeadingRef"),
    exterior("DIAMOND CITY", [-1682.6, 136.8, 3218.8], 34.5),
    exterior("THE CASTLE", [4481.7, 65.4, 4413.6], 115.3),
    exterior("SANCTUARY HILLS", [-7887.3, 787.7, -8988.0], 317.2),
    interior(
        "MUSEUM OF FREEDOM",
        "ConcordMuseum01",
        [42.7, -3.2, -1.7],
        269.4,
    ),
    exterior("GOODNEIGHBOR", [1958.4, 64.0, 1132.4], 125.1),
    exterior("BUNKER HILL", [1827.5, 107.7, -1216.2], 358.9),
];

#[derive(Debug, thiserror::Error)]
pub enum SpawnLocationError {
    #[error("the spawn marker lookup failed: {0}")]
    Query(#[source] anyhow::Error),
    #[error("spawn location {name}: no marker ref {editor_id:?} exists in the database")]
    NotFound {
        name: &'static str,
        editor_id: &'static str,
    },
    #[error("spawn location {name}: marker ref {editor_id:?} matches more than one placed ref")]
    Ambiguous {
        name: &'static str,
        editor_id: &'static str,
    },
    #[error("spawn location {name}: marker ref {editor_id:?} sits in an interior cell")]
    Interior {
        name: &'static str,
        editor_id: &'static str,
    },
    #[error("spawn location {name}: marker ref {editor_id:?} sits in a cell without a worldspace")]
    NoWorldspace {
        name: &'static str,
        editor_id: &'static str,
    },
    #[error("spawn location {name}: marker ref {editor_id:?} has no position")]
    NoPosition {
        name: &'static str,
        editor_id: &'static str,
    },
    #[error(
        "spawn location {name}: the lookup of interior cell {cell_editor_id:?} failed: {source}"
    )]
    CellQuery {
        name: &'static str,
        cell_editor_id: &'static str,
        #[source]
        source: anyhow::Error,
    },
    #[error(
        "spawn location {name}: no cell with editor id {cell_editor_id:?} exists in the database"
    )]
    CellNotFound {
        name: &'static str,
        cell_editor_id: &'static str,
    },
    #[error("spawn location {name}: cell {cell_editor_id:?} has no form key")]
    CellWithoutFormKey {
        name: &'static str,
        cell_editor_id: &'static str,
    },
    #[error("spawn location {name}: cell {cell_editor_id:?} is not an interior cell")]
    CellNotInterior {
        name: &'static str,
        cell_editor_id: &'static str,
    },
}

/// Every location of the spawn picker, in display order.
pub fn resolve_spawn_locations(db: &DbClient) -> Result<Vec<SpawnLocation>, SpawnLocationError> {
    resolve_entries(db, &SPAWN_TABLE)
}

fn resolve_entries(
    db: &DbClient,
    entries: &[SpawnEntry],
) -> Result<Vec<SpawnLocation>, SpawnLocationError> {
    let mut markers = marker_refs_by_editor_id(db, entries)?;
    entries
        .iter()
        .map(|entry| match entry.source {
            SpawnSource::Marker { ref_editor_id } => resolve_marker(
                entry.name,
                ref_editor_id,
                markers.remove(ref_editor_id).unwrap_or_default(),
            ),
            SpawnSource::Exterior { body } => Ok(body_spawn(
                entry.name,
                SpawnPlace::Exterior {
                    worldspace: COMMONWEALTH.to_owned(),
                },
                body,
            )),
            SpawnSource::Interior {
                cell_editor_id,
                body,
            } => Ok(body_spawn(
                entry.name,
                SpawnPlace::Interior {
                    cell_form_key: interior_cell_form_key(db, entry.name, cell_editor_id)?,
                },
                body,
            )),
        })
        .collect()
}

fn marker_refs_by_editor_id(
    db: &DbClient,
    entries: &[SpawnEntry],
) -> Result<HashMap<String, Vec<MarkerRef>>, SpawnLocationError> {
    let editor_ids: Vec<&str> = entries
        .iter()
        .filter_map(|entry| match entry.source {
            SpawnSource::Marker { ref_editor_id } => Some(ref_editor_id),
            SpawnSource::Exterior { .. } | SpawnSource::Interior { .. } => None,
        })
        .collect();
    let mut by_editor_id: HashMap<String, Vec<MarkerRef>> = HashMap::new();
    if editor_ids.is_empty() {
        return Ok(by_editor_id);
    }
    let refs = db
        .get_marker_refs_by_editor_ids(&editor_ids)
        .map_err(SpawnLocationError::Query)?;
    for marker in refs {
        by_editor_id
            .entry(marker.editor_id.clone())
            .or_default()
            .push(marker);
    }
    Ok(by_editor_id)
}

fn interior_cell_form_key(
    db: &DbClient,
    name: &'static str,
    cell_editor_id: &'static str,
) -> Result<String, SpawnLocationError> {
    let cell = db
        .get_cell_by_editor_id(cell_editor_id)
        .map_err(|source| SpawnLocationError::CellQuery {
            name,
            cell_editor_id,
            source,
        })?
        .ok_or(SpawnLocationError::CellNotFound {
            name,
            cell_editor_id,
        })?;
    if !cell.is_interior {
        return Err(SpawnLocationError::CellNotInterior {
            name,
            cell_editor_id,
        });
    }
    cell.form_key.ok_or(SpawnLocationError::CellWithoutFormKey {
        name,
        cell_editor_id,
    })
}

/// Puts the soles below the reported capsule centre, so spawning at a reported position stands the
/// body there again, then lifts the spawn by [`SPAWN_LIFT`].
fn body_spawn(name: &'static str, place: SpawnPlace, body: BodyPose) -> SpawnLocation {
    let [x, y, z] = body.engine_position;
    let soles = position_from_engine(Vec3::new(x, y - STANDING_CENTER_ABOVE_FEET, z));
    SpawnLocation {
        name,
        place,
        position: [
            f64::from(soles.x),
            f64::from(soles.y),
            f64::from(soles.z) + SPAWN_LIFT,
        ],
        heading: Some(f64::from(rotation_z_from_compass_heading_degrees(
            body.compass_heading,
        ))),
    }
}

fn resolve_marker(
    name: &'static str,
    editor_id: &'static str,
    mut matches: Vec<MarkerRef>,
) -> Result<SpawnLocation, SpawnLocationError> {
    let marker = match matches.len() {
        0 => return Err(SpawnLocationError::NotFound { name, editor_id }),
        1 => matches.remove(0),
        _ => return Err(SpawnLocationError::Ambiguous { name, editor_id }),
    };
    if marker.cell_is_interior == Some(true) {
        return Err(SpawnLocationError::Interior { name, editor_id });
    }
    let worldspace = marker
        .worldspace
        .ok_or(SpawnLocationError::NoWorldspace { name, editor_id })?;
    let [x, y, z] = marker
        .position
        .ok_or(SpawnLocationError::NoPosition { name, editor_id })?;
    Ok(SpawnLocation {
        name,
        place: SpawnPlace::Exterior { worldspace },
        position: [x, y, z + SPAWN_LIFT],
        heading: marker.rotation_z,
    })
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "positions are stored and read back bit-exactly; the lift adds an exact integer; headings are rounded to one decimal"
)]
mod tests {
    use rusqlite::{Connection, params};

    use super::*;
    use crate::bethesda_space::{compass_heading_degrees, position_to_engine, yaw_from_rotation_z};
    use crate::sim::world_to_grid;

    const SCHEMA: &str = include_str!("../../../installer/file_converters/esm_schema.sql");
    const MAP_MARKER: &str = "000010:Fallout4.esm";
    const HEADING_MARKER: &str = "000034:Fallout4.esm";
    const OTHER_BASE: &str = "012345:Fallout4.esm";
    const EXTERIOR_CELL: &str = "AA0001:Fallout4.esm";
    const INTERIOR_CELL: &str = "AA0002:Fallout4.esm";
    const ORPHAN_CELL: &str = "AA0003:Fallout4.esm";
    const WORLDSPACE: &str = "BB0001:Fallout4.esm";
    const MUSEUM_CELL: &str = "CC0001:Fallout4.esm";
    const MUSEUM_EDITOR_ID: &str = "ConcordMuseum01";
    const PICKER_NAMES: [&str; 7] = [
        "RED ROCKET",
        "DIAMOND CITY",
        "THE CASTLE",
        "SANCTUARY HILLS",
        "MUSEUM OF FREEDOM",
        "GOODNEIGHBOR",
        "BUNKER HILL",
    ];

    struct Marker<'a> {
        editor_id: &'a str,
        base: &'a str,
        cell: Option<&'a str>,
        position: Option<[f64; 3]>,
        rotation_z: f64,
    }

    impl<'a> Marker<'a> {
        fn exterior(editor_id: &'a str, position: [f64; 3]) -> Self {
            Self {
                editor_id,
                base: MAP_MARKER,
                cell: Some(EXTERIOR_CELL),
                position: Some(position),
                rotation_z: 0.0,
            }
        }
    }

    fn database_with_cells(markers: &[Marker], extra_cells: &[(&str, &str, i32)]) -> DbClient {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        for (cell, worldspace, interior) in [
            (EXTERIOR_CELL, Some(WORLDSPACE), 0),
            (INTERIOR_CELL, None, 1),
            (ORPHAN_CELL, None, 0),
        ] {
            conn.execute(
                "INSERT INTO cells (form_key, worldspace, is_interior) VALUES (?, ?, ?)",
                params![cell, worldspace, interior],
            )
            .unwrap();
        }
        for (form_key, editor_id, interior) in extra_cells {
            conn.execute(
                "INSERT INTO cells (form_key, editor_id, is_interior) VALUES (?, ?, ?)",
                params![form_key, editor_id, interior],
            )
            .unwrap();
        }
        for (index, marker) in markers.iter().enumerate() {
            let form_key = format!("{:06X}:Fallout4.esm", 0x10_0000 + index);
            conn.execute(
                "INSERT INTO records (form_key, record_type, editor_id, source_plugin)
                 VALUES (?, 'REFR', ?, 'Fallout4.esm')",
                params![form_key, marker.editor_id],
            )
            .unwrap();
            let [x, y, z] = marker.position.map_or([None; 3], |p| p.map(Some));
            conn.execute(
                "INSERT INTO placed_objects
                   (form_key, cell_form_key, base_object, object_type,
                    position_x, position_y, position_z, rotation_z)
                 VALUES (?, ?, ?, 'PlacedObject', ?, ?, ?, ?)",
                params![
                    form_key,
                    marker.cell,
                    marker.base,
                    x,
                    y,
                    z,
                    marker.rotation_z
                ],
            )
            .unwrap();
        }
        DbClient::from_connection(conn).unwrap()
    }

    fn database(markers: &[Marker]) -> DbClient {
        database_with_cells(markers, &[])
    }

    fn red_rocket_marker() -> Marker<'static> {
        Marker {
            rotation_z: 1.5,
            ..Marker::exterior("RedRocketMapMarkerHeadingRef", [100.0, 200.0, 300.0])
        }
    }

    fn picker_database() -> DbClient {
        database_with_cells(
            &[red_rocket_marker()],
            &[(MUSEUM_CELL, MUSEUM_EDITOR_ID, 1)],
        )
    }

    fn resolve_marker_entry(markers: &[Marker]) -> Result<SpawnLocation, SpawnLocationError> {
        let db = database(markers);
        resolve_entries(&db, &[marker("PLACE", "PlaceMarker")]).map(|mut all| all.remove(0))
    }

    fn resolve_museum(cells: &[(&str, &str, i32)]) -> Result<SpawnLocation, SpawnLocationError> {
        let db = database_with_cells(&[], cells);
        let entry = interior("PLACE", MUSEUM_EDITOR_ID, [1.0, 2.0, 3.0], 90.0);
        resolve_entries(&db, &[entry]).map(|mut all| all.remove(0))
    }

    fn picker() -> Vec<SpawnLocation> {
        resolve_spawn_locations(&picker_database()).unwrap()
    }

    fn location<'a>(locations: &'a [SpawnLocation], name: &str) -> &'a SpawnLocation {
        locations
            .iter()
            .find(|location| location.name == name)
            .unwrap_or_else(|| panic!("the picker lists {name}"))
    }

    fn unlifted_engine_position(location: &SpawnLocation) -> Vec3 {
        let [x, y, z] = location.position.map(|c| c as f32);
        position_to_engine(x, y, z - SPAWN_LIFT as f32)
    }

    #[test]
    fn the_picker_lists_exactly_the_requested_places_in_order() {
        let names: Vec<_> = picker().iter().map(|location| location.name).collect();
        assert_eq!(names, PICKER_NAMES);
    }

    #[test]
    fn the_table_names_are_uppercase_and_unique() {
        for entry in &SPAWN_TABLE {
            assert_eq!(entry.name, entry.name.to_uppercase());
        }
        let mut names: Vec<_> = SPAWN_TABLE.iter().map(|entry| entry.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), SPAWN_TABLE.len());
    }

    #[test]
    fn red_rocket_comes_from_its_marker_with_the_marker_heading_and_a_lifted_position() {
        let locations = picker();
        let red_rocket = location(&locations, "RED ROCKET");
        assert_eq!(
            red_rocket.place,
            SpawnPlace::Exterior {
                worldspace: WORLDSPACE.to_owned()
            }
        );
        assert_eq!(red_rocket.position, [100.0, 200.0, 400.0]);
        assert_eq!(red_rocket.heading, Some(1.5));
    }

    #[test]
    fn the_explicit_exteriors_spawn_in_the_commonwealth_in_the_users_cells() {
        let locations = picker();
        for (name, cell) in [
            ("DIAMOND CITY", (-5, -8)),
            ("THE CASTLE", (10, -11)),
            ("SANCTUARY HILLS", (-20, 21)),
            ("GOODNEIGHBOR", (4, -3)),
            ("BUNKER HILL", (4, 2)),
        ] {
            let spawn = location(&locations, name);
            assert_eq!(
                spawn.place,
                SpawnPlace::Exterior {
                    worldspace: COMMONWEALTH.to_owned()
                },
                "{name}"
            );
            assert_eq!(
                world_to_grid(unlifted_engine_position(spawn)),
                cell,
                "{name}"
            );
        }
    }

    #[test]
    fn the_converted_positions_are_the_bethesda_coordinates_of_the_reported_bodies() {
        let locations = picker();
        let bethesda = |name| {
            let position = location(&locations, name).position;
            [position[0], position[1]].map(f64::round)
        };
        assert_eq!(bethesda("DIAMOND CITY"), [-16826.0, -32188.0]);
        assert_eq!(bethesda("THE CASTLE"), [44817.0, -44136.0]);
        assert_eq!(bethesda("SANCTUARY HILLS"), [-78873.0, 89880.0]);
        assert_eq!(bethesda("GOODNEIGHBOR"), [19584.0, -11324.0]);
        assert_eq!(bethesda("BUNKER HILL"), [18275.0, 12162.0]);
    }

    #[test]
    fn spawning_at_a_position_stands_the_body_where_the_panel_reported_it() {
        let locations = picker();
        for (name, reported) in [
            ("DIAMOND CITY", Vec3::new(-1682.6, 136.8, 3218.8)),
            ("MUSEUM OF FREEDOM", Vec3::new(42.7, -3.2, -1.7)),
            ("BUNKER HILL", Vec3::new(1827.5, 107.7, -1216.2)),
        ] {
            let soles = unlifted_engine_position(location(&locations, name));
            let body_centre = soles + Vec3::Y * STANDING_CENTER_ABOVE_FEET;
            assert!(
                body_centre.abs_diff_eq(reported, 1e-2),
                "{name}: {body_centre:?} vs {reported:?}"
            );
        }
    }

    #[test]
    fn the_spawn_sits_one_hundred_bethesda_units_above_the_soles() {
        assert_eq!(SPAWN_LIFT, 100.0);
        let locations = picker();
        let castle = location(&locations, "THE CASTLE");
        let soles_z = 654.0 - f64::from(STANDING_CENTER_ABOVE_FEET) * 10.0;
        assert!((castle.position[2] - (soles_z + SPAWN_LIFT)).abs() < 1e-2);
    }

    #[test]
    fn the_headings_read_back_exactly_on_the_compass() {
        let locations = picker();
        for (name, degrees) in [
            ("DIAMOND CITY", 34.5),
            ("THE CASTLE", 115.3),
            ("SANCTUARY HILLS", 317.2),
            ("MUSEUM OF FREEDOM", 269.4),
            ("GOODNEIGHBOR", 125.1),
            ("BUNKER HILL", 358.9),
        ] {
            let rotation_z = location(&locations, name).heading.unwrap() as f32;
            assert_eq!(
                compass_heading_degrees(yaw_from_rotation_z(rotation_z)),
                degrees,
                "{name}"
            );
        }
    }

    #[test]
    fn the_museum_of_freedom_is_the_interior_cell_found_by_editor_id() {
        let locations = picker();
        let museum = location(&locations, "MUSEUM OF FREEDOM");
        assert_eq!(
            museum.place,
            SpawnPlace::Interior {
                cell_form_key: MUSEUM_CELL.to_owned()
            }
        );
        let [x, y, z] = museum.position;
        assert_eq!(
            [x.round(), y.round()],
            [427.0, 17.0],
            "interior position is the reported body in Bethesda units"
        );
        assert!(z.is_finite());
    }

    #[test]
    fn a_missing_interior_cell_is_reported_as_not_found() {
        assert!(matches!(
            resolve_museum(&[]).unwrap_err(),
            SpawnLocationError::CellNotFound {
                name: "PLACE",
                cell_editor_id: MUSEUM_EDITOR_ID
            }
        ));
    }

    #[test]
    fn an_exterior_cell_with_the_editor_id_is_reported_as_not_interior() {
        assert!(matches!(
            resolve_museum(&[(MUSEUM_CELL, MUSEUM_EDITOR_ID, 0)]).unwrap_err(),
            SpawnLocationError::CellNotInterior { .. }
        ));
    }

    #[test]
    fn a_marker_resolves_to_its_worldspace_and_a_lifted_position() {
        let location =
            resolve_marker_entry(&[Marker::exterior("PlaceMarker", [10.0, 20.0, 30.0])]).unwrap();
        assert_eq!(location.name, "PLACE");
        assert_eq!(
            location.place,
            SpawnPlace::Exterior {
                worldspace: WORLDSPACE.to_owned()
            }
        );
        assert_eq!(location.position, [10.0, 20.0, 130.0]);
    }

    #[test]
    fn a_marker_entry_faces_the_ref_heading() {
        let markers = [Marker {
            rotation_z: 2.5,
            ..Marker::exterior("PlaceMarker", [0.0, 0.0, 0.0])
        }];
        assert_eq!(resolve_marker_entry(&markers).unwrap().heading, Some(2.5));
    }

    #[test]
    fn heading_markers_resolve_like_map_markers() {
        let markers = [Marker {
            base: HEADING_MARKER,
            ..Marker::exterior("PlaceMarker", [1.0, 2.0, 3.0])
        }];
        assert!(resolve_marker_entry(&markers).is_ok());
    }

    #[test]
    fn a_missing_ref_is_reported_as_not_found() {
        assert!(matches!(
            resolve_marker_entry(&[]).unwrap_err(),
            SpawnLocationError::NotFound {
                name: "PLACE",
                editor_id: "PlaceMarker"
            }
        ));
    }

    #[test]
    fn a_ref_of_another_base_object_is_not_a_marker() {
        let markers = [Marker {
            base: OTHER_BASE,
            ..Marker::exterior("PlaceMarker", [0.0, 0.0, 0.0])
        }];
        assert!(matches!(
            resolve_marker_entry(&markers).unwrap_err(),
            SpawnLocationError::NotFound { .. }
        ));
    }

    #[test]
    fn a_marker_in_an_interior_cell_is_reported_as_interior() {
        let markers = [Marker {
            cell: Some(INTERIOR_CELL),
            ..Marker::exterior("PlaceMarker", [0.0, 0.0, 0.0])
        }];
        assert!(matches!(
            resolve_marker_entry(&markers).unwrap_err(),
            SpawnLocationError::Interior { .. }
        ));
    }

    #[test]
    fn a_marker_in_an_exterior_cell_without_a_worldspace_is_reported() {
        let markers = [Marker {
            cell: Some(ORPHAN_CELL),
            ..Marker::exterior("PlaceMarker", [0.0, 0.0, 0.0])
        }];
        assert!(matches!(
            resolve_marker_entry(&markers).unwrap_err(),
            SpawnLocationError::NoWorldspace { .. }
        ));
    }

    #[test]
    fn a_marker_in_no_cell_is_reported_as_without_a_worldspace() {
        let markers = [Marker {
            cell: None,
            ..Marker::exterior("PlaceMarker", [0.0, 0.0, 0.0])
        }];
        assert!(matches!(
            resolve_marker_entry(&markers).unwrap_err(),
            SpawnLocationError::NoWorldspace { .. }
        ));
    }

    #[test]
    fn a_marker_without_a_position_is_reported() {
        let markers = [Marker {
            position: None,
            ..Marker::exterior("PlaceMarker", [0.0, 0.0, 0.0])
        }];
        assert!(matches!(
            resolve_marker_entry(&markers).unwrap_err(),
            SpawnLocationError::NoPosition { .. }
        ));
    }

    #[test]
    fn two_refs_with_one_editor_id_are_ambiguous() {
        let markers = [
            Marker::exterior("PlaceMarker", [1.0, 1.0, 1.0]),
            Marker::exterior("PlaceMarker", [2.0, 2.0, 2.0]),
        ];
        assert!(matches!(
            resolve_marker_entry(&markers).unwrap_err(),
            SpawnLocationError::Ambiguous { .. }
        ));
    }

    #[test]
    fn marker_entries_resolve_in_table_order_from_one_database() {
        let markers = [
            Marker::exterior("SecondMarker", [2.0, 0.0, 0.0]),
            Marker::exterior("FirstMarker", [1.0, 0.0, 0.0]),
        ];
        let db = database(&markers);
        let entries = [
            marker("FIRST", "FirstMarker"),
            marker("SECOND", "SecondMarker"),
        ];
        let names: Vec<_> = resolve_entries(&db, &entries)
            .unwrap()
            .iter()
            .map(|location| (location.name, location.position[0]))
            .collect();
        assert_eq!(names, [("FIRST", 1.0), ("SECOND", 2.0)]);
    }

    #[test]
    fn a_table_without_markers_needs_no_marker_query_result() {
        let db = database(&[]);
        let entries = [exterior("HERE", [1.0, 2.0, 3.0], 0.0)];
        assert_eq!(resolve_entries(&db, &entries).unwrap().len(), 1);
    }

    #[test]
    fn the_first_failing_entry_decides_the_error() {
        let db = database(&[Marker::exterior("FirstMarker", [0.0, 0.0, 0.0])]);
        let entries = [
            marker("FIRST", "FirstMarker"),
            marker("SECOND", "SecondMarker"),
        ];
        assert!(matches!(
            resolve_entries(&db, &entries).unwrap_err(),
            SpawnLocationError::NotFound { name: "SECOND", .. }
        ));
    }
}
