//! Load-door teleport destinations (REFR XTEL). One row per source door REFR,
//! already resolved to its destination cell. Read once at boot to build the
//! in-memory activation map (see rendering's teleport map); never touched on
//! the hot streaming path.

use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;

use super::doors::{DoorFlagsError, flags_are_automatic};
use super::schema::{DbSchema, door_trigger_bounds_select_list};

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

/// Where a walk-in door stands: an interior cell, or a worldspace (exterior
/// doors are position-based, many live in the worldspace's persistent cell).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DoorLocation {
    Interior { cell: String },
    Exterior { worldspace: String },
}

/// A door's trigger volume (`doors.trigger_bounds`), door-local, Bethesda units, Z-up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DoorBounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

#[derive(Deserialize)]
struct JsonPoint {
    #[serde(rename = "X")]
    x: f64,
    #[serde(rename = "Y")]
    y: f64,
    #[serde(rename = "Z")]
    z: f64,
}

#[derive(Deserialize)]
struct JsonBounds {
    #[serde(rename = "Min")]
    min: JsonPoint,
    #[serde(rename = "Max")]
    max: JsonPoint,
}

impl DoorBounds {
    fn parse(json: &str) -> Result<Self, serde_json::Error> {
        let JsonBounds { min, max } = serde_json::from_str(json)?;
        Ok(Self {
            min: [min.x, min.y, min.z],
            max: [max.x, max.y, max.z],
        })
    }
}

/// A placed automatic load door and its trigger volume. Position and rotation
/// are Bethesda units and radians, exactly as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct WalkInDoor {
    pub source_ref: String,
    pub location: DoorLocation,
    pub position: [f64; 3],
    pub rotation: [f64; 3],
    pub scale: f64,
    pub bounds: DoorBounds,
}

/// Walk-in doors with a usable trigger volume, plus the refs that could not
/// get one because the database carries no bounds for them (converted before
/// the installer extracted door trigger bounds); the caller reports those once.
#[derive(Debug, Default)]
pub struct WalkInDoors {
    pub doors: Vec<WalkInDoor>,
    pub without_bounds: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum WalkInDoorError {
    #[error(transparent)]
    Flags(#[from] DoorFlagsError),
    #[error("door ref {source_ref}: bounds are malformed: {reason}")]
    MalformedBounds { source_ref: String, reason: String },
    #[error("door ref {source_ref} stands in a non-interior cell without a worldspace")]
    ExteriorCellWithoutWorldspace { source_ref: String },
}

struct WalkInRow {
    source_ref: String,
    flags: Option<String>,
    bounds: Option<String>,
    is_interior: bool,
    cell: String,
    worldspace: Option<String>,
    position: [f64; 3],
    rotation: [f64; 3],
    scale: f64,
}

/// Automatic load doors that are enabled at game start. Refs that start
/// disabled or hidden stay out: nothing in the engine enables them later.
pub fn get_walk_in_doors(conn: &Connection, schema: DbSchema) -> anyhow::Result<WalkInDoors> {
    let mut stmt = conn.prepare(&format!(
        "SELECT t.source_ref, d.flags, {bounds}, c.is_interior, c.form_key, c.worldspace,
                po.position_x, po.position_y, po.position_z,
                po.rotation_x, po.rotation_y, po.rotation_z, po.scale
         FROM placed_teleports t
         JOIN placed_objects po ON po.form_key = t.source_ref
         JOIN doors d ON d.form_key = po.base_object
         JOIN cells c ON c.form_key = po.cell_form_key
         WHERE po.is_initially_disabled = 0 AND po.initially_hidden = 0
         ORDER BY t.source_ref",
        bounds = door_trigger_bounds_select_list(schema, "d."),
    ))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(WalkInRow {
                source_ref: row.get(0)?,
                flags: row.get(1)?,
                bounds: row.get(2)?,
                is_interior: row.get(3)?,
                cell: row.get(4)?,
                worldspace: row.get(5)?,
                position: [row.get(6)?, row.get(7)?, row.get(8)?],
                rotation: [row.get(9)?, row.get(10)?, row.get(11)?],
                scale: row.get(12)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut result = WalkInDoors::default();
    for row in rows {
        if !flags_are_automatic(
            &format!("door ref {}", row.source_ref),
            row.flags.as_deref(),
        )? {
            continue;
        }
        let Some(bounds_json) = row.bounds.as_deref() else {
            result.without_bounds.push(row.source_ref);
            continue;
        };
        let bounds =
            DoorBounds::parse(bounds_json).map_err(|e| WalkInDoorError::MalformedBounds {
                source_ref: row.source_ref.clone(),
                reason: e.to_string(),
            })?;
        let location = door_location(&row)?;
        result.doors.push(WalkInDoor {
            source_ref: row.source_ref,
            location,
            position: row.position,
            rotation: row.rotation,
            scale: row.scale,
            bounds,
        });
    }
    Ok(result)
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
