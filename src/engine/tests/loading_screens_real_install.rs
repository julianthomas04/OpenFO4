//! The loading screens of a converted installation: every screen's model exists as a converted
//! GLB, and the selection finds a screen for the worlds a player is loaded into.
//!
//! Ignored by default (needs an installation whose database was converted with the loading screen
//! tables). Run with `cargo test --test loading_screens_real_install -- --ignored --nocapture`; the
//! installation comes from `OPENFO4_INSTALL_DIR` and the database from `OPENFO4_DB` (see
//! `tests/common`). A database that predates the loading screen export makes the tests skip with a
//! message; re-run the installer's database step to produce the tables.

mod common;

use openfo4::assets::db::DbClient;
use openfo4::assets::db::load_screens::LoadScreenCatalog;
use openfo4::assets::types::glb_key::glb_key;
use openfo4::loading_screen::{LoadScreenContext, Place, select};
use openfo4::random::SplitMix64;
use rusqlite::{Connection, OpenFlags};

const COMMONWEALTH: &str = "00003C:Fallout4.esm";
const DIAMOND_CITY_EDITOR_ID: &str = "DiamondCity";
const SEEDS: u64 = 64;

/// The catalog, or `None` after saying why the test has nothing to check.
fn catalog() -> Option<LoadScreenCatalog> {
    let db = DbClient::new(common::database_path().to_str().unwrap()).unwrap();
    let catalog = db
        .get_load_screen_catalog()
        .expect("the loading screen rows are complete");
    if catalog.is_none() {
        println!(
            "SKIPPED: the database has no loading screen tables; re-run the installer's database \
             step to export them"
        );
    }
    catalog
}

fn diamond_city_worldspace() -> String {
    let connection =
        Connection::open_with_flags(common::database_path(), OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    connection
        .query_row(
            "SELECT form_key FROM worldspaces WHERE editor_id = ?1",
            [DIAMOND_CITY_EDITOR_ID],
            |row| row.get(0),
        )
        .expect("the Diamond City worldspace is in the database")
}

fn exterior(worldspace: &str) -> Place {
    Place::Exterior {
        worldspace: worldspace.to_string(),
    }
}

#[test]
#[ignore = "needs a converted installation"]
fn every_loading_screen_model_exists_as_a_converted_model() {
    let Some(catalog) = catalog() else { return };
    assert!(!catalog.screens.is_empty(), "the catalog has screens");
    let assets = common::asset_root();
    let missing: Vec<String> = catalog
        .screens
        .iter()
        .filter(|screen| !assets.join(glb_key(&screen.model_path)).is_file())
        .map(|screen| format!("{} -> {}", screen.label(), screen.model_path))
        .collect();
    println!(
        "{} loading screens, {} without a converted model",
        catalog.screens.len(),
        missing.len()
    );
    assert!(missing.is_empty(), "screens without a GLB: {missing:#?}");
}

fn assert_a_screen_is_selected_for(catalog: &LoadScreenCatalog, destination: &Place, name: &str) {
    for seed in 0..SEEDS {
        let mut rng = SplitMix64::new(seed);
        let context = LoadScreenContext::new(catalog, destination, "", rng.fork());
        let screen = select(catalog, &context, &mut rng.fork());
        assert!(screen.is_some(), "no screen for {name} with seed {seed}");
    }
}

#[test]
#[ignore = "needs a converted installation"]
fn a_screen_is_selected_for_the_commonwealth_and_for_diamond_city() {
    let Some(catalog) = catalog() else { return };
    assert_a_screen_is_selected_for(&catalog, &exterior(COMMONWEALTH), "the Commonwealth");
    assert_a_screen_is_selected_for(
        &catalog,
        &exterior(&diamond_city_worldspace()),
        "Diamond City",
    );
}

#[test]
#[ignore = "needs a converted installation"]
fn the_screens_of_the_install_do_not_all_repeat_for_one_destination() {
    let Some(catalog) = catalog() else { return };
    let destination = exterior(COMMONWEALTH);
    let mut seen = std::collections::BTreeSet::new();
    for seed in 0..SEEDS {
        let mut rng = SplitMix64::new(seed);
        let context = LoadScreenContext::new(&catalog, &destination, "", rng.fork());
        if let Some(screen) = select(&catalog, &context, &mut rng.fork()) {
            seen.insert(screen.label().to_string());
        }
    }
    println!("{} distinct screens over {SEEDS} seeds", seen.len());
    assert!(seen.len() > 1, "the choice varies: {seen:?}");
}
