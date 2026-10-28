use glam::{Vec3, Vec4};

use super::lerp::lerp_fields;

/// Discriminants are mirrored by the `TONEMAP_*` constants in `shaders/tonemap.wgsl`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u32)]
pub enum TonemapOperator {
    Filmic = 0,
    AgX = 1,
}

impl TonemapOperator {
    pub const ALL: [TonemapOperator; 2] = [TonemapOperator::Filmic, TonemapOperator::AgX];

    pub fn label(self) -> &'static str {
        match self {
            TonemapOperator::Filmic => "Filmic (measured vanilla)",
            TonemapOperator::AgX => "AgX",
        }
    }
}

/// The cinematic block of an image space and its tint. They act on the tone-mapped picture in
/// linear light, before the display encoding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageSpaceCinematic {
    pub saturation: f32,
    pub brightness: f32,
    pub contrast: f32,
    pub tint_amount: f32,
    /// Linear.
    pub tint_color: Vec3,
}

lerp_fields!(ImageSpaceCinematic {
    saturation,
    brightness,
    contrast,
    tint_amount,
    tint_color,
});

impl ImageSpaceCinematic {
    /// Leaves the picture as the tone curve made it.
    pub const NEUTRAL: Self = Self {
        saturation: 1.0,
        brightness: 1.0,
        contrast: 1.0,
        tint_amount: 0.0,
        tint_color: Vec3::ONE,
    };
}

impl Default for ImageSpaceCinematic {
    fn default() -> Self {
        Self::NEUTRAL
    }
}

#[derive(Debug, Copy, Clone)]
pub struct TonemapSettings {
    pub operator: TonemapOperator,
    pub cinematic: ImageSpaceCinematic,
}

impl Default for TonemapSettings {
    fn default() -> Self {
        Self {
            operator: TonemapOperator::Filmic,
            cinematic: ImageSpaceCinematic::NEUTRAL,
        }
    }
}

/// The frame uniform's tonemap fields, laid out as `frame.wgsl` documents them.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct TonemapUniform {
    pub tonemap: Vec4,
    pub cinematic: Vec4,
    pub tint: Vec4,
}

impl TonemapSettings {
    pub fn uniform(&self, tonemap_e: f32) -> TonemapUniform {
        let cinematic = &self.cinematic;
        TonemapUniform {
            tonemap: Vec4::new(self.operator as u32 as f32, tonemap_e, 0.0, 0.0),
            cinematic: Vec4::new(
                cinematic.saturation,
                cinematic.brightness,
                cinematic.contrast,
                cinematic.tint_amount,
            ),
            tint: cinematic.tint_color.extend(0.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_uniform_carries_the_operator_the_toe_and_the_cinematic_fields() {
        let uniform = TonemapSettings {
            operator: TonemapOperator::AgX,
            cinematic: ImageSpaceCinematic {
                saturation: 0.5,
                brightness: 2.0,
                contrast: 1.5,
                tint_amount: 0.25,
                tint_color: Vec3::new(1.0, 0.5, 0.0),
            },
        }
        .uniform(0.04);
        assert_eq!(
            uniform,
            TonemapUniform {
                tonemap: Vec4::new(1.0, 0.04, 0.0, 0.0),
                cinematic: Vec4::new(0.5, 2.0, 1.5, 0.25),
                tint: Vec4::new(1.0, 0.5, 0.0, 0.0),
            }
        );
    }

    #[test]
    fn operator_discriminants_match_the_shader_constants() {
        assert_eq!(TonemapOperator::Filmic as u32, 0);
        assert_eq!(TonemapOperator::AgX as u32, 1);
        let shader = include_str!("../shaders/tonemap.wgsl");
        assert!(shader.contains("const TONEMAP_FILMIC = 0u;"));
        assert!(shader.contains("const TONEMAP_AGX = 1u;"));
    }
}
