//! Walk-in doors and player collision layers against a converted installation that was converted
//! with door trigger bounds and collision layer extraction.
//!
//! Ignored by default (needs an installation). Run with
//! `cargo test --test teleports_real_install -- --ignored --nocapture`; the installation comes
//! from `OPENFO4_INSTALL_DIR` and the database from `OPENFO4_DB` (see `tests/common`).
//! `OPENFO4_SIDECAR_SAMPLE` bounds how many `.col.json` files the histogram reads (default 4000,
//! spread evenly over the sorted list; `0` reads all).

mod common;

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use openfo4::assets::db::DbClient;
use openfo4::assets::db::collision_layers::{
    CollisionLayerRow, LayerSolidity, PlayerCollisionLayers,
};
use openfo4::sim::collision::parse_sidecar_bytes;
use rusqlite::{Connection, OpenFlags};

const DEFAULT_SIDECAR_SAMPLE: usize = 4000;

const SOLID_LAYERS: [&str; 7] = [
    "L_STATIC",
    "L_ANIMSTATIC",
    "L_CLUTTER",
    "L_TRANSPARENT",
    "L_INVISIBLE_WALL",
    "L_STAIRHELPER",
    "L_TERRAIN",
];
const NEVER_TOUCHING_LAYERS: [&str; 6] = [
    "L_NAVCUT",
    "L_TRIGGER",
    "L_ACTORZONE",
    "L_GASTRAP",
    "L_TRAP_TRIGGER",
    "L_NONCOLLIDABLE",
];

fn database() -> DbClient {
    DbClient::new(common::database_path().to_str().unwrap()).unwrap()
}

