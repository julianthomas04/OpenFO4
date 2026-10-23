pub struct LightingTemplate {
    pub form_key: String,
    pub editor_id: Option<String>,
    /// JSON; NULL until the database is re-converted with lighting template data.
    pub data: Option<String>,
}

impl LightingTemplate {
    pub fn get_all(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare("SELECT form_key, editor_id, data FROM lighting_templates")?;

        Ok(stmt
            .query_map([], |row| {
                Ok(Self {
                    form_key: row.get(0)?,
                    editor_id: row.get(1)?,
                    data: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }
}
