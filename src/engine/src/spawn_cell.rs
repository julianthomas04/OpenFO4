use std::ffi::OsString;

use crate::assets::db::DbClient;

pub(crate) const SPAWN_CELL_VAR: &str = "OPENFO4_SPAWN_CELL";
pub(crate) const SPAWN_VAR: &str = "OPENFO4_SPAWN";

/// Where to put the player inside an interior cell, in Bethesda world units
/// (the `placed_objects` / XTEL coordinate space). `heading` is the Z rotation
/// in radians and is only known for door arrivals.
#[derive(Debug, Clone)]
pub struct SpawnCellTarget {
    pub cell_form_key: String,
    pub position: [f64; 3],
    pub heading: Option<f64>,
}

#[derive(Debug, thiserror::Error)]
pub enum SpawnCellError {
    #[error("{SPAWN_CELL_VAR} is set but empty")]
    EmptyEditorId,
    #[error("{SPAWN_CELL_VAR} is not valid unicode: {0:?}")]
    NotUnicode(OsString),
    #[error("{SPAWN_VAR} must be three comma-separated finite numbers \"x,y,z\", got {0:?}")]
    InvalidPosition(String),
    #[error("cannot open the database for {SPAWN_CELL_VAR}={editor_id:?}: {source}")]
    DatabaseOpen {
        editor_id: String,
        source: anyhow::Error,
    },
    #[error("database query failed while resolving {SPAWN_CELL_VAR}={editor_id:?}: {source}")]
    DatabaseQuery {
        editor_id: String,
        source: anyhow::Error,
    },
    #[error("no cell with editor id {0:?} exists in the database")]
    CellNotFound(String),
    #[error("cell {0:?} has no form key in the database")]
    CellWithoutFormKey(String),
    #[error(
        "cell {0:?} is an exterior cell; {SPAWN_CELL_VAR} only supports interior cells \
         (use {SPAWN_VAR} to spawn at exterior coordinates)"
    )]
    NotInterior(String),
    #[error(
        "interior cell {0:?} has no door arrival and no placed object with a finite position \
         to spawn at"
    )]
    NoUsableSpawnPosition(String),
}

/// Reads `OPENFO4_SPAWN_CELL` and resolves it against the database. `None`
/// means the override is not set; every other failure is a distinct error.
pub fn from_env(db_path: &str) -> Result<Option<SpawnCellTarget>, SpawnCellError> {
    let Some(editor_id) = requested_editor_id()? else {
        return Ok(None);
    };
    let position = requested_position()?;
    resolve(db_path, &editor_id, position).map(Some)
}

/// Strictly parsed `OPENFO4_SPAWN` (three finite numbers). Combined with
/// `OPENFO4_SPAWN_CELL` it replaces the door arrival / median fallback.
pub fn requested_position() -> Result<Option<[f64; 3]>, SpawnCellError> {
    let value = match std::env::var(SPAWN_VAR) {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return Ok(None),
        Err(std::env::VarError::NotUnicode(value)) => {
            return Err(SpawnCellError::NotUnicode(value));
        }
    };
    let invalid = || SpawnCellError::InvalidPosition(value.clone());
    let parts: Vec<&str> = value.split(',').collect();
    let [x, y, z] = parts.as_slice() else {
        return Err(invalid());
    };
    let mut position = [0.0; 3];
    for (slot, part) in position.iter_mut().zip([x, y, z]) {
        *slot = part.trim().parse().map_err(|_| invalid())?;
    }
    if !is_finite(&position) {
        return Err(invalid());
    }
    Ok(Some(position))
}

fn requested_editor_id() -> Result<Option<String>, SpawnCellError> {
    let editor_id = match std::env::var(SPAWN_CELL_VAR) {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return Ok(None),
        Err(std::env::VarError::NotUnicode(value)) => {
            return Err(SpawnCellError::NotUnicode(value));
        }
    };
    if editor_id.trim().is_empty() {
        return Err(SpawnCellError::EmptyEditorId);
    }
    Ok(Some(editor_id))
}

fn resolve(
    db_path: &str,
    editor_id: &str,
    position_override: Option<[f64; 3]>,
) -> Result<SpawnCellTarget, SpawnCellError> {
    let query_error = |source| SpawnCellError::DatabaseQuery {
        editor_id: editor_id.to_string(),
        source,
    };

    let db = DbClient::new(db_path).map_err(|source| SpawnCellError::DatabaseOpen {
        editor_id: editor_id.to_string(),
        source,
    })?;
    let cell = db
        .get_cell_by_editor_id(editor_id)
        .map_err(query_error)?
        .ok_or_else(|| SpawnCellError::CellNotFound(editor_id.to_string()))?;
    let cell_form_key = cell
        .form_key
        .ok_or_else(|| SpawnCellError::CellWithoutFormKey(editor_id.to_string()))?;
    if !cell.is_interior {
        return Err(SpawnCellError::NotInterior(editor_id.to_string()));
    }

    if let Some(position) = position_override {
        return Ok(SpawnCellTarget {
            cell_form_key,
            position,
            heading: None,
        });
    }

    let door_arrival = db
        .get_teleport_into_cell(&cell_form_key)
        .map_err(query_error)?
        .map(|door| {
            (
                [door.dest_x, door.dest_y, door.dest_z],
                Some(door.dest_rot_z),
            )
        })
        .filter(|(position, _)| is_finite(position));
    let (position, heading) = if let Some(arrival) = door_arrival {
        arrival
    } else {
        let anchor = db
            .get_interior_anchors()
            .map_err(query_error)?
            .into_iter()
            .find(|(form_key, ..)| *form_key == cell_form_key)
            .map(|(_, x, y, z)| [x, y, z])
            .filter(is_finite)
            .ok_or_else(|| SpawnCellError::NoUsableSpawnPosition(editor_id.to_string()))?;
        (anchor, None)
    };

    Ok(SpawnCellTarget {
        cell_form_key,
        position,
        heading,
    })
}

fn is_finite(position: &[f64; 3]) -> bool {
    position.iter().all(|c| c.is_finite())
}
