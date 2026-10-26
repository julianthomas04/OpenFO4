//! How the rain's materials draw. The streaks follow the effect material the weather's particle
//! geometry names, as the database gives it. The splashes follow the effect material of the
//! game's ground splash particles, which no particle geometry names and so does not reach the
//! engine through the database; what it says is written down here.

use glam::Vec3;

use crate::rendering::effects::pipeline::BlendFactors;

/// The blend and base colour of the raindrop material.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreakMaterial {
    /// `None` draws the streaks unblended.
    pub blend: Option<BlendFactors>,
    /// 0..=1 per channel. The streak texture is multiplied by it and by the light the rain
    /// catches.
    pub base_color: Vec3,
}

impl StreakMaterial {
    /// What the streaks draw with when the database has no material data for them: the raindrop
    /// material the game's rainy weathers name (`Effects\FXWeatherRaindrops.BGEM`). It adds the
    /// light the drops catch to the scene, source alpha onto one; its texture is a mid grey with
    /// darker cores, so drawn with straight alpha it would darken the sky behind every drop.
    pub const FALLBACK: Self = Self {
        blend: Some((wgpu::BlendFactor::SrcAlpha, wgpu::BlendFactor::One)),
        base_color: Vec3::new(205.0 / 255.0, 213.0 / 255.0, 220.0 / 255.0),
    };
}

/// Whether streaks drawn with `blend` add their light to the scene (the destination is kept
/// whole), so the fog takes its share of that light instead of bringing its own colour. Any
/// other blend draws the streaks as a surface, which the fog covers like the rest of the scene.
pub(super) fn adds_light(blend: Option<BlendFactors>) -> bool {
    matches!(blend, Some((_, wgpu::BlendFactor::One)))
}

/// The splashes are drawn over the scene with straight alpha.
pub(super) const SPLASH_BLEND: BlendFactors = (
    wgpu::BlendFactor::SrcAlpha,
    wgpu::BlendFactor::OneMinusSrcAlpha,
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_blend_that_keeps_the_destination_whole_adds_light() {
        assert!(adds_light(StreakMaterial::FALLBACK.blend));
        assert!(adds_light(Some((
            wgpu::BlendFactor::One,
            wgpu::BlendFactor::One
        ))));
        assert!(!adds_light(Some(SPLASH_BLEND)));
        assert!(!adds_light(Some((
            wgpu::BlendFactor::Dst,
            wgpu::BlendFactor::Zero
        ))));
        assert!(!adds_light(None));
    }
}
