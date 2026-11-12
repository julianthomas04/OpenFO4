//! A small item sound catalog for tests: the four default slots with a descriptor per side, and five
//! items that differ in which side they own a sound for.

use std::collections::HashMap;

use super::footsteps::{catalog as footstep_catalog, standard};
use crate::assets::db::item_sounds::{DefaultSoundSlot, ItemSoundCatalog, ItemSoundEntry};
use crate::assets::db::sound_catalog::SoundCatalog;

/// A weapon with its own pick-up sound and no put-down sound.
pub const PISTOL: &str = "item_pistol";
/// Armor with its own put-down sound and no pick-up sound.
pub const JACKET: &str = "item_jacket";
/// A book that owns no sound.
pub const NOTE: &str = "item_note";
/// A generic item that owns both sounds.
pub const TONIC: &str = "item_tonic";
/// A generic item that owns no sound.
pub const SCRAP: &str = "item_scrap";
pub const UNKNOWN_ITEM: &str = "item_unknown";

pub const PISTOL_PICKUP_FILE: &str = "sound/fx/itm/pistol_up.flac";
pub const JACKET_PUT_DOWN_FILE: &str = "sound/fx/itm/jacket_down.flac";
pub const TONIC_PICKUP_FILE: &str = "sound/fx/itm/tonic_up.flac";
pub const TONIC_PUT_DOWN_FILE: &str = "sound/fx/itm/tonic_down.flac";

/// The file of the default descriptor of `slot` letters, for example `"PUSW"`.
pub fn default_file(slot: &str) -> String {
    format!("sound/fx/itm/default_{}.flac", slot.to_lowercase())
}

pub fn default_descriptor(slot: &str) -> String {
    format!("default_{slot}")
}

pub const PISTOL_PICKUP: &str = "pistol_up";
pub const JACKET_PUT_DOWN: &str = "jacket_down";
pub const TONIC_PICKUP: &str = "tonic_up";
pub const TONIC_PUT_DOWN: &str = "tonic_down";

fn slot_letters() -> impl Iterator<Item = &'static str> {
    [
        DefaultSoundSlot::Weapon,
        DefaultSoundSlot::Armor,
        DefaultSoundSlot::Book,
        DefaultSoundSlot::Generic,
    ]
    .into_iter()
    .flat_map(|slot| [slot.pickup_slot(), slot.put_down_slot()])
}

/// Every file the catalog's entries and defaults can reach.
pub fn all_files() -> Vec<String> {
    let mut files: Vec<String> = slot_letters().map(default_file).collect();
    files.extend(
        [
            PISTOL_PICKUP_FILE,
            JACKET_PUT_DOWN_FILE,
            TONIC_PICKUP_FILE,
            TONIC_PUT_DOWN_FILE,
        ]
        .map(String::from),
    );
    files
}

fn entry(slot: DefaultSoundSlot, pickup: Option<&str>, put_down: Option<&str>) -> ItemSoundEntry {
    ItemSoundEntry {
        slot,
        pickup: pickup.map(String::from),
        put_down: put_down.map(String::from),
    }
}

pub fn catalog() -> ItemSoundCatalog {
    let own = [
        (PISTOL_PICKUP, PISTOL_PICKUP_FILE),
        (JACKET_PUT_DOWN, JACKET_PUT_DOWN_FILE),
        (TONIC_PICKUP, TONIC_PICKUP_FILE),
        (TONIC_PUT_DOWN, TONIC_PUT_DOWN_FILE),
    ]
    .map(|(key, file)| standard(key, &[file], vec![]));
    let defaults = slot_letters().map(|slot| {
        let file = default_file(slot);
        standard(&default_descriptor(slot), &[file.as_str()], vec![])
    });
    let models_and_categories = footstep_catalog().sounds;
    ItemSoundCatalog {
        items: HashMap::from([
            (
                PISTOL.to_string(),
                entry(DefaultSoundSlot::Weapon, Some(PISTOL_PICKUP), None),
            ),
            (
                JACKET.to_string(),
                entry(DefaultSoundSlot::Armor, None, Some(JACKET_PUT_DOWN)),
            ),
            (NOTE.to_string(), entry(DefaultSoundSlot::Book, None, None)),
            (
                TONIC.to_string(),
                entry(
                    DefaultSoundSlot::Generic,
                    Some(TONIC_PICKUP),
                    Some(TONIC_PUT_DOWN),
                ),
            ),
            (
                SCRAP.to_string(),
                entry(DefaultSoundSlot::Generic, None, None),
            ),
        ]),
        defaults: slot_letters()
            .map(|slot| (slot, default_descriptor(slot)))
            .collect(),
        sounds: SoundCatalog {
            descriptors: own
                .into_iter()
                .chain(defaults)
                .map(|descriptor| (descriptor.form_key.clone(), descriptor))
                .collect(),
            ..models_and_categories
        },
    }
}
