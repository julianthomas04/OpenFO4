//! Building runtime meshes from database records, tolerating and reporting bad ones.

use super::decode::{DecodeError, NavMeshRecord, decode};
use super::error::NavMeshError;
use super::mesh::NavMesh;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    #[error("blob decode failed: {0}")]
    Decode(#[from] DecodeError),
    #[error("mesh rejected: {0}")]
    Mesh(#[from] NavMeshError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildFailure {
    pub form_key: String,
    pub error: BuildError,
}

#[derive(Debug, Default)]
pub struct BuildOutcome {
    pub meshes: Vec<NavMesh>,
    pub failures: Vec<BuildFailure>,
}

pub fn build_mesh(record: &NavMeshRecord) -> Result<NavMesh, BuildError> {
    Ok(NavMesh::new(decode(record)?)?)
}

/// Builds every record; a failing record is skipped and listed in `failures` so the caller decides
/// how loudly to report it.
pub fn build_meshes(records: &[NavMeshRecord]) -> BuildOutcome {
    let mut outcome = BuildOutcome::default();
    for record in records {
        match build_mesh(record) {
            Ok(mesh) => outcome.meshes.push(mesh),
            Err(error) => outcome.failures.push(BuildFailure {
                form_key: record.form_key.clone(),
                error,
            }),
        }
    }
    outcome
}
