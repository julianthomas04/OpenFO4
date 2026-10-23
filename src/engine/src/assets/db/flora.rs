pub struct Flora {
    pub form_key: Option<String>,
    pub editor_id: Option<String>,
    pub name: Option<String>,
    pub model_path: Option<String>,
    pub harvest_sound: Option<String>,
    pub ingredient: Option<String>,
    pub flags: Option<String>,
    pub is_marker: bool,
}

fn row_to_flora(row: &rusqlite::Row) -> rusqlite::Result<Flora> {
    Ok(Flora {
        form_key: row.get(0)?,
        editor_id: row.get(1)?,
        name: row.get(2)?,
        model_path: row.get(3)?,
        harvest_sound: row.get(4)?,
        ingredient: row.get(5)?,
        flags: row.get(6)?,
        is_marker: row.get(7)?,
    })
}

impl Flora {
    pub fn get(form_key: &str, conn: &rusqlite::Connection) -> anyhow::Result<Self> {
        Ok(conn.query_row(
            "SELECT form_key, editor_id, name, model_path,
                    harvest_sound, ingredient, flags, is_marker
             FROM flora WHERE form_key = ?",
            rusqlite::params![form_key],
            row_to_flora,
        )?)
    }

    pub fn get_all(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(
            "SELECT form_key, editor_id, name, model_path,
                    harvest_sound, ingredient, flags, is_marker
             FROM flora",
        )?;

        Ok(stmt
            .query_map([], row_to_flora)?
            .collect::<Result<Vec<_>, _>>()?)
    }
}
