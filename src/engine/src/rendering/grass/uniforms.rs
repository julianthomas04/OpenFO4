//! The grass shader's uniform blocks: the frame-wide wind and fade parameters
//! and the per-batch material and type parameters. Mirrors `GrassFrame` and
//! `GrassBatch` in `shaders/grass.wgsl`.

use bytemuck::{Pod, Zeroable};

use super::fade::GrassFade;
use super::wind::{GrassWind, wave_rate};
use crate::assets::types::grass::GrassFlags;

/// The pixel shader's alpha floor, `alpha - 4/255 < 0` discards.
pub const MIN_ALPHA: f32 = 4.0 / 255.0;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct GrassFrameUniform {
    /// x: wind heading, y: magnitude min, z: magnitude max (game units),
    /// w: wave frequency.
    pub wind: [f32; 4],
    /// x: fade start, y: fade range (engine units).
    pub fade: [f32; 4],
}

impl GrassFrameUniform {
    pub fn new(wind: GrassWind, fade: GrassFade) -> Self {
        Self {
            wind: [
                wind.direction,
                wind.magnitude_min,
                wind.magnitude_max,
                wind.frequency,
            ],
            fade: [fade.start, fade.range, 0.0, 0.0],
        }
    }
}

pub const BATCH_UNIFORM_SCALING: u32 = 1;
pub const BATCH_UNTINTED: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct GrassBatchUniform {
    pub material_id: u32,
    pub alpha_cutoff: f32,
    /// Radians per second of the wave before the wind frequency applies.
    pub wave_rate: f32,
    pub flags: u32,
}

impl GrassBatchUniform {
    pub fn new(
        material_id: u32,
        material_cutoff: f32,
        flags: GrassFlags,
        wave_period: f32,
        has_color_stream: bool,
    ) -> Self {
        let mut bits = 0;
        if flags.uniform_scaling() {
            bits |= BATCH_UNIFORM_SCALING;
        }
        if !has_color_stream {
            bits |= BATCH_UNTINTED;
        }
        Self {
            material_id,
            alpha_cutoff: material_cutoff.max(MIN_ALPHA),
            wave_rate: wave_rate(wave_period),
            flags: bits,
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals parsed or copied, not computed"
)]
mod tests {
    use super::*;

    #[test]
    fn frame_uniform_is_two_vec4s() {
        assert_eq!(std::mem::size_of::<GrassFrameUniform>(), 32);
        assert_eq!(std::mem::size_of::<GrassBatchUniform>(), 16);
    }

    #[test]
    fn frame_uniform_carries_wind_then_fade() {
        let uniform = GrassFrameUniform::new(
            GrassWind {
                direction: 1.5,
                magnitude_min: 2.0,
                magnitude_max: 30.0,
                frequency: 1.25,
            },
            GrassFade {
                start: 320.0,
                range: 110.0,
            },
        );
        assert_eq!(uniform.wind, [1.5, 2.0, 30.0, 1.25]);
        assert_eq!(uniform.fade, [320.0, 110.0, 0.0, 0.0]);
    }

    #[test]
    fn batch_flags_follow_the_grass_flags() {
        let uniform = |bits| GrassBatchUniform::new(3, 0.4, GrassFlags(bits), 2.0, true).flags;
        assert_eq!(uniform(0), 0);
        assert_eq!(uniform(GrassFlags::UNIFORM_SCALING), BATCH_UNIFORM_SCALING);
        assert_eq!(
            uniform(GrassFlags::FIT_TO_SLOPE),
            0,
            "slope fitting lives in the blades' up axis, not the shader"
        );
        assert_eq!(
            uniform(GrassFlags::VERTEX_LIGHTING),
            0,
            "vanilla's vertex lighting flag does not change the shading normals"
        );
    }

    #[test]
    fn a_model_without_a_colour_stream_is_flagged_untinted() {
        let flags = |has_color_stream| {
            GrassBatchUniform::new(
                3,
                0.4,
                GrassFlags(GrassFlags::UNIFORM_SCALING),
                2.0,
                has_color_stream,
            )
            .flags
        };
        assert_eq!(flags(true), BATCH_UNIFORM_SCALING);
        assert_eq!(flags(false), BATCH_UNIFORM_SCALING | BATCH_UNTINTED);
    }

    #[test]
    fn batch_alpha_cutoff_never_falls_below_the_pixel_shader_floor() {
        let cutoff =
            |material| GrassBatchUniform::new(0, material, GrassFlags(0), 1.0, true).alpha_cutoff;
        assert_eq!(cutoff(0.0), MIN_ALPHA);
        assert_eq!(cutoff(0.4), 0.4);
    }

    #[test]
    fn batch_carries_material_and_the_wave_rate_of_the_gras_period() {
        let uniform = GrassBatchUniform::new(17, 0.4, GrassFlags(0), 100.0, true);
        assert_eq!(uniform.material_id, 17);
        assert!((uniform.wave_rate - wave_rate(100.0)).abs() < 1e-6);
        assert!(
            uniform.wave_rate < 2.0,
            "a period of 100 is about one radian per second, not 100"
        );
    }
}
