pub struct Container {
    pub form_key: Option<String>,
    pub editor_id: Option<String>,
    pub name: Option<String>,
    pub model_path: Option<String>,
    pub open_sound: Option<String>,
    pub close_sound: Option<String>,
    pub flags: Option<String>,
    pub items: Option<String>,
    pub is_marker: bool,
}

fn row_to_container(row: &rusqlite::Row) -> rusqlite::Result<Container> {
    Ok(Container {
        form_key: row.get(0)?,
        editor_id: row.get(1)?,
        name: row.get(2)?,
        model_path: row.get(3)?,
        open_sound: row.get(4)?,
        close_sound: row.get(5)?,
        flags: row.get(6)?,
        items: row.get(7)?,
        is_marker: row.get(8)?,
    })
}

impl Container {
    pub fn get(form_key: &str, conn: &rusqlite::Connection) -> anyhow::Result<Self> {
        Ok(conn.query_row(
            "SELECT form_key, editor_id, name, model_path,
                    open_sound, close_sound, flags, items, is_marker
             FROM containers WHERE form_key = ?",
            rusqlite::params![form_key],
            row_to_container,
        )?)
    }

    pub fn get_all(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(
            "SELECT form_key, editor_id, name, model_path,
                    open_sound, close_sound, flags, items, is_marker
             FROM containers",
        )?;

        Ok(stmt
            .query_map([], row_to_container)?
            .collect::<Result<Vec<_>, _>>()?)
    }
}
