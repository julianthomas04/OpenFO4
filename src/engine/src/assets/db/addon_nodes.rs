//! `ADDN` records: which model, light and caps a marker model's `BSValueNode`
//! value stands for.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, thiserror::Error)]
pub enum AddonNodeError {
    #[error("addon_nodes query failed: {0}")]
    Query(#[from] rusqlite::Error),
    #[error("addon node `{form_key}` has {field} {value}, which does not fit a u16")]
    OutOfRange {
        form_key: String,
        field: &'static str,
        value: i64,
    },
    #[error("addon node `{form_key}` has node_index {value}, which does not fit an i32")]
    IndexOutOfRange { form_key: String, value: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddonNode {
    pub form_key: String,
    /// `None` for an addon node that only spawns a light.
    pub model_path: Option<String>,
    /// The `BSValueNode` value this record answers to.
    pub node_index: i32,
    /// `lights.form_key` of the light spawned with it.
    pub light: Option<String>,
    /// Master particle system cap (ADDN `DNAM` u16); 0 when the record has none.
    pub cap: u16,
    /// Raw `DNAM` flags; 0 when the record has none.
    pub flags: u16,
}

/// The addon nodes of the database by `node_index`.
#[derive(Debug, Default)]
pub struct AddonNodes {
    by_index: HashMap<i32, AddonNode>,
    /// Rows without a `node_index`: nothing can reference them.
    pub rows_without_index: usize,
    /// Rows replaced by a later row with the same `node_index`.
    pub duplicate_indices: usize,
}

impl AddonNodes {
    pub fn get(&self, node_index: i32) -> Option<&AddonNode> {
        self.by_index.get(&node_index)
    }

    #[cfg(test)]
    pub(crate) fn from_rows(rows: impl IntoIterator<Item = AddonNode>) -> Self {
        Self {
            by_index: rows.into_iter().map(|row| (row.node_index, row)).collect(),
            ..Self::default()
        }
    }

    pub fn len(&self) -> usize {
        self.by_index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_index.is_empty()
    }

    /// Loads every row. A database converted before addon nodes were extracted
    /// has no table; it loads as empty (a warning is logged once).
    pub fn load(conn: &Connection) -> Result<Self, AddonNodeError> {
        let has_table: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'addon_nodes'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if has_table.is_none() {
            crate::utils::warn_once("addon_nodes-table-missing", || {
                "addon_nodes table missing (database predates addon node extraction); marker fires and addon lights are unavailable".to_owned()
            });
            return Ok(Self::default());
        }

        let mut stmt = conn.prepare(
            "SELECT form_key, model_path, node_index, light, master_particle_system_cap, flags
             FROM addon_nodes ORDER BY rowid",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(RawRow {
                form_key: row.get(0)?,
                model_path: row.get(1)?,
                node_index: row.get(2)?,
                light: row.get(3)?,
                cap: row.get(4)?,
                flags: row.get(5)?,
            })
        })?;

        let mut table = Self::default();
        for row in rows {
            table.insert(row?)?;
        }
        Ok(table)
    }

    fn insert(&mut self, row: RawRow) -> Result<(), AddonNodeError> {
        let Some(raw_index) = row.node_index else {
            self.rows_without_index += 1;
            return Ok(());
        };
        let node_index = i32::try_from(raw_index).map_err(|_| AddonNodeError::IndexOutOfRange {
            form_key: row.form_key.clone(),
            value: raw_index,
        })?;
        let node = AddonNode {
            cap: to_u16(&row.form_key, "master_particle_system_cap", row.cap)?,
            flags: to_u16(&row.form_key, "flags", row.flags)?,
            form_key: row.form_key,
            model_path: row.model_path,
            node_index,
            light: row.light,
        };
        if self.by_index.insert(node_index, node).is_some() {
            self.duplicate_indices += 1;
        }
        Ok(())
    }
}

