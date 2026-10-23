//! `map_markers` rows: the map-marker references of a worldspace.

#[derive(Clone, Debug, PartialEq)]
pub struct MapMarkerRow {
    pub form_key: String,
    pub worldspace: String,
    pub name: Option<String>,
    pub marker_type: u16,
    pub flags: u8,
    /// Bethesda-space position (units, Z up).
    pub position: [f64; 3],
}

impl MapMarkerRow {
    pub fn list_for_worldspace(
        worldspace: &str,
        conn: &rusqlite::Connection,
    ) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(
            "SELECT form_key, worldspace, name, marker_type, flags, position_x, position_y, position_z
             FROM map_markers WHERE worldspace = ? ORDER BY form_key",
        )?;
        Ok(stmt
            .query_map(rusqlite::params![worldspace], |row| {
                Ok(Self {
                    form_key: row.get(0)?,
                    worldspace: row.get(1)?,
                    name: row.get(2)?,
                    marker_type: row.get(3)?,
                    flags: row.get(4)?,
                    position: [row.get(5)?, row.get(6)?, row.get(7)?],
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }
}
