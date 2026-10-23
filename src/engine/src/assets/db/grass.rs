//! `GRAS` records and the per-landscape-texture grass lists (`LTEX` `GNAM`).

use anyhow::Context;

/// One `GRAS` record. Every numeric `DATA` column must be present and within
/// its type's range; a NULL or out-of-range value is an error, not a default.
#[derive(Debug, Clone, PartialEq)]
pub struct GrassRow {
    pub form_key: String,
    pub editor_id: Option<String>,
    pub model_path: Option<String>,
    pub flags: u8,
    pub density: u8,
    pub min_slope: u8,
    pub max_slope: u8,
    pub units_from_water: u16,
    pub units_from_water_type: u32,
    pub position_range: f32,
    pub height_range: f32,
    pub color_range: f32,
    pub wave_period: f32,
}

/// One entry of an `LTEX` grass list; `ord` is the `GNAM` order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LtexGrassRow {
    pub ltex_form_key: String,
    pub grass_form_key: String,
    pub ord: u32,
}

/// A REAL column; NULL fails with `InvalidColumnType`.
fn real(row: &rusqlite::Row, index: usize) -> rusqlite::Result<f32> {
    Ok(row.get::<_, f64>(index)? as f32)
}

impl GrassRow {
    pub fn get_all(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(
            "SELECT form_key, editor_id, model_path, flags, density, min_slope, max_slope,
                    units_from_water, units_from_water_type, position_range, height_range,
                    color_range, wave_period
             FROM grasses ORDER BY form_key",
        )?;
        stmt.query_map([], |row| {
            Ok(Self {
                form_key: row.get(0)?,
                editor_id: row.get(1)?,
                model_path: row.get(2)?,
                flags: row.get(3)?,
                density: row.get(4)?,
                min_slope: row.get(5)?,
                max_slope: row.get(6)?,
                units_from_water: row.get(7)?,
                units_from_water_type: row.get(8)?,
                position_range: real(row, 9)?,
                height_range: real(row, 10)?,
                color_range: real(row, 11)?,
                wave_period: real(row, 12)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .context("reading the grasses table")
    }
}

impl LtexGrassRow {
    /// Every list entry, grouped by texture and in `GNAM` order within one.
    pub fn get_all(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(
            "SELECT ltex_form_key, grass_form_key, ord FROM ltex_grasses
             ORDER BY ltex_form_key, ord",
        )?;
        stmt.query_map([], |row| {
            Ok(Self {
                ltex_form_key: row.get(0)?,
                grass_form_key: row.get(1)?,
                ord: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .context("reading the ltex_grasses table")
    }
}
