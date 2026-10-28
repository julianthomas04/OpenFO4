use glam::{Mat4, UVec4, Vec4};

use super::ambient::AMBIENT_CUBE_FACES;

pub const MAX_CASCADES: usize = 4;

/// Mirrors `FrameUniform` in `shaders/frame.wgsl`; bound at group 1 by every
/// world pipeline. Only vec4/mat4 members so the WGSL layout equals `repr(C)`.
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FrameUniform {
    pub view_proj: Mat4,
    pub inv_view_proj: Mat4,
    pub camera_pos: Vec4,
    /// xyz: unit direction towards the sun, w: cosine of the sun disc's angular radius.
    pub to_sun: Vec4,
    /// rgb: linear sun colour times intensity, w: that intensity (the HDR sunlight scale, 1 when
    /// none applies), for consumers that need the unscaled colour.
    pub sun_illuminance: Vec4,
    /// x: the sun's alpha around sunrise and sunset, 0 to 1, for the water. It stays the sun's
    /// while `to_sun` follows the moon at night.
    pub daylight: Vec4,
    /// rgb: linear ambient colour of each cube face, order +X, -X, +Y, -Y, +Z, -Z (engine axes).
    pub ambient_cube: [Vec4; AMBIENT_CUBE_FACES],
    pub sky_zenith: Vec4,
    pub sky_horizon: Vec4,
    pub sky_ground: Vec4,
    /// The fog, laid out and documented by `fog::FogUniformFields`.
    pub fog_near_color: Vec4,
    pub fog_far_color: Vec4,
    pub fog_near_high_color: Vec4,
    pub fog_far_high_color: Vec4,
    pub fog_params: Vec4,
    pub fog_near_band: Vec4,
    pub fog_far_band: Vec4,
    pub cascade_view_proj: [Mat4; MAX_CASCADES],
    /// xyz: world-space centre, w: radius.
    pub cascade_spheres: [Vec4; MAX_CASCADES],
    /// x: constant, y: slope, z: normal-offset bias (texels), w: split blend band (engine units).
    pub shadow_params: Vec4,
    /// World size of one shadow-map texel, one component per cascade.
    pub cascade_texel_world: Vec4,
    /// x: compare taps of the sun shadow filter.
    pub shadow_filter: UVec4,
    /// x: shadows enabled, y: cascade count, z: debug view, w: material sampler anisotropy clamp.
    pub flags: UVec4,
    /// xyz: unit view direction.
    pub camera_forward: Vec4,
    /// xy: viewport size in pixels, zw: its reciprocal.
    pub viewport: Vec4,
    /// See `local_lights::cluster_grid::cluster_grid`.
    pub cluster_grid: UVec4,
    /// See `ClusterDepth::uniform`.
    pub cluster_z: Vec4,
    /// See `TonemapSettings::uniform`.
    pub tonemap: Vec4,
    /// See `TextureFeedback::frame_params`.
    pub texture_feedback: UVec4,
    /// x: seconds since start wrapped at `TIME_WRAP_SECONDS`, y: water surface
    /// height at the camera, z: 1 while the camera is underwater, w: the share
    /// of the near plane's height under the surface (0 to 1).
    pub time_water: Vec4,
}

/// The time uniform restarts at zero after this many seconds. An f32 holds
/// whole seconds exactly and keeps sub-millisecond resolution up to 2^12 s;
/// unbounded it degrades to tens of milliseconds within hours, which makes
/// wave phase visibly step. 4096 s (68 min) is a power of two, so the wrap is
/// exact in f32, and consumers using periods dividing it hitch-free.
pub const TIME_WRAP_SECONDS: f32 = 4096.0;

pub fn wrapped_time(elapsed_seconds: f32) -> f32 {
    elapsed_seconds.rem_euclid(TIME_WRAP_SECONDS)
}

const _: () = assert!(std::mem::size_of::<FrameUniform>() == 944);

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "the wrap is exact in f32 by construction")]
mod tests {
    use super::*;

    #[test]
    fn water_fields_close_the_uniform_at_the_wgsl_offset() {
        assert_eq!(std::mem::offset_of!(FrameUniform, time_water), 928);
        assert_eq!(std::mem::size_of::<FrameUniform>() % 16, 0);
    }

    #[test]
    fn the_daylight_field_follows_the_sun_illuminance() {
        assert_eq!(std::mem::offset_of!(FrameUniform, daylight), 176);
    }

    #[test]
    fn time_wraps_exactly_at_the_period() {
        assert_eq!(wrapped_time(0.5), 0.5);
        assert_eq!(wrapped_time(TIME_WRAP_SECONDS), 0.0);
        assert_eq!(wrapped_time(TIME_WRAP_SECONDS + 1.25), 1.25);
    }
}
