//! The item pick-up and put-down sounds of a converted installation: every descriptor resolves as a
//! one-shot and no placed item base is left without a sound on either side.
//!
//! Ignored by default (needs an installation converted with the item and sound tables and audio).
//! Run with `cargo test --test item_sound_real_install -- --ignored --nocapture`.

mod common;

use std::collections::BTreeSet;
use std::path::PathBuf;

use openfo4::assets::db::DbClient;
use openfo4::assets::db::item_sounds::ItemSoundCatalog;
use openfo4::assets::layout::audio_dir;
use openfo4::audio::item_sounds::{ItemSoundKind, ItemSounds};
use rusqlite::{Connection, OpenFlags};

const SEED: u64 = 1;
const PLACED_ITEM_BASES: usize = 1312;
const SLOT_COUNT: usize = 8;

fn catalog() -> ItemSoundCatalog {
    let db = DbClient::new(common::database_path().to_str().unwrap()).unwrap();
    db.get_item_sound_catalog().unwrap()
}

fn audio_folder() -> PathBuf {
    PathBuf::from(audio_dir(common::asset_root().to_str().unwrap()))
}

fn placed_item_bases() -> BTreeSet<String> {
    let conn =
        Connection::open_with_flags(common::database_path(), OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let mut statement = conn
        .prepare(
            "SELECT DISTINCT po.base_object FROM placed_objects po \
             JOIN items i ON i.form_key = po.base_object",
        )
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
#[ignore = "needs a converted installation"]
fn every_descriptor_of_the_catalog_resolves_as_a_one_shot() {
    let catalog = catalog();
    assert_eq!(catalog.defaults.len(), SLOT_COUNT);
    ItemSounds::new(catalog, &audio_folder(), SEED).unwrap();
}

#[test]
#[ignore = "needs a converted installation"]
fn every_placed_item_base_has_a_sound_on_both_sides() {
    let catalog = catalog();
    let bases = placed_item_bases();
    assert_eq!(bases.len(), PLACED_ITEM_BASES);
    let mut sounds = ItemSounds::new(catalog.clone(), &audio_folder(), SEED).unwrap();
    let silent: Vec<(&String, ItemSoundKind)> = bases
        .iter()
        .flat_map(|base| [ItemSoundKind::PickUp, ItemSoundKind::PutDown].map(|kind| (base, kind)))
        .filter(|(base, kind)| {
            assert!(
                catalog.items.contains_key(*base),
                "placed item base {base} is not in the item sound catalog"
            );
            sounds.sound(base, *kind).unwrap().is_none()
        })
        .collect();
    assert_eq!(silent, [], "placed item bases left silent");
}