struct RawRow {
    form_key: String,
    model_path: Option<String>,
    node_index: Option<i64>,
    light: Option<String>,
    cap: Option<i64>,
    flags: Option<i64>,
}

fn to_u16(form_key: &str, field: &'static str, value: Option<i64>) -> Result<u16, AddonNodeError> {
    let Some(value) = value else { return Ok(0) };
    u16::try_from(value).map_err(|_| AddonNodeError::OutOfRange {
        form_key: form_key.to_owned(),
        field,
        value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database(rows: &str) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE addon_nodes (
                form_key TEXT PRIMARY KEY, editor_id TEXT, model_path TEXT,
                node_index INTEGER, light TEXT, sound TEXT,
                master_particle_system_cap INTEGER, flags INTEGER);
             {rows}"
        ))
        .unwrap();
        conn
    }

    #[test]
    fn loads_rows_by_node_index() {
        let conn = database(
            "INSERT INTO addon_nodes VALUES
               ('0A:Fallout4.esm', 'MPSFireMed01', 'effects\\mpsfiremed01.nif', 5,
                '1EB2DB:Fallout4.esm', NULL, 20, 1),
               ('0B:Fallout4.esm', 'FXSmoke', NULL, 7, NULL, NULL, NULL, NULL);",
        );
        let table = AddonNodes::load(&conn).unwrap();
        assert_eq!(table.len(), 2);
        assert_eq!(
            table.get(5),
            Some(&AddonNode {
                form_key: "0A:Fallout4.esm".into(),
                model_path: Some("effects\\mpsfiremed01.nif".into()),
                node_index: 5,
                light: Some("1EB2DB:Fallout4.esm".into()),
                cap: 20,
                flags: 1,
            })
        );
        let light_only = table.get(7).unwrap();
        assert_eq!(light_only.model_path, None);
        assert_eq!((light_only.cap, light_only.flags), (0, 0));
        assert!(table.get(6).is_none());
    }

    #[test]
    fn row_without_node_index_is_counted_not_loaded() {
        let conn = database(
            "INSERT INTO addon_nodes (form_key, model_path) VALUES ('0A:x.esm', 'a.nif');",
        );
        let table = AddonNodes::load(&conn).unwrap();
        assert!(table.is_empty());
        assert_eq!(table.rows_without_index, 1);
    }

    #[test]
    fn later_row_with_the_same_index_wins_and_is_counted() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE addon_nodes (form_key TEXT, model_path TEXT, node_index INTEGER,
                light TEXT, master_particle_system_cap INTEGER, flags INTEGER);
             INSERT INTO addon_nodes VALUES ('first', 'a.nif', 3, NULL, 0, 0);
             INSERT INTO addon_nodes VALUES ('second', 'b.nif', 3, NULL, 0, 0);",
        )
        .unwrap();
        let table = AddonNodes::load(&conn).unwrap();
        assert_eq!(table.get(3).unwrap().form_key, "second");
        assert_eq!(table.duplicate_indices, 1);
    }

    #[test]
    fn missing_table_loads_empty() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(AddonNodes::load(&conn).unwrap().is_empty());
    }

    #[test]
    fn dnam_value_beyond_u16_is_an_error_naming_the_record() {
        let conn = database(
            "INSERT INTO addon_nodes VALUES ('0A:x.esm', NULL, NULL, 1, NULL, NULL, 70000, 0);",
        );
        let error = AddonNodes::load(&conn).unwrap_err();
        assert!(matches!(
            error,
            AddonNodeError::OutOfRange {
                field: "master_particle_system_cap",
                value: 70000,
                ..
            }
        ));
        assert!(error.to_string().contains("0A:x.esm"));
    }

    #[test]
    fn missing_column_surfaces_as_a_query_error() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE addon_nodes (form_key TEXT, model_path TEXT);")
            .unwrap();
        assert!(matches!(
            AddonNodes::load(&conn),
            Err(AddonNodeError::Query(_))
        ));
    }
}
