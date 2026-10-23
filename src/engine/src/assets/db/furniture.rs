pub struct Furniture {
    pub form_key: Option<String>,
    pub editor_id: Option<String>,
    pub name: Option<String>,
    pub model_path: Option<String>,
    pub furniture_flags: Option<String>,
    pub markers: Option<String>,
    pub is_marker: bool,
}

fn row_to_furniture(row: &rusqlite::Row) -> rusqlite::Result<Furniture> {
    Ok(Furniture {
        form_key: row.get(0)?,
        editor_id: row.get(1)?,
        name: row.get(2)?,
        model_path: row.get(3)?,
        furniture_flags: row.get(4)?,
        markers: row.get(5)?,
        is_marker: row.get(6)?,
    })
}

impl Furniture {
    pub fn get(form_key: &str, conn: &rusqlite::Connection) -> anyhow::Result<Self> {
        Ok(conn.query_row(
            "SELECT form_key, editor_id, name, model_path,
                    furniture_flags, markers, is_marker
             FROM furniture WHERE form_key = ?",
            rusqlite::params![form_key],
            row_to_furniture,
        )?)
    }

    pub fn get_all(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(
            "SELECT form_key, editor_id, name, model_path,
                    furniture_flags, markers, is_marker
             FROM furniture",
        )?;

        Ok(stmt
            .query_map([], row_to_furniture)?
            .collect::<Result<Vec<_>, _>>()?)
    }
}
