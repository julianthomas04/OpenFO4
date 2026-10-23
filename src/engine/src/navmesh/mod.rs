//! Runtime navigation mesh: storage, cross-mesh linking and pathfinding queries.

mod build;
mod decode;
mod error;
pub mod filter;
pub mod flags;
mod geometry;
mod grid;
mod link;
mod mesh;
mod nav_ref;
mod portal;
pub mod query;
mod residency;
mod source;
mod validate;
mod world;

#[cfg(test)]
mod tests;

pub use build::{BuildError, BuildFailure, BuildOutcome, build_mesh, build_meshes};
pub use decode::{DecodeError, NavMeshRecord, SUPPORTED_FORMAT_VERSION, decode};
pub use error::NavMeshError;
pub use filter::{FilterError, QueryFilter};
pub use link::{EdgeLink, LinkType};
pub use mesh::{Aabb, NO_NEIGHBOUR, NavMesh, TriangleView};
pub use nav_ref::NavRef;
pub use portal::Span;
pub use residency::{CellNavmeshes, NavResidency};
pub use source::{DoorLinkSource, EdgeLinkSource, NavMeshSource};
pub use world::{
    LoadReport, NavLink, NavStats, NavWorld, RejectedLink, ResidentMesh, UnloadReport, WorldError,
};
