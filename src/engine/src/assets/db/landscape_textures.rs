pub struct LandscapeTexture {
    pub form_key: String,
    pub editor_id: String,
    pub texture_set: Option<String>,
    pub material_type: Option<String>,
    pub friction: u8,
    pub restitution: u8,
    pub havok_data: Option<String>,
}

pub struct LandscapeTextureDiffuse {
    pub form_key: String,
    pub editor_id: Option<String>,
    pub texture_set_diffuse: Option<String>,
}

impl LandscapeTextureDiffuse {
    pub fn get_all(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(
            "SELECT lt.form_key, lt.editor_id, ts.diffuse
             FROM landscape_textures lt
             LEFT JOIN texture_sets ts ON lt.texture_set = ts.form_key",
        )?;

        Ok(stmt
            .query_map([], |row| {
                Ok(Self {
                    form_key: row.get(0)?,
                    editor_id: row.get(1)?,
                    texture_set_diffuse: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }
}
