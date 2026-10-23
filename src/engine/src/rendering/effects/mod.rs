//! Effect-shader rendering: translated effect shaders composed into pipelines, drawn by the
//! effect stage.

pub mod constants;
pub mod draw_buffer;
pub mod geometry;
pub mod lifetime;
pub mod lights_pick;
pub mod material_bindings;
pub mod particle_draw;
pub mod pipeline;
pub mod shader;
pub mod stage;
#[cfg(test)]
pub(crate) mod test_fixtures;
pub mod warmup;
