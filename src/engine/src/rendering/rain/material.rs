//! How the rain's materials draw. The streaks follow the raindrop material the weathers' particle
//! geometry names (`Effects\FXWeatherRaindrops.BGEM`), the splashes the effect material of the
//! game's ground splash particles. Neither reaches the engine through the database, so what they
//! say is written down here.

use glam::Vec3;

/// The raindrops add the light they catch to the scene: source alpha onto one. Their texture is a
/// mid grey with darker cores, so drawn over the scene it would darken the sky behind every drop.
pub(super) const STREAK_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::SrcAlpha,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: KEEP_TARGET_ALPHA,
};

/// The splashes are drawn over the scene with straight alpha.
pub(super) const SPLASH_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::SrcAlpha,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: KEEP_TARGET_ALPHA,
};

const KEEP_TARGET_ALPHA: wgpu::BlendComponent = wgpu::BlendComponent {
    src_factor: wgpu::BlendFactor::Zero,
    dst_factor: wgpu::BlendFactor::One,
    operation: wgpu::BlendOperation::Add,
};

/// The base colour of the raindrop material, which the streak texture is multiplied by. The
/// material takes the light around it in full; the rain's tint is that light.
pub(super) const STREAK_BASE_COLOR: Vec3 = Vec3::new(205.0 / 255.0, 213.0 / 255.0, 220.0 / 255.0);
