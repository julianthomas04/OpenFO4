//! The sounds of picking an item up and putting it down: the record's own sound descriptors and the
//! default-object descriptors an item falls back on. Values are form keys; playback belongs to
//! `audio::item_sounds`.

use std::collections::HashMap;

use anyhow::Context;
use rusqlite::Connection;

use crate::assets::db::rows::{DefaultObjectSlot, collect_rows, default_object_slot, required};
use crate::assets::db::sound_catalog::SoundCatalog;

/// The default-object pair an item falls back to. The four letter pairs exist in the data (PUSW/PDSW,
/// PUSA/PDSA, PUSB/PDSB, PUSG/PDSG) and match the public slot names for weapon, armor, book and generic; that WEAP, ARMO and BOOK use their own slots
/// follows from those names. That ALCH, AMMO, MISC, HOLT and KEYM use the generic slot is the stand-in: no slot of theirs exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DefaultSoundSlot {
    Weapon,
    Armor,
    Book,
    Generic,
}

impl DefaultSoundSlot {
    const ALL: [Self; 4] = [Self::Weapon, Self::Armor, Self::Book, Self::Generic];

    /// WEAP Weapon, ARMO Armor, BOOK Book; ALCH, AMMO, MISC, HOLT, KEYM Generic; any other signature None.
    pub fn of_record(item_type: &str) -> Option<Self> {
        match item_type {
            "WEAP" => Some(Self::Weapon),
            "ARMO" => Some(Self::Armor),
            "BOOK" => Some(Self::Book),
            "ALCH" | "AMMO" | "MISC" | "HOLT" | "KEYM" => Some(Self::Generic),
            _ => None,
        }
    }

    pub fn pickup_slot(self) -> &'static str {
        match self {
            Self::Weapon => "PUSW",
            Self::Armor => "PUSA",
            Self::Book => "PUSB",
            Self::Generic => "PUSG",
        }
    }

    pub fn put_down_slot(self) -> &'static str {
        match self {
            Self::Weapon => "PDSW",
            Self::Armor => "PDSA",
            Self::Book => "PDSB",
            Self::Generic => "PDSG",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemSoundEntry {
    pub slot: DefaultSoundSlot,
    /// The record's own descriptors; each side falls back on its own (the data has one without the other).
    pub pickup: Option<String>,
    pub put_down: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemSoundCatalog {
    /// Item base form key -> its sounds; only record types `DefaultSoundSlot::of_record` accepts.
    pub items: HashMap<String, ItemSoundEntry>,
    /// Slot letters -> descriptor form key, for the eight slots of the four `DefaultSoundSlot`s.
    pub defaults: HashMap<&'static str, String>,
    /// Only the descriptors reachable from the entries and the defaults.
    pub sounds: SoundCatalog,
}

impl ItemSoundCatalog {
    /// One `SELECT form_key, item_type, pickup_sound, drop_sound FROM items`; the eight slots through `default_object_slots`;
    /// then `SoundCatalog::load_reachable`. Errors: a slot that is absent or unassigned, a descriptor the sound tables do not
    /// hold. NULL or blank own sound = no own sound.
    pub(crate) fn from_connection(conn: &Connection) -> anyhow::Result<Self> {
        let items = load_items(conn)?;
        let defaults = load_defaults(conn)?;
        let roots = items
            .values()
            .flat_map(|entry| [&entry.pickup, &entry.put_down])
            .flatten()
            .chain(defaults.values())
            .cloned();
        let sounds = SoundCatalog::load_reachable(conn, roots)?;
        Ok(Self {
            items,
            defaults,
            sounds,
        })
    }
}

fn load_items(conn: &Connection) -> anyhow::Result<HashMap<String, ItemSoundEntry>> {
    let rows = collect_rows(
        conn,
        "SELECT form_key, item_type, pickup_sound, drop_sound FROM items ORDER BY form_key",
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        },
    )?;
    Ok(rows
        .into_iter()
        .filter_map(|(form_key, item_type, pickup, put_down)| {
            let slot = DefaultSoundSlot::of_record(&item_type)?;
            let entry = ItemSoundEntry {
                slot,
                pickup: non_blank(pickup),
                put_down: non_blank(put_down),
            };
            Some((form_key, entry))
        })
        .collect())
}

fn non_blank(sound: Option<String>) -> Option<String> {
    sound.filter(|key| !key.trim().is_empty())
}

fn load_defaults(conn: &Connection) -> anyhow::Result<HashMap<&'static str, String>> {
    DefaultSoundSlot::ALL
        .into_iter()
        .flat_map(|slot| [slot.pickup_slot(), slot.put_down_slot()])
        .map(|slot| Ok::<_, anyhow::Error>((slot, load_default(conn, slot)?)))
        .collect()
}

