//! Load-door teleport destinations (REFR XTEL). One row per source door REFR,
//! already resolved to its destination cell. Read once at boot to build the
//! in-memory activation map (see rendering's teleport map); never touched on
//! the hot streaming path.

use rusqlite::{Connection, OptionalExtension};

use super::doors::{DoorFlagsError, flags_are_automatic};
use super::schema::{DbSchema, cell_name_select_list, teleport_flags_select_list};

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
    /// The XTEL flags as stored; 0 for a database without the column.
    pub flags: u32,
    /// The destination cell's display name; NULL when it has none, and for a database without
    /// the column.
    pub dest_cell_name: Option<String>,
    /// The destination worldspace's display name.
    pub dest_worldspace_name: Option<String>,
    /// The source door's base record flags, a decimal number as text; NULL when the source ref
    /// or its base door is not in the database.
    pub source_door_flags: Option<String>,
}

const FLAG_NO_LOAD_SCREEN: u32 = 1 << 1;

fn select_resolved(schema: DbSchema) -> String {
    format!(
        "SELECT t.source_ref,
            t.dest_position_x, t.dest_position_y, t.dest_position_z,
            t.dest_rotation_z,
            c.is_interior, c.worldspace, c.form_key,
            {flags},
            {cell_name}, w.name, d.flags
     FROM placed_teleports t
     JOIN placed_objects po ON po.form_key = t.destination_ref
     JOIN cells c ON c.form_key = po.cell_form_key
     LEFT JOIN worldspaces w ON w.form_key = c.worldspace
     LEFT JOIN placed_objects source_po ON source_po.form_key = t.source_ref
     LEFT JOIN doors d ON d.form_key = source_po.base_object",
        flags = teleport_flags_select_list(schema, "t."),
        cell_name = cell_name_select_list(schema, "c."),
    )
}

impl TeleportRecord {
    /// Whether the door goes through without a loading screen.
    pub fn no_load_screen(&self) -> bool {
        self.flags & FLAG_NO_LOAD_SCREEN != 0
    }

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
            flags: row.get::<_, Option<u32>>(8)?.unwrap_or(0),
            dest_cell_name: row.get(9)?,
            dest_worldspace_name: row.get(10)?,
            source_door_flags: row.get(11)?,
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
pub fn get_all(conn: &Connection, schema: DbSchema) -> anyhow::Result<Vec<TeleportRecord>> {
    let mut stmt = conn.prepare(&select_resolved(schema))?;
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
    schema: DbSchema,
) -> anyhow::Result<Option<TeleportRecord>> {
    let mut stmt = conn.prepare(&format!(
        "{} WHERE c.form_key = ? ORDER BY t.source_ref LIMIT 1",
        select_resolved(schema)
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

/// Where a walk-in door stands: an interior cell, or a worldspace (exterior
/// doors are position-based, many live in the worldspace's persistent cell).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DoorLocation {
    Interior { cell: String },
    Exterior { worldspace: String },
}

/// A placed automatic load door. Position and rotation are Bethesda units and
/// radians, exactly as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct WalkInDoor {
    pub source_ref: String,
    pub location: DoorLocation,
    pub position: [f64; 3],
    pub rotation: [f64; 3],
    pub scale: f64,
}

/// The walk-in doors, plus the load doors whose row is unusable; each of those
/// is left out on its own and the caller reports them once.
#[derive(Debug, Default)]
pub struct WalkInDoors {
    pub doors: Vec<WalkInDoor>,
    pub rejected: Vec<WalkInDoorError>,
}

#[derive(Debug, thiserror::Error)]
pub enum WalkInDoorError {
    #[error(transparent)]
    Flags(#[from] DoorFlagsError),
    #[error("door ref {source_ref} stands in a non-interior cell without a worldspace")]
    ExteriorCellWithoutWorldspace { source_ref: String },
}

struct WalkInRow {
    source_ref: String,
    flags: Option<String>,
    is_interior: bool,
    cell: String,
    worldspace: Option<String>,
    position: [f64; 3],
    rotation: [f64; 3],
    scale: f64,
}

/// Automatic load doors that are enabled at game start. Refs that start
/// disabled or hidden stay out: nothing in the engine enables them later.
pub fn get_walk_in_doors(conn: &Connection) -> anyhow::Result<WalkInDoors> {
    let mut stmt = conn.prepare(
        "SELECT t.source_ref, d.flags, c.is_interior, c.form_key, c.worldspace,
                po.position_x, po.position_y, po.position_z,
                po.rotation_x, po.rotation_y, po.rotation_z, po.scale
         FROM placed_teleports t
         JOIN placed_objects po ON po.form_key = t.source_ref
         JOIN doors d ON d.form_key = po.base_object
         JOIN cells c ON c.form_key = po.cell_form_key
         WHERE po.is_initially_disabled = 0 AND po.initially_hidden = 0
         ORDER BY t.source_ref",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(WalkInRow {
                source_ref: row.get(0)?,
                flags: row.get(1)?,
                is_interior: row.get(2)?,
                cell: row.get(3)?,
                worldspace: row.get(4)?,
                position: [row.get(5)?, row.get(6)?, row.get(7)?],
                rotation: [row.get(8)?, row.get(9)?, row.get(10)?],
                scale: row.get(11)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut result = WalkInDoors::default();
    for row in rows {
        match walk_in_door(row) {
            Ok(Some(door)) => result.doors.push(door),
            Ok(None) => {}
            Err(error) => result.rejected.push(error),
        }
    }
    Ok(result)
}

/// The door of an automatic row; `None` for any other door.
fn walk_in_door(row: WalkInRow) -> Result<Option<WalkInDoor>, WalkInDoorError> {
    if !flags_are_automatic(
        &format!("door ref {}", row.source_ref),
        row.flags.as_deref(),
    )? {
        return Ok(None);
    }
    let location = door_location(&row)?;
    Ok(Some(WalkInDoor {
        source_ref: row.source_ref,
        location,
        position: row.position,
        rotation: row.rotation,
        scale: row.scale,
    }))
}

fn door_location(row: &WalkInRow) -> Result<DoorLocation, WalkInDoorError> {
    if row.is_interior {
        return Ok(DoorLocation::Interior {
            cell: row.cell.clone(),
        });
    }
    row.worldspace
        .clone()
        .map(|worldspace| DoorLocation::Exterior { worldspace })
        .ok_or_else(|| WalkInDoorError::ExteriorCellWithoutWorldspace {
            source_ref: row.source_ref.clone(),
        })
}
