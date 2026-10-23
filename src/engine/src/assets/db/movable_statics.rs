pub struct MovableStatic {
    pub form_key: Option<String>,
    pub editor_id: Option<String>,
    pub model_path: Option<String>,
    pub destruction_data: Option<String>,
    pub is_marker: bool,
}

fn row_to_movable_static(row: &rusqlite::Row) -> rusqlite::Result<MovableStatic> {
    Ok(MovableStatic {
        form_key: row.get(0)?,
        editor_id: row.get(1)?,
        model_path: row.get(2)?,
        destruction_data: row.get(3)?,
        is_marker: row.get(4)?,
    })
}

impl MovableStatic {
    pub fn get(form_key: &str, conn: &rusqlite::Connection) -> anyhow::Result<Self> {
        Ok(conn.query_row(
            "SELECT form_key, editor_id, model_path, destruction_data, is_marker
             FROM movable_statics WHERE form_key = ?",
            rusqlite::params![form_key],
            row_to_movable_static,
        )?)
    }

    pub fn get_all(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(
            "SELECT form_key, editor_id, model_path, destruction_data, is_marker
             FROM movable_statics",
        )?;

        Ok(stmt
            .query_map([], row_to_movable_static)?
            .collect::<Result<Vec<_>, _>>()?)
    }
}
