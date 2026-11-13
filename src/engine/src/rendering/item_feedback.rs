//! What the player hears of an inventory change (the pick-up sound of a taken item and the
//! put-down sound of a dropped one) and the line shown for an item too heavy to carry.

use std::path::Path;
use std::sync::Arc;

use super::game_text::required_nonblank_text;
use crate::assets::db::DbClient;
use crate::audio::{ItemSoundKind, ItemSounds};
use crate::sim::InventoryEvent;
use crate::startup::StartupError;

const TOO_HEAVY_SETTING: &str = "sItemTooHeavy";

/// The sound one inventory change asks for; the item key is borrowed from the event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ItemSound<'a> {
    pub item: &'a str,
    pub kind: ItemSoundKind,
}

/// Added -> pick-up sound, Dropped -> put-down sound.
pub(super) fn item_sound_of(event: &InventoryEvent) -> ItemSound<'_> {
    match event {
        InventoryEvent::Added { form_key, .. } => ItemSound {
            item: form_key,
            kind: ItemSoundKind::PickUp,
        },
        InventoryEvent::Dropped { form_key, .. } => ItemSound {
            item: form_key,
            kind: ItemSoundKind::PutDown,
        },
    }
}

/// The debug log line of one inventory change; the own-build check reads `added N x key`.
pub(super) fn inventory_log_line(event: &InventoryEvent) -> String {
    match event {
        InventoryEvent::Added { form_key, count } => {
            format!("inventory: added {count} x {form_key}")
        }
        InventoryEvent::Dropped { form_key, count } => {
            format!("inventory: dropped {count} x {form_key}")
        }
    }
}

/// The line shown when E is held on an item too heavy to carry. A missing or blank setting stops
/// startup.
pub(super) fn load_too_heavy_text(db: &DbClient) -> Result<Arc<str>, StartupError> {
    required_nonblank_text(db, TOO_HEAVY_SETTING).map_err(StartupError::TooHeavyText)
}

/// The item sounds of the database, or none: a catalog that cannot be read or used turns the item
/// sounds off for the run, reported once, and the game starts without them.
pub(super) fn load_item_sounds(db_path: &str, audio_dir: &Path, seed: u64) -> Option<ItemSounds> {
    let loaded = DbClient::new(db_path).and_then(|db| read_item_sounds(&db, audio_dir, seed));
    match loaded {
        Ok(sounds) => Some(sounds),
        Err(error) => {
            tracing::error!("item sounds are off: {error:#}");
            None
        }
    }
}

fn read_item_sounds(db: &DbClient, audio_dir: &Path, seed: u64) -> anyhow::Result<ItemSounds> {
    let catalog = db.get_item_sound_catalog()?;
    Ok(ItemSounds::new(catalog, audio_dir, seed)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::db::test_support::database_with;

    const KEY: &str = "0F3B01:Test.esm";

    #[test]
    fn an_added_item_plays_its_pick_up_sound() {
        let event = InventoryEvent::Added {
            form_key: KEY.to_owned(),
            count: 3,
        };
        assert_eq!(
            item_sound_of(&event),
            ItemSound {
                item: KEY,
                kind: ItemSoundKind::PickUp
            }
        );
    }

    #[test]
    fn a_dropped_item_plays_its_put_down_sound_whatever_the_count() {
        for count in [1, 40] {
            let event = InventoryEvent::Dropped {
                form_key: KEY.to_owned(),
                count,
            };
            assert_eq!(
                item_sound_of(&event),
                ItemSound {
                    item: KEY,
                    kind: ItemSoundKind::PutDown
                },
                "{count}"
            );
        }
    }

    #[test]
    fn the_log_lines_keep_the_plain_format_the_own_build_check_reads() {
        let added = InventoryEvent::Added {
            form_key: KEY.to_owned(),
            count: 3,
        };
        let dropped = InventoryEvent::Dropped {
            form_key: KEY.to_owned(),
            count: 30,
        };
        assert_eq!(
            inventory_log_line(&added),
            "inventory: added 3 x 0F3B01:Test.esm"
        );
        assert_eq!(
            inventory_log_line(&dropped),
            "inventory: dropped 30 x 0F3B01:Test.esm"
        );
    }

    fn too_heavy_setting(text: &str) -> String {
        format!(
            "INSERT INTO game_settings (form_key, editor_id, value_type, value_string)
             VALUES ('1:T.esm', '{TOO_HEAVY_SETTING}', 'string', '{text}');"
        )
    }

    #[test]
    fn the_too_heavy_line_is_the_setting_text_verbatim() {
        let text = "Too heavy for the test to lift.";
        let loaded = load_too_heavy_text(&database_with(&too_heavy_setting(text))).unwrap();
        assert_eq!(loaded.as_ref(), text);
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the converted database"]
    fn the_installed_database_has_the_too_heavy_line() {
        let db = crate::assets::db::test_support::installed_database();
        assert_eq!(
            load_too_heavy_text(&db).unwrap().as_ref(),
            "Too heavy for the test to lift."
        );
    }

    #[test]
    fn a_missing_too_heavy_setting_stops_startup_naming_it() {
        let error = load_too_heavy_text(&database_with("")).unwrap_err();
        assert!(matches!(error, StartupError::TooHeavyText(_)));
        assert!(error.to_string().contains(TOO_HEAVY_SETTING), "{error}");
    }

    #[test]
    fn a_blank_too_heavy_setting_stops_startup_naming_it() {
        let error = load_too_heavy_text(&database_with(&too_heavy_setting("  "))).unwrap_err();
        assert!(matches!(error, StartupError::TooHeavyText(_)));
        assert!(error.to_string().contains(TOO_HEAVY_SETTING), "{error}");
    }

    #[test]
    fn a_database_that_cannot_be_opened_leaves_the_item_sounds_off() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing.db");
        let sounds = load_item_sounds(missing.to_str().unwrap(), directory.path(), 1);
        assert!(sounds.is_none());
    }

    #[test]
    fn a_catalog_without_its_default_sounds_is_an_error_and_not_a_silent_success() {
        let directory = tempfile::tempdir().unwrap();
        let Err(error) = read_item_sounds(&database_with(""), directory.path(), 1) else {
            panic!("an empty database holds no default sound slots");
        };
        let text = format!("{error:#}");
        assert!(text.contains("default object slot"), "{text}");
    }
}
