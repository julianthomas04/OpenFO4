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

    /// The diffuse texture of the texture set `form_key`; `None` when the set is absent or names
    /// no diffuse texture.
    pub fn texture_set_diffuse(
        conn: &rusqlite::Connection,
        form_key: &str,
    ) -> anyhow::Result<Option<String>> {
        let mut stmt = conn.prepare("SELECT diffuse FROM texture_sets WHERE form_key = ?1")?;
        let mut rows = stmt.query([form_key])?;
        Ok(match rows.next()? {
            Some(row) => row.get(0)?,
            None => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;

    fn database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE texture_sets (form_key TEXT PRIMARY KEY, diffuse TEXT);
             INSERT INTO texture_sets VALUES ('0A:Fallout4.esm', 'landscape/ground/a_d.dds');
             INSERT INTO texture_sets VALUES ('0B:Fallout4.esm', NULL);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn a_texture_set_gives_its_diffuse_texture() {
        let diffuse = LandscapeTextureDiffuse::texture_set_diffuse(&database(), "0A:Fallout4.esm");
        assert_eq!(
            diffuse.unwrap().as_deref(),
            Some("landscape/ground/a_d.dds")
        );
    }

    #[test]
    fn a_texture_set_without_a_diffuse_texture_or_record_gives_none() {
        let conn = database();
        for form_key in ["0B:Fallout4.esm", "0C:Fallout4.esm"] {
            let diffuse = LandscapeTextureDiffuse::texture_set_diffuse(&conn, form_key);
            assert_eq!(diffuse.unwrap(), None, "{form_key}");
        }
    }
}
