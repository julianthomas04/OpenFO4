//! Load-door teleport destinations (REFR XTEL). One row per source door REFR,
//! already resolved to its destination cell. Read once at boot to build the
//! in-memory activation map (see rendering's teleport map); never touched on
//! the hot streaming path.

use rusqlite::{Connection, OptionalExtension};

/// A source load door and the cell/position/rotation it teleports to. Positions
/// are Bethesda world units and `dest_rot_z` is radians, exactly as stored —
/// the renderer converts to engine space.
pub struct TeleportRecord {
    pub source_ref: String,
    pub dest_x: f64,
    pub dest_y: f64,
    pub dest_z: f64,
    pub dest_rot_z: f64,
    pub is_interior: bool,
    /// The destination cell's worldspace `form_key`; NULL for interior cells.
    pub worldspace: Option<String>,
    pub dest_cell: String,
}

const SELECT_RESOLVED: &str = "SELECT t.source_ref,
            t.dest_position_x, t.dest_position_y, t.dest_position_z,
            t.dest_rotation_z,
            c.is_interior, c.worldspace, c.form_key
     FROM placed_teleports t
     JOIN placed_objects po ON po.form_key = t.destination_ref
     JOIN cells c ON c.form_key = po.cell_form_key";

impl TeleportRecord {
    fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Self {
            source_ref: row.get(0)?,
            dest_x: row.get(1)?,
            dest_y: row.get(2)?,
            dest_z: row.get(3)?,
            dest_rot_z: row.get(4)?,
            is_interior: row.get(5)?,
            worldspace: row.get(6)?,
            dest_cell: row.get(7)?,
        })
    }
}

/// Every load door whose destination ref resolves to a real cell, in one
/// batched query (no per-row lookups). Doors whose destination ref is absent
/// from `placed_objects` are dropped by the inner JOIN, so they never surface
/// as interactible — the caller logs that gap against the raw teleport count.
///
/// Unlike the cell-streaming queries, this deliberately does NOT exclude the
/// worldspace-persistent cell: a teleport destination is often a persistent ref
/// living in that `TopCell`, and we still want its `is_interior` (0) and
/// `worldspace` for classification. The `TopCell`'s spoofed grid is irrelevant
/// here — the caller derives the arrival grid from the destination position.
pub fn get_all(conn: &Connection) -> anyhow::Result<Vec<TeleportRecord>> {
    let mut stmt = conn.prepare(SELECT_RESOLVED)?;
    let rows = stmt
        .query_map([], TeleportRecord::from_row)?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// The load door with the lowest source ref that arrives in `cell_form_key`,
/// for a deterministic pick when several doors lead into the same cell.
pub fn get_into_cell(
    cell_form_key: &str,
    conn: &Connection,
) -> anyhow::Result<Option<TeleportRecord>> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT_RESOLVED} WHERE c.form_key = ? ORDER BY t.source_ref LIMIT 1"
    ))?;
    Ok(stmt
        .query_row(rusqlite::params![cell_form_key], TeleportRecord::from_row)
        .optional()?)
}

/// Total rows in `placed_teleports`, including those whose destination cannot be
/// resolved — so the caller can report how many doors were dropped by `get_all`.
pub fn count(conn: &Connection) -> anyhow::Result<usize> {
    let n: i64 = conn.query_row("SELECT count(*) FROM placed_teleports", [], |r| r.get(0))?;
    Ok(n as usize)
}
