//! WEAP bases and the model data of their default object-template mods: what a
//! placed weapon is assembled from (`assets::types::weapon_assembly`).

use anyhow::Context;

use crate::assets::types::weapon_assembly::ModRow;

/// Form keys per `IN` list; far below `SQLite`'s bound-variable limit.
const KEY_CHUNK: usize = 500;

pub struct WeaponRow {
    pub form_key: String,
    pub editor_id: Option<String>,
    /// The base model: a dummy root for guns, the whole weapon otherwise.
    pub model_path: Option<String>,
    /// OMOD form keys of the default combination; `None` when the weapon has
    /// no default combination.
    pub default_mods: Option<Vec<String>>,
}

impl WeaponRow {
    pub fn get_all(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(
            "SELECT i.form_key, i.editor_id, i.model_path, w.default_mods
             FROM weapons w JOIN items i ON i.form_key = w.form_key",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (form_key, editor_id, model_path, default_mods) = row?;
            let default_mods = default_mods
                .map(|json| serde_json::from_str(&json))
                .transpose()
                .with_context(|| {
                    format!("weapon {form_key}: default_mods is not a JSON array of form keys")
                })?;
            Ok(Self {
                form_key,
                editor_id,
                model_path,
                default_mods,
            })
        })
        .collect()
    }
}

pub struct ModModelRow {
    pub form_key: String,
    pub row: ModRow,
}

impl ModModelRow {
    pub fn get_for_keys(
        form_keys: &[String],
        conn: &rusqlite::Connection,
    ) -> anyhow::Result<Vec<Self>> {
        let mut found = Vec::new();
        for chunk in form_keys.chunks(KEY_CHUNK) {
            let placeholders = vec!["?"; chunk.len()].join(", ");
            let mut stmt = conn.prepare(&format!(
                "SELECT form_key, model_path, attach_point_name
                 FROM object_modifications WHERE form_key IN ({placeholders})"
            ))?;
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk), |row| {
                Ok(Self {
                    form_key: row.get(0)?,
                    row: ModRow {
                        model_path: row.get(1)?,
                        attach_point_name: row.get(2)?,
                    },
                })
            })?;
            for row in rows {
                found.push(row?);
            }
        }
        Ok(found)
    }
}
