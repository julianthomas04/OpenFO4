//! Grass placement for one real cell of the converted game data, CPU only,
//! through the path the streaming worker ships (`GrassWorkerData`).
//!
//! Ignored by default (needs an installation). Run with
//! `OPENFO4_DB=<install>/assets/data/data.db cargo test --test grass_real_cell -- --ignored --nocapture`;
//! `OPENFO4_GRASS_CELL=x,y` picks another Commonwealth cell (the exact-count
//! assertion only holds for the default Sanctuary cell -20,22).

use std::path::Path;
use std::time::Instant;

use glam::Vec3;
use openfo4::assets::db::DbClient;
use openfo4::rendering::game_settings::GameSettings;
use openfo4::rendering::grass_streaming::GrassWorkerData;

const COMMONWEALTH: &str = "00003C:Fallout4.esm";
const SANCTUARY: (i32, i32) = (-20, 22);
/// Placement is deterministic, so the Sanctuary count is exact. It changes
/// when the placement, the layer decoding or the installed data change.
const SANCTUARY_BLADES: usize = 1270;
const SANCTUARY_TYPES: usize = 8;

fn requested_cell() -> (i32, i32) {
    std::env::var("OPENFO4_GRASS_CELL").map_or(SANCTUARY, |value| {
        let (x, y) = value.split_once(',').expect("OPENFO4_GRASS_CELL is x,y");
        (x.trim().parse().unwrap(), y.trim().parse().unwrap())
    })
}

#[test]
#[ignore = "needs a converted installation (OPENFO4_DB)"]
fn places_grass_for_a_commonwealth_cell() {
    let db_path = std::env::var("OPENFO4_DB").expect("set OPENFO4_DB to data.db");
    let asset_root = Path::new(&db_path)
        .parent()
        .and_then(Path::parent)
        .expect("data.db sits in <assets>/data");
    let db = DbClient::new(&db_path).unwrap();
    let settings = GameSettings::load(asset_root).expect("the install carries the game INIs");
    let worker = GrassWorkerData::load(&db, asset_root.to_str().unwrap(), settings.grass_placement);

    let (gx, gy) = requested_cell();
    let cell = db.get_cell(gx, gy, COMMONWEALTH).unwrap();
    let cell_key = cell.form_key.clone().unwrap();
    let landscape = db.get_landscape(&cell_key, gx, gy).unwrap();

    let started = Instant::now();
    let grass = worker.place(&db, COMMONWEALTH, &cell_key, &landscape);
    let elapsed = started.elapsed();

    let total: usize = grass.iter().map(|g| g.blades.len()).sum();
    println!(
        "cell ({gx}, {gy}): {total} blades of {} types in {elapsed:?}",
        grass.len()
    );
    for type_grass in &grass {
        println!(
            "  {:>6}  {}",
            type_grass.blades.len(),
            type_grass.model_path
        );
        assert!(
            asset_root.join(&type_grass.model_path).is_file(),
            "{} is not a converted model",
            type_grass.model_path
        );
        for blade in &type_grass.blades {
            let (right, up) = (Vec3::from(blade.right), Vec3::from(blade.up));
            assert!((right.length() - 1.0).abs() < 1e-3 && (up.length() - 1.0).abs() < 1e-3);
            assert!(right.dot(up).abs() < 1e-3);
        }
    }

    if (gx, gy) == SANCTUARY {
        assert_eq!(total, SANCTUARY_BLADES);
        assert_eq!(grass.len(), SANCTUARY_TYPES);
    } else {
        assert!(total > 0, "a land cell with grass textures grows some");
    }
}
