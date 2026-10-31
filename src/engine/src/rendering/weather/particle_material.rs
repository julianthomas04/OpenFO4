//! The effect material a weather's precipitation is drawn with, as the installer reads it from the
//! BGEM its shader particle geometry names.

use glam::Vec3;
use serde::Deserialize;

use crate::assets::types::effect_material::BlendFactor;

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct ParticleMaterialData {
    /// The factors are not used while this is off.
    pub blend_enabled: bool,
    pub blend_src: BlendFactor,
    pub blend_dst: BlendFactor,
    /// 0..=255 per channel.
    pub base_color: [u8; 3],
    pub depth_write: bool,
    pub effect_lighting: bool,
    /// Stored even when `effect_lighting` is off.
    pub lighting_influence: f32,
}

impl ParticleMaterialData {
    /// The stored factors, `None` while blending is off.
    pub fn blend_factors(&self) -> Option<(BlendFactor, BlendFactor)> {
        self.blend_enabled
            .then_some((self.blend_src, self.blend_dst))
    }

    /// 0..=1 per channel.
    pub fn base_color(&self) -> Vec3 {
        Vec3::from_array(self.base_color.map(f32::from)) / 255.0
    }
}

#[cfg(test)]
pub(super) mod fixtures {
    use serde_json::{Value, json};

    /// The raindrop material the game ships.
    pub(in crate::rendering::weather) fn raindrop_material() -> Value {
        json!({
            "blend_enabled": true, "blend_src": "src_alpha", "blend_dst": "one",
            "base_color": [205, 213, 220],
            "depth_write": false, "effect_lighting": true, "lighting_influence": 1.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::raindrop_material;
    use super::*;

    fn material(json: serde_json::Value) -> ParticleMaterialData {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn enabled_blending_yields_the_stored_factors() {
        assert_eq!(
            material(raindrop_material()).blend_factors(),
            Some((BlendFactor::SrcAlpha, BlendFactor::One))
        );
    }

    #[test]
    fn disabled_blending_ignores_the_stored_factors() {
        let mut json = raindrop_material();
        json["blend_enabled"] = false.into();

        assert_eq!(material(json).blend_factors(), None);
    }

    #[test]
    fn the_base_colour_is_scaled_to_unit_range() {
        let mut json = raindrop_material();
        json["base_color"] = serde_json::json!([0, 51, 255]);

        assert_eq!(material(json).base_color(), Vec3::new(0.0, 0.2, 1.0));
    }
}
