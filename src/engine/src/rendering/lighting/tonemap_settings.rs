use glam::Vec4;

/// Discriminants are mirrored by the `TONEMAP_*` constants in `shaders/tonemap.wgsl`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u32)]
pub enum TonemapOperator {
    Reinhard = 0,
    AgX = 1,
}

impl TonemapOperator {
    pub const ALL: [TonemapOperator; 2] = [TonemapOperator::Reinhard, TonemapOperator::AgX];

    pub fn label(self) -> &'static str {
        match self {
            TonemapOperator::Reinhard => "Extended Reinhard (vanilla-like)",
            TonemapOperator::AgX => "AgX",
        }
    }
}

/// Scene luminance that maps to full white in the extended Reinhard curve; calibrated against the vanilla reference screenshot.
pub const DEFAULT_WHITE_POINT: f32 = 2.5;
/// Chroma multiplier applied after tonemapping; calibrated against the vanilla reference screenshot.
pub const DEFAULT_SATURATION: f32 = 1.15;

#[derive(Debug, Copy, Clone)]
pub struct TonemapSettings {
    pub operator: TonemapOperator,
    pub white_point: f32,
    pub saturation: f32,
}

impl Default for TonemapSettings {
    fn default() -> Self {
        Self {
            operator: TonemapOperator::Reinhard,
            white_point: DEFAULT_WHITE_POINT,
            saturation: DEFAULT_SATURATION,
        }
    }
}

impl TonemapSettings {
    /// x: operator, y: white point, z: saturation.
    pub fn uniform(&self) -> Vec4 {
        Vec4::new(
            self.operator as u32 as f32,
            self.white_point,
            self.saturation,
            0.0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_uniform_carries_operator_white_point_and_saturation() {
        let uniform = TonemapSettings {
            operator: TonemapOperator::AgX,
            white_point: 4.0,
            saturation: 0.9,
        }
        .uniform();
        assert_eq!(uniform, Vec4::new(1.0, 4.0, 0.9, 0.0));
    }

    #[test]
    fn operator_discriminants_match_the_shader_constants() {
        // TONEMAP_REINHARD = 0u and TONEMAP_AGX = 1u in shaders/tonemap.wgsl.
        assert_eq!(TonemapOperator::Reinhard as u32, 0);
        assert_eq!(TonemapOperator::AgX as u32, 1);
        let shader = include_str!("../shaders/tonemap.wgsl");
        assert!(shader.contains("const TONEMAP_REINHARD = 0u;"));
        assert!(shader.contains("const TONEMAP_AGX = 1u;"));
    }
}
