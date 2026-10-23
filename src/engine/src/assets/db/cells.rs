use rusqlite::OptionalExtension;

pub struct Cell {
    pub form_key: Option<String>,
    pub editor_id: Option<String>,
    pub worldspace: Option<String>,
    pub grid_x: Option<i8>,
    pub grid_y: Option<i8>,
    pub is_interior: bool,
    pub flags: Option<String>,    // In essence a u16
    pub lighting: Option<String>, // This is JSON
    pub encounter_zone: Option<String>,
}

pub struct InteriorLightingRow {
    pub form_key: String,
    pub lighting: Option<String>,
    pub lighting_template: Option<String>,
}

/// `template_column_present` is false for databases converted before the
/// `cells.lighting_template` column existed; every row's template link is then
/// `None` because the link is unknown, not because the cell has no template.
pub struct InteriorLightingRows {
    pub template_column_present: bool,
    pub rows: Vec<InteriorLightingRow>,
}

impl Cell {
    /// Column list shared by every query feeding [`from_row`], whose positional
    /// `row.get(0..8)` must stay aligned with this order.
    const COLUMNS: &'static str = "form_key, editor_id, worldspace, grid_x, grid_y, is_interior, flags, lighting, encounter_zone";

    fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Self {
            form_key: row.get(0)?,
            editor_id: row.get(1)?,
            worldspace: row.get(2)?,
            grid_x: row.get(3)?,
            grid_y: row.get(4)?,
            is_interior: row.get(5)?,
            flags: row.get(6)?,
            lighting: row.get(7)?,
            encounter_zone: row.get(8)?,
        })
    }

    // Grid queries must exclude the worldspace persistent cell (WRLD.TopCell):
    // Mutagen defaults its grid coords to (0,0), which collides with the real
    // (0,0) sub-cell. Its refs load separately via PlacedObject::get_worldspace_persistent.
    pub fn get(
        x: i32,
        y: i32,
        worldspace: &str,
        conn: &rusqlite::Connection,
    ) -> anyhow::Result<Self> {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {} FROM cells WHERE worldspace = ? AND grid_x = ? AND grid_y = ?
               AND COALESCE(is_worldspace_persistent_cell, 0) = 0",
            Self::COLUMNS
        ))?;
        Ok(stmt.query_row(rusqlite::params![worldspace, x, y], Self::from_row)?)
    }

    /// Resolve a single cell by its `form_key`. The grid queries above can't reach
    /// interior cells (NULL worldspace / NULL grid); this is how the interior
    /// loader fetches one. Returns the `anyhow::Error` intact so the caller can
    /// distinguish [`rusqlite::Error::QueryReturnedNoRows`] from a real failure.
    pub fn get_by_form_key(form_key: &str, conn: &rusqlite::Connection) -> anyhow::Result<Self> {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {} FROM cells WHERE form_key = ?",
            Self::COLUMNS
        ))?;
        Ok(stmt.query_row(rusqlite::params![form_key], Self::from_row)?)
    }

    /// Resolve a cell by editor id, case-insensitively. `None` means no such
    /// cell, so callers can tell a missing id from a real query failure.
    pub fn get_by_editor_id(
        editor_id: &str,
        conn: &rusqlite::Connection,
    ) -> anyhow::Result<Option<Self>> {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {} FROM cells WHERE editor_id = ? COLLATE NOCASE",
            Self::COLUMNS
        ))?;
        Ok(stmt
            .query_row(rusqlite::params![editor_id], Self::from_row)
            .optional()?)
    }

    /// Every interior cell, for the debug interior dropdown. Ordered by editor
    /// id (NULLs first under NOCASE) — rows without one fall back to the
    /// `form_key` for their label at the call site.
    pub fn list_interiors(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM cells WHERE is_interior = 1 ORDER BY editor_id COLLATE NOCASE",
            Self::COLUMNS
        ))?;
        Ok(stmt
            .query_map([], Self::from_row)?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn list_interior_lighting(
        conn: &rusqlite::Connection,
    ) -> anyhow::Result<InteriorLightingRows> {
        let template_column_present = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('cells') WHERE name = 'lighting_template'",
            [],
            |row| row.get::<_, i64>(0),
        )? > 0;
        let template_expression = if template_column_present {
            "lighting_template"
        } else {
            "NULL"
        };
        let mut stmt = conn.prepare(&format!(
            "SELECT form_key, lighting, {template_expression} FROM cells WHERE is_interior = 1"
        ))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(InteriorLightingRow {
                    form_key: row.get(0)?,
                    lighting: row.get(1)?,
                    lighting_template: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(InteriorLightingRows {
            template_column_present,
            rows,
        })
    }

    pub fn get_for_worldspace(
        worldspace: &str,
        conn: &rusqlite::Connection,
    ) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM cells WHERE worldspace = ?
               AND COALESCE(is_worldspace_persistent_cell, 0) = 0",
            Self::COLUMNS
        ))?;
        Ok(stmt
            .query_map(rusqlite::params![worldspace], Self::from_row)?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn get_populated_cell(
        worldspace: &str,
        conn: &rusqlite::Connection,
    ) -> anyhow::Result<Self> {
        // The JOIN makes `form_key` (in both tables) ambiguous, so the
        // projection must be alias-qualified — derived from COLUMNS so the
        // column set/order can never drift from [`from_row`]'s positional reads.
        let projection = Self::COLUMNS
            .split(", ")
            .map(|c| format!("c.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        Ok(conn.query_row(
            &format!(
                "SELECT {projection}
                 FROM cells c
                 JOIN placed_objects po ON po.cell_form_key = c.form_key
                 WHERE c.worldspace = ?
                   AND c.grid_x IS NOT NULL
                   AND COALESCE(c.is_worldspace_persistent_cell, 0) = 0
                 GROUP BY c.form_key
                 ORDER BY COUNT(*) DESC
                 LIMIT 1"
            ),
            rusqlite::params![worldspace],
            Self::from_row,
        )?)
    }

    pub fn get_in_range(
        worldspace: &str,
        x_min: i32,
        x_max: i32,
        y_min: i32,
        y_max: i32,
        conn: &rusqlite::Connection,
    ) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM cells
             WHERE worldspace = ?
               AND grid_x BETWEEN ? AND ?
               AND grid_y BETWEEN ? AND ?
               AND COALESCE(is_worldspace_persistent_cell, 0) = 0",
            Self::COLUMNS
        ))?;
        Ok(stmt
            .query_map(
                rusqlite::params![worldspace, x_min, x_max, y_min, y_max],
                Self::from_row,
            )?
            .collect::<Result<Vec<_>, _>>()?)
    }
}
