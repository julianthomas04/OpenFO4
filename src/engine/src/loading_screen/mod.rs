//! Which loading screen a cell switch shows: the kind of screen from where the player comes and
//! goes, and the large screen picked from the catalog for the destination.

mod classify;
mod context;
mod select;

pub use classify::{LoadKind, Place, classify};
pub use context::LoadScreenContext;
pub use select::select;
