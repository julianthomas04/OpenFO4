//! Where water is placed: the cell and worldspace water columns and the placed
//! refs of water activators. The exact rules turning these into water planes
//! live with the renderer; this module only reads the records.

use std::collections::HashMap;

use crate::assets::db::schema::{
    DbSchema, activator_water_select_list, cell_water_select_list, worldspace_water_select_list,
};

/// `XCLW` stores float max when the cell sets no water height.
const NO_WATER_HEIGHT: f64 = f32::MAX as f64;

/// A cell's `XCLW` water height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CellWaterHeight {
    /// No `XCLW` subrecord, or a database predating water extraction.
    Absent,
    /// `XCLW` holds float max: the cell sets no height of its own, so exterior
    /// cells inherit the worldspace's default water height.
    Unset,
    Level(f32),
}

impl CellWaterHeight {
    fn from_column(value: Option<f64>) -> Self {
        match value {
            None => Self::Absent,
            Some(height) if height >= NO_WATER_HEIGHT => Self::Unset,
            Some(height) => Self::Level(height as f32),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CellWater {
    /// The CK "Has Water" flag: the real water signal for interiors.
    pub has_water: bool,
    pub no_lod_water: bool,
    pub height: CellWaterHeight,
    /// XCWT: WATR form key overriding the worldspace water.
    pub water_type: Option<String>,
    pub environment_map: Option<String>,
    /// `XWCN` count and `XWCU` entries: the raw 16-byte words of the cell's
    /// water current volume elements (not interpreted yet).
    pub currents: Option<Vec<[f32; 4]>>,
}

/// The columns of [`cell_water_select_list`], read from column `offset` on.
struct CellWaterColumns {
    has_water: bool,
    no_lod_water: bool,
    height: CellWaterHeight,
    water_type: Option<String>,
    environment_map: Option<String>,
    currents_json: Option<String>,
}

impl CellWaterColumns {
    fn read(row: &rusqlite::Row, offset: usize) -> rusqlite::Result<Self> {
        Ok(Self {
            has_water: row.get::<_, Option<bool>>(offset)?.unwrap_or(false),
            no_lod_water: row.get::<_, Option<bool>>(offset + 1)?.unwrap_or(false),
            height: CellWaterHeight::from_column(row.get::<_, Option<f64>>(offset + 2)?),
            water_type: row.get(offset + 3)?,
            environment_map: row.get(offset + 4)?,
            currents_json: row.get(offset + 5)?,
        })
    }

    fn into_cell_water(self, currents: Option<Vec<[f32; 4]>>) -> CellWater {
        CellWater {
            has_water: self.has_water,
            no_lod_water: self.no_lod_water,
            height: self.height,
            water_type: self.water_type,
            environment_map: self.environment_map,
            currents,
        }
    }
}

impl CellWater {
    pub fn get(
        cell_form_key: &str,
        conn: &rusqlite::Connection,
        schema: DbSchema,
    ) -> anyhow::Result<Self> {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {} FROM cells WHERE form_key = ?",
            cell_water_select_list(schema)
        ))?;
        let mut columns = stmt.query_row(rusqlite::params![cell_form_key], |row| {
            CellWaterColumns::read(row, 0)
        })?;
        let currents = columns
            .currents_json
            .take()
            .map(|json| {
                serde_json::from_str(&json)
                    .map_err(|e| anyhow::anyhow!("cell {cell_form_key} water currents: {e}"))
            })
            .transpose()?;
        Ok(columns.into_cell_water(currents))
    }

    /// The water fields of every gridded exterior cell of a worldspace that
    /// has water, keyed by grid coordinate. The flow block is not decoded: the
    /// bulk read serves height and type lookups.
    pub fn exterior_with_water(
        worldspace_form_key: &str,
        conn: &rusqlite::Connection,
        schema: DbSchema,
    ) -> anyhow::Result<HashMap<(i32, i32), Self>> {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT grid_x, grid_y, {} FROM cells
             WHERE worldspace = ? AND is_interior = 0 AND has_water = 1
               AND grid_x IS NOT NULL AND grid_y IS NOT NULL",
            cell_water_select_list(schema)
        ))?;
        let rows = stmt.query_map(rusqlite::params![worldspace_form_key], |row| {
            let grid = (row.get::<_, i32>(0)?, row.get::<_, i32>(1)?);
            Ok((grid, CellWaterColumns::read(row, 2)?.into_cell_water(None)))
        })?;
        Ok(rows.collect::<Result<HashMap<_, _>, _>>()?)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorldspaceWater {
    /// NAM2: the worldspace's default WATR.
    pub water_type: Option<String>,
    /// NAM3: WATR of the distant LOD water.
    pub lod_water_type: Option<String>,
    pub lod_water_height: Option<f32>,
    pub default_land_height: Option<f32>,
    pub default_water_height: Option<f32>,
    pub environment_map: Option<String>,
    /// `PNAM`: which data the worldspace takes from its parent.
    pub parent_flags: Option<u16>,
    pub parent_worldspace: Option<String>,
    /// `DATA` worldspace flags (bit `0x08`: no LOD water).
    pub flags: Option<u32>,
}

impl WorldspaceWater {
    pub fn get(
        worldspace_form_key: &str,
        conn: &rusqlite::Connection,
        schema: DbSchema,
    ) -> anyhow::Result<Self> {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {}, parent_worldspace, flags FROM worldspaces WHERE form_key = ?",
            worldspace_water_select_list(schema)
        ))?;
        let (water, flags_text) =
            stmt.query_row(rusqlite::params![worldspace_form_key], |row| {
                let height = |index| {
                    row.get::<_, Option<f64>>(index)
                        .map(|v| v.map(|v| v as f32))
                };
                let water = Self {
                    water_type: row.get(0)?,
                    lod_water_type: row.get(1)?,
                    lod_water_height: height(2)?,
                    default_land_height: height(3)?,
                    default_water_height: height(4)?,
                    environment_map: row.get(5)?,
                    parent_flags: row.get(6)?,
                    parent_worldspace: row.get(7)?,
                    flags: None,
                };
                Ok((water, row.get::<_, Option<String>>(8)?))
            })?;
        // The installer stores the integer flags as text.
        let flags = flags_text
            .map(|text| {
                text.trim().parse::<u32>().map_err(|e| {
                    anyhow::anyhow!("worldspace {worldspace_form_key} flags {text:?}: {e}")
                })
            })
            .transpose()?;
        Ok(Self { flags, ..water })
    }
}

