//! Heat-haze refraction: screen-space distortion of the copied opaque scene colour inside
//! refraction cards, drawn in the translucent pass.

pub mod offset;
pub mod stage;
mod uniforms;

pub use stage::{RefractionFrameInputs, RefractionStage};