fn connection() -> Connection {
    Connection::open_with_flags(common::database_path(), OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap()
}

fn layer_indices() -> BTreeMap<String, u8> {
    CollisionLayerRow::list_all(&connection())
        .unwrap()
        .into_iter()
        .filter_map(|row| Some((row.editor_id?, u8::try_from(row.layer_index?).ok()?)))
        .collect()
}

fn player_layers() -> PlayerCollisionLayers {
    let layers = database().get_player_collision_layers().unwrap();
    assert_ne!(
        layers.fingerprint(),
        PlayerCollisionLayers::all_solid().fingerprint(),
        "the collision layer table did not resolve; re-run the installer"
    );
    layers
}

#[test]
#[ignore = "needs a converted installation"]
fn every_active_walk_in_door_has_usable_trigger_bounds_and_a_teleport() {
    let db = database();
    let doors = db.get_walk_in_doors().unwrap();
    assert_eq!(
        doors.without_bounds,
        Vec::<String>::new(),
        "doors without trigger bounds; re-run the installer"
    );
    assert!(doors.rejected.is_empty(), "{:?}", doors.rejected);
    assert!(!doors.doors.is_empty(), "no active walk-in doors found");

    let teleport_sources: HashSet<String> = db
        .get_placed_teleports()
        .unwrap()
        .into_iter()
        .map(|teleport| teleport.source_ref)
        .collect();
    for door in &doors.doors {
        let extents: Vec<f64> = (0..3)
            .map(|axis| door.bounds.max[axis] - door.bounds.min[axis])
            .collect();
        assert!(
            extents.iter().all(|extent| *extent > 0.0),
            "{} has degenerate trigger bounds {:?}",
            door.source_ref,
            door.bounds
        );
        assert!(
            teleport_sources.contains(&door.source_ref),
            "{} has no resolved teleport entry",
            door.source_ref
        );
    }
    println!("{} active walk-in doors", doors.doors.len());
}

#[test]
#[ignore = "needs a converted installation"]
fn every_walk_in_door_trigger_is_the_doorway_slab() {
    let doors = database().get_walk_in_doors().unwrap();
    assert!(!doors.doors.is_empty(), "no active walk-in doors found");
    for door in &doors.doors {
        assert_eq!(door.bounds.min, [-64.0, -8.0, 0.0], "{}", door.source_ref);
        assert_eq!(door.bounds.max, [64.0, 8.0, 192.0], "{}", door.source_ref);
    }
}

#[test]
#[ignore = "needs a converted installation"]
fn the_player_layer_table_separates_solid_layers_from_never_touching_ones() {
    let layers = player_layers();
    let indices = layer_indices();
    let solidity = |editor_id: &str| {
        let index = *indices
            .get(editor_id)
            .unwrap_or_else(|| panic!("no collision layer {editor_id}"));
        layers.solidity(Some(index))
    };
    for editor_id in SOLID_LAYERS {
        assert_eq!(solidity(editor_id), LayerSolidity::Solid, "{editor_id}");
    }
    for editor_id in NEVER_TOUCHING_LAYERS {
        assert_eq!(
            solidity(editor_id),
            LayerSolidity::NeverTouchesPlayer,
            "{editor_id}"
        );
    }
}

fn sidecar_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.to_string_lossy().ends_with(".col.json") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[test]
#[ignore = "needs a converted installation"]
fn junk_wall_door_loses_exactly_its_navcut_shapes() {
    let path = common::asset_root().join("meshes/SetDressing/JunkWalls/JunkWallDoor01.col.json");
    let bytes = std::fs::read(&path).unwrap();
    let parsed = parse_sidecar_bytes(&bytes, "JunkWallDoor01", &player_layers()).unwrap();
    let nodes: Vec<_> = parsed
        .shapes
        .iter()
        .map(|shape| shape.node.as_deref().unwrap_or(""))
        .collect();
    assert_eq!(nodes, ["JunkWallDoor01"]);
    assert_eq!(parsed.dropped_for_player, 3);
}

#[derive(serde::Deserialize)]
struct SidecarLayers {
    shapes: Vec<ShapeLayer>,
}

#[derive(serde::Deserialize)]
struct ShapeLayer {
    layer: Option<serde_json::Value>,
}

#[test]
#[ignore = "needs a converted installation"]
fn sidecar_layers_are_known_and_navcut_and_static_are_split() {
    let layers = player_layers();
    let names: BTreeMap<u8, String> = layer_indices()
        .into_iter()
        .map(|(editor_id, index)| (index, editor_id))
        .collect();
    let all = sidecar_files(&common::asset_root().join("meshes"));
    let sample = std::env::var("OPENFO4_SIDECAR_SAMPLE").map_or(DEFAULT_SIDECAR_SAMPLE, |value| {
        value.parse().expect("OPENFO4_SIDECAR_SAMPLE is a number")
    });
    let step = if sample == 0 {
        1
    } else {
        all.len().div_ceil(sample).max(1)
    };
    let files: Vec<_> = all.iter().step_by(step).collect();

    let mut histogram: BTreeMap<Option<u8>, usize> = BTreeMap::new();
    for file in &files {
        let parsed: SidecarLayers = serde_json::from_slice(&std::fs::read(file).unwrap())
            .unwrap_or_else(|error| panic!("{}: {error}", file.display()));
        for shape in parsed.shapes {
            let layer = shape
                .layer
                .and_then(|value| value.as_u64())
                .and_then(|value| u8::try_from(value).ok());
            *histogram.entry(layer).or_default() += 1;
        }
    }

    println!(
        "{} of {} sidecars read (every {step}th)",
        files.len(),
        all.len()
    );
    for (layer, count) in &histogram {
        let name = layer
            .and_then(|index| names.get(&index))
            .map_or("(none)", String::as_str);
        println!(
            "layer {layer:?} {name}: {count} shapes, {:?}",
            layers.solidity(*layer)
        );
    }
    let unknown: Vec<u8> = histogram
        .keys()
        .flatten()
        .filter(|index| !names.contains_key(index))
        .copied()
        .collect();
    assert_eq!(
        unknown,
        Vec::<u8>::new(),
        "sidecar layers missing from the layer table"
    );
    let index_of = |editor_id: &str| {
        *names
            .iter()
            .find(|(_, name)| name.as_str() == editor_id)
            .unwrap_or_else(|| panic!("no collision layer {editor_id}"))
            .0
    };
    for (editor_id, solidity) in [
        ("L_NAVCUT", LayerSolidity::NeverTouchesPlayer),
        ("L_STATIC", LayerSolidity::Solid),
    ] {
        let index = index_of(editor_id);
        assert!(
            histogram.get(&Some(index)).is_some_and(|count| *count > 0),
            "no sampled shape on {editor_id}; re-run the installer"
        );
        assert_eq!(layers.solidity(Some(index)), solidity, "{editor_id}");
    }
}
