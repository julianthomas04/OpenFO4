//! CPU particle simulation: emitters, modifiers and camera-facing quads built
//! from the converter's `particle_system` extras.

mod background_cache;
pub mod constants;
mod curves;
mod instance;
mod quads;
pub mod rng;
mod template;
#[cfg(test)]
mod tests;
mod world;

pub use background_cache::{BackgroundCache, Lookup};
pub use instance::SystemInstance;
pub use quads::CameraBasis;
pub use template::{
    NodeTransforms, ParticleSystemTemplate, TemplateError, TemplateSet, node_transforms,
};
pub use world::{InstanceId, ParticleWorld, ParticleWorldConfig};
