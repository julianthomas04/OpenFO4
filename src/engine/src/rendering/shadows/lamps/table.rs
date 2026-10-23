use glam::{Mat4, UVec4, Vec4};

use super::geometry::{CUBE_FACES, LampFrustums, LampProjection};
use super::settings::{LampShadowConfig, LampShadowSettings, MAX_SHADOWED_LAMPS};

/// Table slots: the shadowed lamps plus as many fading out.
pub const MAX_SHADOW_SLOTS: usize = 2 * MAX_SHADOWED_LAMPS as usize;
/// Mirrors `SHADOW_SLOT_NONE` in `light_data.wgsl`.
pub const SHADOW_SLOT_NONE: u32 = u32::MAX;

/// Mirrors `ShadowLamp` in `frame.wgsl`. A zeroed record (strength 0) is
/// never sampled.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuShadowLamp {
    faces: [Mat4; CUBE_FACES],
    /// Per face: atlas uv origin (xy) and side (z); z = 0 for a face without
    /// a tile.
    tiles: [Vec4; CUBE_FACES],
    /// The lamp basis (forward, up, side), xyz.
    axes: [Vec4; 3],
    /// xyz: position, w: near clip.
    position: Vec4,
    /// x: strength, y: world texel size per unit of face-axis distance,
    /// z: 1 for cube storage, 0 for one spot face.
    params: Vec4,
}

const _: () = assert!(std::mem::size_of::<GpuShadowLamp>() == 560);

impl GpuShadowLamp {
    pub fn new(
        frustums: &LampFrustums,
        tiles: [Vec4; CUBE_FACES],
        position: glam::Vec3,
        near_clip: f32,
        strength: f32,
    ) -> Self {
        let basis = frustums.basis;
        let cube = match frustums.projection {
            LampProjection::Spot { .. } => 0.0,
            LampProjection::Cube { .. } => 1.0,
        };
        Self {
            faces: frustums.view_projs,
            tiles,
            axes: [basis.forward, basis.up, basis.side].map(|axis| axis.extend(0.0)),
            position: position.extend(near_clip),
            params: Vec4::new(strength, frustums.texel_per_distance, cube, 0.0),
        }
    }
}

/// Mirrors `ShadowLampTable` in `frame.wgsl`; bound as the frame group's
/// lamp-shadow uniform.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuShadowLampTable {
    pub lamps: [GpuShadowLamp; MAX_SHADOW_SLOTS],
    /// Constant, slope and normal-offset bias in texels (× fShadowBiasScale),
    /// w: squared receiver fade distance.
    pub bias: Vec4,
    /// x: PCF taps per axis.
    pub filter_taps: UVec4,
}

const _: () = assert!(std::mem::size_of::<GpuShadowLampTable>() == 560 * MAX_SHADOW_SLOTS + 32);

impl GpuShadowLampTable {
    pub fn header(&mut self, config: &LampShadowConfig, settings: &LampShadowSettings) {
        self.bias = Vec4::new(
            settings.constant_bias * config.bias_scale,
            settings.slope_bias * config.bias_scale,
            settings.normal_offset * config.bias_scale,
            config.fade_end * config.fade_end,
        );
        self.filter_taps = UVec4::new(config.filter.taps_per_axis(), 0, 0, 0);
    }
}
