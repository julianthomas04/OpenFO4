pub mod events;
pub mod timings;

pub use events::{FootTag, FootstepEvent, Gait, SurfaceMaterial};
pub use timings::{GaitCycle, GaitTimings, GaitTimingsError};
