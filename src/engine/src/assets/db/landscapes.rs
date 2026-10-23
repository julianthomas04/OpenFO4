use crate::assets::db::cells::Cell;

pub struct Landscape {
    pub form_key: Option<String>,
    pub editor_id: Option<String>,
    pub cell: String,
    pub grid_x: i32,
    pub grid_y: i32,
    pub vertex_height_map: Vec<u8>,
    pub vertex_colors: Option<Vec<u8>>,
    pub layers: Option<Vec<u8>>,
    pub flags: Option<String>,
}

impl Landscape {
    pub fn get(
        cell: &str,
        grid_x: i32,
        grid_y: i32,
        conn: &rusqlite::Connection,
    ) -> anyhow::Result<Self> {
        let mut stmt = conn.prepare_cached(
            "SELECT form_key, editor_id, cell, vertex_height_map, vertex_colors, layers, flags
             FROM landscapes WHERE cell = ?",
        )?;
        Ok(stmt.query_row(rusqlite::params![cell], |row| {
            Ok(Self {
                form_key: row.get(0)?,
                editor_id: row.get(1)?,
                cell: row.get(2)?,
                grid_x,
                grid_y,
                vertex_height_map: row.get(3)?,
                vertex_colors: row.get(4)?,
                layers: row.get(5)?,
                flags: row.get(6)?,
            })
        })?)
    }

    pub fn get_for_cells(cells: &[Cell], conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let keys: Vec<(&str, i32, i32)> = cells
            .iter()
            .filter_map(|c| {
                Some((
                    c.form_key.as_deref()?,
                    i32::from(c.grid_x?),
                    i32::from(c.grid_y?),
                ))
            })
            .collect();

        if keys.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: String = keys.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT form_key, editor_id, cell, vertex_height_map, vertex_colors, layers, flags
             FROM landscapes WHERE cell IN ({placeholders})"
        );

        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = keys
            .iter()
            .map(|k| &k.0 as &dyn rusqlite::types::ToSql)
            .collect();

        let grid_lookup: std::collections::HashMap<&str, (i32, i32)> =
            keys.iter().map(|(fk, gx, gy)| (*fk, (*gx, *gy))).collect();

        Ok(stmt
            .query_map(params.as_slice(), |row| {
                let cell_fk: String = row.get(2)?;
                let (gx, gy) = grid_lookup.get(cell_fk.as_str()).copied().unwrap_or((0, 0));
                Ok(Self {
                    form_key: row.get(0)?,
                    editor_id: row.get(1)?,
                    cell: cell_fk,
                    grid_x: gx,
                    grid_y: gy,
                    vertex_height_map: row.get(3)?,
                    vertex_colors: row.get(4)?,
                    layers: row.get(5)?,
                    flags: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }
}
