//! The ambient music catalog of a converted installation, checked against the vanilla record counts.
//!
//! Ignored by default (needs an installation whose database was converted with the music tables).
//! Run with `cargo test --test music_catalog_real_install -- --ignored --nocapture`; the database is
//! `OPENFO4_DB` when set, else the `data/data.db` of the installation in `OPENFO4_INSTALL_DIR`.

#![allow(
    clippy::float_cmp,
    reason = "values are stored and read back bit-exactly; no arithmetic is involved"
)]

mod common;

use openfo4::assets::db::DbClient;
use openfo4::assets::db::music::MusicTrackKind;

const EXPLORE_WORLDSPACE: &str = "00003C:Fallout4.esm";

#[test]
#[ignore = "needs a converted installation"]
fn vanilla_music_catalog_has_the_expected_shape() {
    let db = DbClient::new(common::database_path().to_str().unwrap()).unwrap();
    let catalog = db.get_music_catalog().unwrap();

    assert_eq!(catalog.music_types.len(), 111, "MUSC records");
    let count = |wanted: fn(&MusicTrackKind) -> bool| {
        catalog.tracks.iter().filter(|t| wanted(&t.kind)).count()
    };
    assert_eq!(catalog.tracks.len(), 552, "MUST records");
    assert_eq!(count(|k| matches!(k, MusicTrackKind::Single { .. })), 506);
    assert_eq!(count(|k| matches!(k, MusicTrackKind::Palette { .. })), 38);
    assert_eq!(count(|k| matches!(k, MusicTrackKind::Silent { .. })), 8);

    let music_bearing = catalog
        .cell_places
        .iter()
        .filter(|place| place.music_type.is_some())
        .count();
    assert!(music_bearing >= 281, "music-bearing cells: {music_bearing}");
    assert_eq!(catalog.location_music.len(), 67);

    let regions_without_worldspace = catalog
        .region_music
        .iter()
        .filter(|region| region.worldspace.is_none())
        .count();
    println!(
        "sound regions: {}, without a worldspace: {regions_without_worldspace}",
        catalog.region_music.len()
    );
    assert_eq!(catalog.globals.len(), 1346);

    let time_scale = &catalog.global_form_keys["TimeScale"];
    assert_ne!(catalog.globals[time_scale], 0.0);

    let explore = catalog
        .music_types
        .iter()
        .find(|t| t.editor_id.as_deref() == Some("MUSzExplore"))
        .expect("MUSzExplore");
    assert_eq!((explore.priority, explore.fade_seconds), (90, 12.0));
    assert_eq!(
        catalog.worldspace_music[EXPLORE_WORLDSPACE], explore.form_key,
        "worldspace 00003C plays MUSzExplore"
    );
}