/// A placed ref whose base is a water activator, with the WATR it draws.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedWaterRef {
    pub form_key: Option<String>,
    pub base_object: String,
    pub water_type: String,
    pub position: [f64; 3],
    pub rotation: [f64; 3],
    pub scale: f64,
}

impl PlacedWaterRef {
    pub fn get_for_cell(
        cell_form_key: &str,
        conn: &rusqlite::Connection,
        schema: DbSchema,
    ) -> anyhow::Result<Vec<Self>> {
        let water_type = activator_water_select_list(schema, "a.");
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT po.form_key, po.base_object, {water_type},
                    po.position_x, po.position_y, po.position_z,
                    po.rotation_x, po.rotation_y, po.rotation_z, po.scale
             FROM placed_objects po
             JOIN activators a ON a.form_key = po.base_object
             WHERE po.cell_form_key = ? AND a.water_type IS NOT NULL
               AND po.initially_hidden = 0"
        ))?;
        Ok(stmt
            .query_map(rusqlite::params![cell_form_key], |row| {
                Ok(Self {
                    form_key: row.get(0)?,
                    base_object: row.get(1)?,
                    water_type: row.get(2)?,
                    position: [row.get(3)?, row.get(4)?, row.get(5)?],
                    rotation: [row.get(6)?, row.get(7)?, row.get(8)?],
                    scale: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }

    /// Cells holding at least one visible water activator ref, ranked by how
    /// many, for choosing where to look at placed water.
    pub fn cells_with_placed_water(
        limit: usize,
        conn: &rusqlite::Connection,
    ) -> anyhow::Result<Vec<(String, usize)>> {
        let mut stmt = conn.prepare_cached(
            "SELECT po.cell_form_key, COUNT(*) AS n
             FROM placed_objects po
             JOIN activators a ON a.form_key = po.base_object
             WHERE a.water_type IS NOT NULL AND po.initially_hidden = 0
             GROUP BY po.cell_form_key
             ORDER BY n DESC, po.cell_form_key
             LIMIT ?",
        )?;
        Ok(stmt
            .query_map(rusqlite::params![limit as i64], |row| {
                Ok((row.get(0)?, row.get::<_, i64>(1)? as usize))
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }
}

/// One 4096 x 4096 unit distant-water quad of the level-4 terrain LOD: a cell
/// of the grid, flat at a baked height (game units).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LodWaterQuad {
    pub cell_x: i32,
    pub cell_y: i32,
    pub height: f32,
}

impl LodWaterQuad {
    /// Every quad of a worldspace. A database converted before the installer
    /// extracted the terrain LOD water has no such table; that is an error
    /// naming it, not an empty list.
    pub fn for_worldspace(
        worldspace_form_key: &str,
        conn: &rusqlite::Connection,
    ) -> anyhow::Result<Vec<Self>> {
        let table_present: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'lod_water_quads')",
            [],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            table_present,
            "lod_water_quads table missing (database predates LOD water extraction)"
        );
        let mut stmt = conn.prepare_cached(
            "SELECT cell_x, cell_y, height FROM lod_water_quads WHERE worldspace = ?
             ORDER BY cell_x, cell_y",
        )?;
        Ok(stmt
            .query_map(rusqlite::params![worldspace_form_key], |row| {
                Ok(Self {
                    cell_x: row.get(0)?,
                    cell_y: row.get(1)?,
                    height: row.get::<_, f64>(2)? as f32,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }
}