fn load_default(conn: &Connection, slot: &str) -> anyhow::Result<String> {
    let assigned = default_object_slot(conn, slot)
        .with_context(|| format!("reading default object slot {slot}"))?;
    match assigned {
        DefaultObjectSlot::Absent => anyhow::bail!("default object slot {slot} is absent"),
        DefaultObjectSlot::Empty => {
            required(None, "target", &format!("default object slot {slot}"))
        }
        DefaultObjectSlot::Target(target) => Ok(target),
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::types::Value;

    use super::*;

    const SCHEMA: &str = include_str!("../../../../installer/file_converters/esm_schema.sql");

    const PISTOL: &str = "000A01:Test.esm";
    const JACKET: &str = "000A02:Test.esm";
    const TONIC: &str = "000A03:Test.esm";
    const BLANK_SOUNDS: &str = "000A04:Test.esm";
    const TOMATO: &str = "000A05:Test.esm";
    const PISTOL_UP: &str = "000B01:Test.esm";
    const TONIC_DOWN: &str = "000B02:Test.esm";
    const UNREACHED: &str = "000B03:Test.esm";
    const MANAGER: &str = "000001:Test.esm";

    fn text(value: &str) -> Value {
        Value::Text(value.to_string())
    }

    fn insert(conn: &Connection, table: &str, row: &[(&str, Value)]) {
        let columns = row
            .iter()
            .map(|(column, _)| *column)
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders = vec!["?"; row.len()].join(", ");
        conn.execute(
            &format!("INSERT INTO {table} ({columns}) VALUES ({placeholders})"),
            rusqlite::params_from_iter(row.iter().map(|(_, value)| value)),
        )
        .unwrap();
    }

    fn descriptor(conn: &Connection, key: &str) {
        insert(
            conn,
            "sound_descriptors",
            &[
                ("form_key", text(key)),
                ("descriptor_type", Value::Integer(0x1EEF_540A)),
                ("freq_shift_pct", Value::Integer(0)),
                ("freq_variance_pct", Value::Integer(0)),
                ("priority", Value::Integer(0)),
                ("db_variance", Value::Integer(0)),
                ("static_attenuation_centidb", Value::Integer(0)),
                ("looping_flags", Value::Integer(0)),
            ],
        );
    }

    fn item(conn: &Connection, key: &str, item_type: &str, pickup: Value, put_down: Value) {
        insert(
            conn,
            "items",
            &[
                ("form_key", text(key)),
                ("item_type", text(item_type)),
                ("pickup_sound", pickup),
                ("drop_sound", put_down),
            ],
        );
    }

    fn slot_descriptor(slot: &str) -> String {
        format!("{slot}:Test.esm")
    }

    fn assign_slot(conn: &Connection, slot: &str, target: Value) {
        insert(
            conn,
            "default_object_slots",
            &[
                ("manager", text(MANAGER)),
                ("slot", text(slot)),
                ("target", target),
            ],
        );
    }

    /// Eight slots, each with its own descriptor; a pistol with a pick-up sound only, a jacket and a
    /// tonic that differ in which side they own, a record with blank sounds, a tomato (INGR) and an
    /// unreferenced descriptor.
    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        for slot in DefaultSoundSlot::ALL
            .into_iter()
            .flat_map(|slot| [slot.pickup_slot(), slot.put_down_slot()])
        {
            descriptor(&conn, &slot_descriptor(slot));
            assign_slot(&conn, slot, text(&slot_descriptor(slot)));
        }
        for key in [PISTOL_UP, TONIC_DOWN, UNREACHED] {
            descriptor(&conn, key);
        }
        item(&conn, PISTOL, "WEAP", text(PISTOL_UP), Value::Null);
        item(&conn, JACKET, "ARMO", Value::Null, Value::Null);
        item(&conn, TONIC, "ALCH", Value::Null, text(TONIC_DOWN));
        item(&conn, BLANK_SOUNDS, "KEYM", text("  "), text(""));
        item(&conn, TOMATO, "INGR", text(UNREACHED), text(UNREACHED));
        conn
    }

    mod slots {
        use super::*;

        #[test]
        fn every_inventory_record_type_has_the_slot_its_name_implies() {
            let cases = [
                ("WEAP", DefaultSoundSlot::Weapon),
                ("ARMO", DefaultSoundSlot::Armor),
                ("BOOK", DefaultSoundSlot::Book),
                ("ALCH", DefaultSoundSlot::Generic),
                ("AMMO", DefaultSoundSlot::Generic),
                ("MISC", DefaultSoundSlot::Generic),
                ("HOLT", DefaultSoundSlot::Generic),
                ("KEYM", DefaultSoundSlot::Generic),
            ];
            for (item_type, slot) in cases {
                assert_eq!(DefaultSoundSlot::of_record(item_type), Some(slot));
            }
        }

        #[test]
        fn other_signatures_and_other_spellings_have_no_slot() {
            for item_type in ["INGR", "NPC_", "weap", "", "WEAP "] {
                assert_eq!(DefaultSoundSlot::of_record(item_type), None, "{item_type}");
            }
        }

        #[test]
        fn the_letters_pair_each_slot_with_its_own_put_down_slot() {
            let letters: Vec<_> = DefaultSoundSlot::ALL
                .into_iter()
                .map(|slot| (slot.pickup_slot(), slot.put_down_slot()))
                .collect();
            assert_eq!(
                letters,
                [
                    ("PUSW", "PDSW"),
                    ("PUSA", "PDSA"),
                    ("PUSB", "PDSB"),
                    ("PUSG", "PDSG"),
                ]
            );
        }
    }

    mod loading {
        use super::*;

        fn catalog() -> ItemSoundCatalog {
            ItemSoundCatalog::from_connection(&connection()).unwrap()
        }

        #[test]
        fn each_side_keeps_only_what_the_record_owns() {
            let catalog = catalog();
            assert_eq!(
                catalog.items[PISTOL],
                ItemSoundEntry {
                    slot: DefaultSoundSlot::Weapon,
                    pickup: Some(PISTOL_UP.to_string()),
                    put_down: None,
                }
            );
            assert_eq!(
                catalog.items[TONIC],
                ItemSoundEntry {
                    slot: DefaultSoundSlot::Generic,
                    pickup: None,
                    put_down: Some(TONIC_DOWN.to_string()),
                }
            );
            assert_eq!(catalog.items[JACKET].slot, DefaultSoundSlot::Armor);
        }

        #[test]
        fn blank_own_sounds_are_no_own_sounds() {
            let entry = &catalog().items[BLANK_SOUNDS];
            assert_eq!((&entry.pickup, &entry.put_down), (&None, &None));
        }

        #[test]
        fn a_record_type_without_slot_is_left_out() {
            assert!(!catalog().items.contains_key(TOMATO));
        }

        #[test]
        fn the_eight_defaults_are_loaded_by_their_slot_letters() {
            let catalog = catalog();
            assert_eq!(catalog.defaults.len(), 8);
            for slot in [
                "PUSW", "PDSW", "PUSA", "PDSA", "PUSB", "PDSB", "PUSG", "PDSG",
            ] {
                assert_eq!(catalog.defaults[slot], slot_descriptor(slot), "{slot}");
            }
        }

        #[test]
        fn only_the_reachable_descriptors_are_loaded() {
            let catalog = catalog();
            let mut keys: Vec<&str> = catalog
                .sounds
                .descriptors
                .keys()
                .map(String::as_str)
                .collect();
            keys.sort_unstable();
            assert_eq!(keys.len(), 10);
            assert!(keys.contains(&PISTOL_UP) && keys.contains(&TONIC_DOWN));
            assert!(!keys.contains(&UNREACHED));
        }

        #[test]
        fn an_absent_slot_is_an_error_naming_it() {
            let conn = connection();
            conn.execute("DELETE FROM default_object_slots WHERE slot = 'PDSB'", [])
                .unwrap();
            let error = ItemSoundCatalog::from_connection(&conn).unwrap_err();
            assert!(format!("{error:#}").contains("PDSB"), "{error:#}");
        }

        #[test]
        fn an_unassigned_slot_is_an_error_naming_it() {
            let conn = connection();
            conn.execute(
                "UPDATE default_object_slots SET target = NULL WHERE slot = 'PUSW'",
                [],
            )
            .unwrap();
            let error = ItemSoundCatalog::from_connection(&conn).unwrap_err();
            let message = format!("{error:#}");
            assert!(
                message.contains("PUSW") && message.contains("NULL"),
                "{message}"
            );
        }

        #[test]
        fn an_own_sound_the_sound_tables_do_not_hold_is_an_error() {
            let conn = connection();
            item(
                &conn,
                "000A09:Test.esm",
                "MISC",
                text("0DEAD0:Test.esm"),
                Value::Null,
            );
            let error = ItemSoundCatalog::from_connection(&conn).unwrap_err();
            assert!(format!("{error:#}").contains("0DEAD0"), "{error:#}");
        }
    }
}
