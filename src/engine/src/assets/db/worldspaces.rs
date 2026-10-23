pub struct Worldspace {
    pub form_key: Option<String>,
    pub editor_id: Option<String>,
    pub name: Option<String>,
    pub parent_worldspace: Option<String>, // No idea what this is for. No populated field
    pub flags: Option<String>, // No idea what that is. Also it stores JUST INTEGERS BUT IS A String (u8 it looks like)
    pub world_map_data: Option<String>, // No idea what this is, all fields contain empty JSON
}

impl Worldspace {
    /// Column list shared by every query feeding [`from_row`], whose positional
    /// `row.get(0..6)` must stay aligned with this order.
    const COLUMNS: &'static str =
        "form_key, editor_id, name, parent_worldspace, flags, world_map_data";

    pub fn get(form_key: &str, conn: &rusqlite::Connection) -> anyhow::Result<Self> {
        Ok(conn.query_row(
            &format!(
                "SELECT {} FROM worldspaces WHERE form_key = ?",
                Self::COLUMNS
            ),
            rusqlite::params![form_key],
            Self::from_row,
        )?)
    }

    /// Every WRLD record, for the debug worldspace dropdown.
    pub fn list_all(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM worldspaces ORDER BY editor_id COLLATE NOCASE",
            Self::COLUMNS
        ))?;
        Ok(stmt
            .query_map([], Self::from_row)?
            .collect::<Result<Vec<_>, _>>()?)
    }

    /// The worldspace's map image path (`ICON`, relative to `textures`), e.g.
    /// `Interface\Pip-Boy\WorldMap_d.dds`; `None` when it has no map.
    pub fn map_image(
        form_key: &str,
        conn: &rusqlite::Connection,
    ) -> anyhow::Result<Option<String>> {
        let image = conn.query_row(
            "SELECT map_image FROM worldspaces WHERE form_key = ?",
            rusqlite::params![form_key],
            |row| row.get::<_, Option<String>>(0),
        );
        match image {
            Ok(image) => Ok(image),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Self {
            form_key: row.get(0)?,
            editor_id: row.get(1)?,
            name: row.get(2)?,
            parent_worldspace: row.get(3)?,
            flags: row.get(4)?,
            world_map_data: row.get(5)?,
        })
    }
}
