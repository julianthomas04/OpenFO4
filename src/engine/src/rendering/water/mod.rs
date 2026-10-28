//! The water renderer: the stage the water passes draw in, the WATR to
//! register mapping, the shader textures and pipelines. Where water is and
//! whether the camera is in it is the shared [`crate::water_world`].

mod config;
pub mod constants;
mod debug_output;
pub mod displacement;
pub mod permutation;
pub mod pipelines;
pub mod placement;
mod settings;
pub mod ssr;
mod stage;
#[cfg(test)]
mod test_fixtures;
pub mod textures;
mod underwater_fog;

pub use crate::water_world::CameraWater;
pub use config::WaterConfig;
pub use constants::WaterSetting;
pub use debug_output::WaterDebugOutput;
pub use placement::{PlacedWater, WaterPlane};
pub use settings::{WaterSettings, water_panel};
pub use stage::{UnderwaterDofInputs, WaterStage};
