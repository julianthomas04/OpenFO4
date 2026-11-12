//! The leveled item lists (`leveled_items`): chance none, flags and the entries a list rolls from.

use anyhow::Context;
use serde::Deserialize;

const KNOWN_FLAG_BITS: u8 = 0b111;
const MAX_CHANCE_NONE: u8 = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeveledEntryRow {
    pub target: String,
    pub level: u32,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeveledItemRow {
    pub form_key: String,
    pub editor_id: Option<String>,
    /// Percent, 0..=100.
    pub chance_none: u8,
    /// Bits 1, 2, 4 (Mutagen: `CalculateFromAllLevelsLessThanOrEqualPlayer`,
    /// `CalculateForEachItemInCount`, `UseAll`); the database stores the enum number as text.
    pub flags: u8,
    /// A NULL column reads as an empty list.
    pub entries: Vec<LeveledEntryRow>,
}

#[derive(Deserialize)]
struct StoredEntry {
    #[serde(rename = "ref")]
    target: String,
    level: u32,
    count: u32,
}

struct StoredList {
    form_key: String,
    editor_id: Option<String>,
    chance_none: Option<i64>,
    flags: Option<String>,
    entries: Option<String>,
}

impl LeveledItemRow {
    /// One malformed row (chance none, flags, entries) fails the whole read with an error naming
    /// the list; nothing is skipped.
    pub fn list_all(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(
            "SELECT form_key, editor_id, chance_none, flags, entries FROM leveled_items ORDER BY form_key",
        )?;
        let stored = stmt
            .query_map([], |row| {
                Ok(StoredList {
                    form_key: row.get(0)?,
                    editor_id: row.get(1)?,
                    chance_none: row.get(2)?,
                    flags: row.get(3)?,
                    entries: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .context("reading the leveled item lists")?;
        stored.into_iter().map(Self::from_stored).collect()
    }

    fn from_stored(stored: StoredList) -> anyhow::Result<Self> {
        let list = match &stored.editor_id {
            Some(editor_id) => format!("{} ({editor_id})", stored.form_key),
            None => stored.form_key.clone(),
        };
        let stored_chance = stored
            .chance_none
            .with_context(|| format!("leveled list {list} has no chance none"))?;
        let chance_none = u8::try_from(stored_chance)
            .ok()
            .filter(|&percent| percent <= MAX_CHANCE_NONE)
            .with_context(|| format!("leveled list {list} has a chance none of {stored_chance}%"))?;
        let stored_flags = stored
            .flags
            .as_deref()
            .with_context(|| format!("leveled list {list} has no flags"))?;
        let flags: u8 = stored_flags
            .trim()
            .parse()
            .with_context(|| format!("leveled list {list} has the flags '{stored_flags}'"))?;
        anyhow::ensure!(
            flags & !KNOWN_FLAG_BITS == 0,
            "leveled list {list} has the unknown flag bits {:#b}",
            flags & !KNOWN_FLAG_BITS
        );
        let entries = match stored.entries.as_deref() {
            None => Vec::new(),
            Some(json) => serde_json::from_str::<Vec<StoredEntry>>(json)
                .with_context(|| format!("leveled list {list} has malformed entries"))?
                .into_iter()
                .map(|entry| LeveledEntryRow {
                    target: entry.target,
                    level: entry.level,
                    count: entry.count,
                })
                .collect(),
        };
        Ok(Self {
            editor_id: stored.editor_id,
            form_key: stored.form_key,
            chance_none,
            flags,
            entries,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::db::test_support::database_with;

    fn read(statements: &str) -> anyhow::Result<Vec<LeveledItemRow>> {
        LeveledItemRow::list_all(&database_with(statements).conn)
    }

    fn entry(target: &str, level: u32, count: u32) -> LeveledEntryRow {
        LeveledEntryRow {
            target: target.to_owned(),
            level,
            count,
        }
    }

    #[test]
    fn a_stored_list_reads_chance_flags_and_entries_in_order() {
        let rows = read(
            r#"INSERT INTO leveled_items (form_key, editor_id, chance_none, flags, entries) VALUES
               ('0A:T.esm', 'LL_Test', 80, '2',
                '[{"ref":"01:T.esm","level":1,"count":1},{"ref":"02:T.esm","level":11,"count":3}]')"#,
        )
        .unwrap();
        assert_eq!(
            rows,
            vec![LeveledItemRow {
                form_key: "0A:T.esm".into(),
                editor_id: Some("LL_Test".into()),
                chance_none: 80,
                flags: 2,
                entries: vec![entry("01:T.esm", 1, 1), entry("02:T.esm", 11, 3)],
            }]
        );
    }

    #[test]
    fn a_null_entries_column_is_an_empty_list() {
        let rows = read(
            "INSERT INTO leveled_items (form_key, chance_none, flags, entries) VALUES ('0A:T.esm', 0, '0', NULL)",
        )
        .unwrap();
        assert_eq!(rows[0].entries, Vec::new());
        assert_eq!(rows[0].editor_id, None);
    }

    #[test]
    fn the_known_flag_bits_combine() {
        let rows = read(
            "INSERT INTO leveled_items (form_key, chance_none, flags, entries) VALUES ('0A:T.esm', 100, '7', '[]')",
        )
        .unwrap();
        assert_eq!((rows[0].chance_none, rows[0].flags), (100, 7));
    }

    #[test]
    fn a_malformed_row_fails_the_whole_read_naming_the_list() {
        let cases = [
            ("101", "'0'", "'[]'", "chance none of 101%"),
            ("-1", "'0'", "'[]'", "chance none of -1%"),
            ("NULL", "'0'", "'[]'", "has no chance none"),
            ("0", "NULL", "'[]'", "has no flags"),
            ("0", "'8'", "'[]'", "unknown flag bits"),
            ("0", "'two'", "'[]'", "the flags 'two'"),
            ("0", "'256'", "'[]'", "the flags '256'"),
            ("0", "'0'", "'not json'", "malformed entries"),
            (
                "0",
                "'0'",
                r#"'[{"ref":"01:T.esm","level":-1,"count":1}]'"#,
                "malformed entries",
            ),
        ];
        for (chance, flags, entries, expected) in cases {
            let error = read(&format!(
                "INSERT INTO leveled_items (form_key, chance_none, flags, entries) VALUES ('0B:T.esm', 0, '0', '[]');
                 INSERT INTO leveled_items (form_key, chance_none, flags, entries) VALUES ('0A:T.esm', {chance}, {flags}, {entries})"
            ))
            .unwrap_err();
            let message = format!("{error:#}");
            assert!(message.contains("0A:T.esm"), "{message}");
            assert!(message.contains(expected), "{message}");
        }
    }

    /// Every list a placed reference links to loads and resolves at the starting level: a list
    /// without chance none never comes up empty, and the two lists with one (25 and 80) carry the
    /// installer's LVLD values.
    #[test]
    #[ignore = "needs an installation converted with leveled_items.chance_none (OPENFO4_INSTALL_DIR or OPENFO4_DB)"]
    fn every_list_a_placed_reference_links_to_resolves_at_the_starting_level() {
        use std::collections::HashSet;

        use crate::leveled_items::{LeveledLists, resolve};
        use crate::random::SplitMix64;
        use crate::sim::pipboy::character::STARTING_LEVEL;

        let db = crate::assets::db::test_support::installed_database();
        let lists = LeveledLists::from_rows(db.get_leveled_items().unwrap());
        let items: HashSet<String> = db
            .get_items()
            .unwrap()
            .into_iter()
            .filter(|item| item.model_path.is_some() && item.is_marker != Some(true))
            .map(|item| item.form_key)
            .collect();
        let mut stmt = db
            .conn
            .prepare(
                "SELECT DISTINCT leveled_item FROM placed_objects WHERE leveled_item IS NOT NULL",
            )
            .unwrap();
        let linked = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            linked.len(),
            131,
            "distinct lists linked from placed references"
        );

        let is_item = |key: &str| items.contains(key);
        let mut chances_none = Vec::new();
        for key in &linked {
            let list = lists
                .get(key)
                .unwrap_or_else(|| panic!("{key} is linked but not loaded"));
            if list.chance_none > 0 {
                chances_none.push(list.chance_none);
            }
            for seed in 0..64 {
                let mut rng = SplitMix64::new(seed);
                let stack = resolve(&lists, &is_item, key, STARTING_LEVEL, &mut rng)
                    .unwrap_or_else(|error| panic!("{key} at seed {seed}: {error}"));
                assert!(
                    stack.is_some() || list.chance_none > 0,
                    "{key} came up empty at seed {seed} without a chance none"
                );
            }
        }
        chances_none.sort_unstable();
        assert_eq!(chances_none, vec![25, 80]);
    }
}
