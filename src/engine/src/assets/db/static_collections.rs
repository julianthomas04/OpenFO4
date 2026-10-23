pub struct StaticCollection {
    pub form_key: Option<String>,
    pub editor_id: Option<String>,
    pub model_path: Option<String>,
    pub is_marker: bool,
    pub parts: Option<String>,
}

fn row_to_static_collection(row: &rusqlite::Row) -> rusqlite::Result<StaticCollection> {
    Ok(StaticCollection {
        form_key: row.get(0)?,
        editor_id: row.get(1)?,
        model_path: row.get(2)?,
        is_marker: row.get(3)?,
        parts: row.get(4)?,
    })
}

impl StaticCollection {
    pub fn get(form_key: &str, conn: &rusqlite::Connection) -> anyhow::Result<Self> {
        Ok(conn.query_row(
            "SELECT form_key, editor_id, model_path, is_marker, parts
             FROM static_collections WHERE form_key = ?",
            rusqlite::params![form_key],
            row_to_static_collection,
        )?)
    }

    pub fn get_all(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(
            "SELECT form_key, editor_id, model_path, is_marker, parts
             FROM static_collections",
        )?;

        Ok(stmt
            .query_map([], row_to_static_collection)?
            .collect::<Result<Vec<_>, _>>()?)
    }
}
