//! The water world: Fallout 4's rules for where water is and how high, shared
//! by the renderer (what to draw, whether the camera is under water) and the
//! simulation (the water a body floats in). The rules are in game units and
//! pure ([`height`], [`volume`], [`camera`], [`lod`]); [`WaterWorld`] ties them
//! to the database and the placed water of the resident cells.

pub mod camera;
pub mod cell_water;
pub mod height;
pub mod lod;
mod physics;
pub mod volume;
mod world;

pub use camera::{CameraWater, Immersion, NearPlane, VIRTUAL_NEAR_DISTANCE, camera_water};
pub use cell_water::{CellSurface, LoadedCell};
pub use lod::{CellPlane, LodWater};
pub use physics::physics_query;
pub use volume::{PlacedWaterVolume, SurfacePlane, VolumeShapes};
pub use world::{
    Scope, SharedWaterWorld, WaterHit, WaterWorld, exterior_cell_of,
    worldspace_default_water_height,
};
