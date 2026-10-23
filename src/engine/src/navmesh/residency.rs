//! Which navmeshes each streamed cell put into the world, and the add/withdraw rules that go
//! with it, kept apart from the streamer so they can be tested without one.

use tracing::{error, warn};

use super::mesh::NavMesh;
use super::world::NavWorld;

/// The meshes one cell added to the world; the cell hands this back to withdraw exactly them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CellNavmeshes {
    form_ids: Vec<u32>,
}

impl CellNavmeshes {
    pub fn form_ids(&self) -> &[u32] {
        &self.form_ids
    }

    pub fn is_empty(&self) -> bool {
        self.form_ids.is_empty()
    }
}

/// The resident nav world plus the bookkeeping of which cell owns which mesh.
#[derive(Debug, Default)]
pub struct NavResidency {
    world: NavWorld,
}

impl NavResidency {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn world(&self) -> &NavWorld {
        &self.world
    }

    /// Adds a cell's meshes and returns the ones that went in. A mesh whose form id is already
    /// resident is refused and not owned by this cell, so retiring the cell leaves the earlier
    /// owner's mesh alone. Rejected links and data-quality counters are logged per mesh.
    pub fn integrate(&mut self, meshes: Vec<NavMesh>) -> CellNavmeshes {
        let mut added = Vec::with_capacity(meshes.len());
        for mesh in meshes {
            let form_id = mesh.form_id();
            match self.world.add(mesh) {
                Ok(report) => {
                    added.push(form_id);
                    for rejected in &report.rejected {
                        warn!(
                            "navmesh {form_id:#08x}: authored link {} of mesh {:#08x} rejected (target triangle or edge out of range); it stays pending",
                            rejected.authored_index, rejected.source_form_id
                        );
                    }
                    if report.asymmetric_neighbours > 0 {
                        warn!(
                            "navmesh {form_id:#08x}: {} one-way neighbour edge(s)",
                            report.asymmetric_neighbours
                        );
                    }
                }
                Err(e) => error!("navmesh {form_id:#08x} not added: {e}"),
            }
        }
        CellNavmeshes { form_ids: added }
    }

    /// Withdraws the meshes a cell added. Meshes already gone (a teardown in between) are logged,
    /// not fatal.
    pub fn retire(&mut self, cell: &CellNavmeshes) {
        for &form_id in &cell.form_ids {
            if let Err(e) = self.world.remove(form_id) {
                error!("navmesh {form_id:#08x} not removed: {e}");
            }
        }
    }

    /// Drops every resident mesh; handles held by cells become stale.
    pub fn clear(&mut self) {
        self.world.clear();
    }
}
