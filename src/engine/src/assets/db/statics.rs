pub struct Static {
    pub form_key: Option<String>,
    pub editor_id: Option<String>,
    pub model_path: Option<String>,
    pub material_object: Option<String>,
    pub has_distant_lod: bool,
    pub bounds: Option<String>,
    pub alternate_textures: Option<String>,
    pub is_marker: bool,
    pub lod_model_path: Option<String>,
}

fn row_to_static(row: &rusqlite::Row) -> rusqlite::Result<Static> {
    Ok(Static {
        form_key: row.get(0)?,
        editor_id: row.get(1)?,
        model_path: row.get(2)?,
        material_object: row.get(3)?,
        has_distant_lod: row.get(4)?,
        bounds: row.get(5)?,
        alternate_textures: row.get(6)?,
        is_marker: row.get(7)?,
        lod_model_path: row.get(8)?,
    })
}

impl Static {
    pub fn get(form_key: &str, conn: &rusqlite::Connection) -> anyhow::Result<Self> {
        Ok(conn.query_row(
            "SELECT form_key, editor_id, model_path, material_object,
                    has_distant_lod, bounds, alternate_textures, is_marker,
                    lod_model_path
             FROM statics WHERE form_key = ?",
            rusqlite::params![form_key],
            row_to_static,
        )?)
    }

    pub fn get_all(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(
            "SELECT form_key, editor_id, model_path, material_object,
                    has_distant_lod, bounds, alternate_textures, is_marker,
                    lod_model_path
             FROM statics",
        )?;

        Ok(stmt
            .query_map([], row_to_static)?
            .collect::<Result<Vec<_>, _>>()?)
    }
}
