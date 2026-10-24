use rusqlite::OptionalExtension;

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
    /// Column list shared by every query feeding [`from_row`], whose positional
    /// `row.get(0..6)` must stay aligned with this order.
    const COLUMNS: &'static str =
        "form_key, editor_id, cell, vertex_height_map, vertex_colors, layers, flags";

    fn from_row(row: &rusqlite::Row, grid_x: i32, grid_y: i32) -> rusqlite::Result<Self> {
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
    }

    pub fn get(
        cell: &str,
        grid_x: i32,
        grid_y: i32,
        conn: &rusqlite::Connection,
    ) -> anyhow::Result<Self> {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {} FROM landscapes WHERE cell = ?",
            Self::COLUMNS
        ))?;
        Ok(stmt.query_row(rusqlite::params![cell], |row| {
            Self::from_row(row, grid_x, grid_y)
        })?)
    }

    /// The LAND of the exterior grid cell of a worldspace itself, `None` when the
    /// cell has none. The worldspace persistent cell is no grid cell.
    pub fn get_in_worldspace(
        worldspace: &str,
        grid_x: i32,
        grid_y: i32,
        conn: &rusqlite::Connection,
    ) -> anyhow::Result<Option<Self>> {
        let mut stmt = conn.prepare_cached(
            "SELECT l.form_key, l.editor_id, l.cell, l.vertex_height_map, l.vertex_colors,
                    l.layers, l.flags
             FROM landscapes l JOIN cells c ON c.form_key = l.cell
             WHERE c.worldspace = ? AND c.grid_x = ? AND c.grid_y = ?
               AND COALESCE(c.is_worldspace_persistent_cell, 0) = 0",
        )?;
        Ok(stmt
            .query_row(rusqlite::params![worldspace, grid_x, grid_y], |row| {
                Self::from_row(row, grid_x, grid_y)
            })
            .optional()?)
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
            "SELECT {} FROM landscapes WHERE cell IN ({placeholders})",
            Self::COLUMNS
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
                Self::from_row(row, gx, gy)
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }
}
