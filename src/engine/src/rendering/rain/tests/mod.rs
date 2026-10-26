//! The tests of the rain: the numbers it hands the shader, and the passes drawn in a headless
//! frame.

mod frame;
mod plan;
mod scene;
mod splash;

use glam::{UVec2, Vec2, Vec3};

use super::RainInput;

const EYE: Vec3 = Vec3::new(1234.5, 300.0, -987.25);
const STREAK_TEXTURE: &str = "effects\\test_rain.dds";
/// The streamer generation the tests draw the world in.
const GENERATION: u64 = 3;

/// The rain of the weather the game ships for a downpour.
fn downpour() -> RainInput {
    RainInput {
        intensity: 1.0,
        occludable: true,
        fall_speed: 750.0,
        size: Vec2::new(1.5, 0.35),
        subtextures: UVec2::ZERO,
        density: 7.0,
        box_size: 1200.0,
        texture: STREAK_TEXTURE.into(),
        wind: Vec2::ZERO,
        tint: Vec3::ONE,
    }
}
