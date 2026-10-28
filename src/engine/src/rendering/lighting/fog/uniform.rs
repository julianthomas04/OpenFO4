use glam::Vec4;

use super::measured::cap_ramp_start;
use super::{Fog, HeightBand};
use crate::rendering::lighting::frame_uniform::FrameUniform;

/// The `FrameUniform` fog fields, laid out as in `shaders/frame.wgsl`. Fog that does not render
/// is all zeros: its far distance does not exceed its near one, which `fog_at` reads as no fog.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct FogUniformFields {
    /// rgb: near colour, w: distance at which the fog starts.
    pub near_color: Vec4,
    /// rgb: far colour, also the fill, w: distance from which the fog is full.
    pub far_color: Vec4,
    /// rgb: near colour at the top of the height bands, w: share of the normal branch's fog
    /// colour the sunlight leaves.
    pub near_high_color: Vec4,
    /// rgb: far colour at the top of the height bands.
    pub far_high_color: Vec4,
    /// x: power, y: Max, z: high density scale, w: distance at which the fog capped at Max
    /// starts ramping to the fill.
    pub params: Vec4,
    /// The near band: `share = z * saturate(height * x + y)`.
    pub near_band: Vec4,
    /// The far band, as the near one.
    pub far_band: Vec4,
}

fn band_fields(band: HeightBand) -> Vec4 {
    let (scale, offset) = band.share_line();
    Vec4::new(scale, offset, band.weight, 0.0)
}

impl FogUniformFields {
    const DISABLED: Self = Self {
        near_color: Vec4::ZERO,
        far_color: Vec4::ZERO,
        near_high_color: Vec4::ZERO,
        far_high_color: Vec4::ZERO,
        params: Vec4::ZERO,
        near_band: Vec4::ZERO,
        far_band: Vec4::ZERO,
    };

    pub fn write_to(&self, frame: &mut FrameUniform) {
        frame.fog_near_color = self.near_color;
        frame.fog_far_color = self.far_color;
        frame.fog_near_high_color = self.near_high_color;
        frame.fog_far_high_color = self.far_high_color;
        frame.fog_params = self.params;
        frame.fog_near_band = self.near_band;
        frame.fog_far_band = self.far_band;
    }

    pub fn of_frame(frame: &FrameUniform) -> Self {
        Self {
            near_color: frame.fog_near_color,
            far_color: frame.fog_far_color,
            near_high_color: frame.fog_near_high_color,
            far_high_color: frame.fog_far_high_color,
            params: frame.fog_params,
            near_band: frame.fog_near_band,
            far_band: frame.fog_far_band,
        }
    }

    pub fn renders(&self) -> bool {
        self.far_color.w > self.near_color.w
    }
}

impl Fog {
    pub fn uniform_fields(&self) -> FogUniformFields {
        let Some(fog) = self.rendered() else {
            return FogUniformFields::DISABLED;
        };
        let colors = fog.colors;
        FogUniformFields {
            near_color: colors.near.extend(fog.near),
            far_color: colors.far.extend(fog.far),
            near_high_color: colors.near_high.extend(fog.sunlight.kept),
            far_high_color: colors.far_high.extend(0.0),
            params: Vec4::new(
                fog.power,
                fog.max,
                fog.height.high_density_scale,
                cap_ramp_start(fog.near, fog.far),
            ),
            near_band: band_fields(fog.height.near),
            far_band: band_fields(fog.height.far),
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "the packed values are exact in f32")]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::rendering::lighting::fog::{
        FogColors, FogParams, HeightBand, HeightBands, SunlightTerm,
    };

    fn params() -> FogParams {
        FogParams {
            colors: FogColors {
                near: Vec3::new(0.1, 0.2, 0.3),
                far: Vec3::new(0.4, 0.5, 0.6),
                near_high: Vec3::new(0.7, 0.8, 0.9),
                far_high: Vec3::new(1.0, 1.1, 1.2),
            },
            near: 10.0,
            far: 100.0,
            power: 2.0,
            max: 0.5,
            height: HeightBands {
                near: HeightBand::new(20.0, 10.0),
                far: HeightBand::new(50.0, 25.0),
                high_density_scale: 0.25,
            },
            sunlight: SunlightTerm { kept: 0.9 },
        }
    }

    #[test]
    fn enabled_fog_packs_every_field() {
        let fields = Fog::Enabled(params()).uniform_fields();
        assert_eq!(fields.near_color, Vec4::new(0.1, 0.2, 0.3, 10.0));
        assert_eq!(fields.far_color, Vec4::new(0.4, 0.5, 0.6, 100.0));
        assert_eq!(fields.near_high_color, Vec4::new(0.7, 0.8, 0.9, 0.9));
        assert_eq!(fields.far_high_color, Vec4::new(1.0, 1.1, 1.2, 0.0));
        assert_eq!(fields.params.truncate(), Vec3::new(2.0, 0.5, 0.25));
        assert!(
            (fields.params.w - 78.94).abs() < 1e-4,
            "{}",
            fields.params.w
        );
        assert_eq!(fields.near_band, Vec4::new(0.05, -0.5, 1.0, 0.0));
        assert_eq!(fields.far_band, Vec4::new(0.02, -0.5, 1.0, 0.0));
        assert!(fields.renders());
    }

    #[test]
    fn fog_that_does_not_render_packs_to_zeros() {
        let inverted = Fog::Enabled(FogParams {
            near: 100.0,
            far: 10.0,
            ..params()
        });
        for fog in [Fog::Disabled, inverted] {
            let fields = fog.uniform_fields();
            assert_eq!(fields, FogUniformFields::DISABLED, "{fog:?}");
            assert!(!fields.renders());
        }
    }

    #[test]
    fn the_frame_uniform_carries_the_fields_there_and_back() {
        let fields = Fog::Enabled(params()).uniform_fields();
        let mut frame: FrameUniform = bytemuck::Zeroable::zeroed();
        fields.write_to(&mut frame);
        assert_eq!(FogUniformFields::of_frame(&frame), fields);
    }
}
