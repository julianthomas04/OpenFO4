//! The uniform blocks of `shaders/refraction.wgsl` and how they are filled.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec4};

use super::offset::{
    FAR_VIEW_DEPTH_EU, LayerTransforms, MAX_DEPTH_GAIN, MAX_OFFSET_UV, MIN_VIEW_DEPTH_EU,
    REFERENCE_DEPTH_EU, REJECT_BIAS_EU, REPLACE_OFFSET_UV, SOFT_FADE_EU,
};
use crate::rendering::lighting::frame_uniform::FrameUniform;

/// Per-draw uniforms sit at this dynamic-offset stride (the largest uniform offset alignment
/// WebGPU guarantees).
pub const DRAW_STRIDE: u64 = 256;

/// `struct Frame` in `refraction.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct FrameBlock {
    pub view_proj: Mat4,
    /// xyz: camera position, engine space.
    pub camera: Vec4,
    /// xy: viewport in pixels, zw: its reciprocal.
    pub viewport: Vec4,
    /// near plane, reference depth, maximum depth gain, minimum view depth.
    pub depth: Vec4,
    /// maximum offset, soft fade distance, rejection bias, replacement offset.
    pub limits: Vec4,
    /// x: the view depth of a sky pixel.
    pub far: Vec4,
}

impl FrameBlock {
    pub fn new(frame: &FrameUniform, near_plane: f32) -> Self {
        Self {
            view_proj: frame.view_proj,
            camera: frame.camera_pos,
            viewport: frame.viewport,
            depth: Vec4::new(
                near_plane,
                REFERENCE_DEPTH_EU,
                MAX_DEPTH_GAIN,
                MIN_VIEW_DEPTH_EU,
            ),
            limits: Vec4::new(
                MAX_OFFSET_UV,
                SOFT_FADE_EU,
                REJECT_BIAS_EU,
                REPLACE_OFFSET_UV,
            ),
            far: Vec4::new(FAR_VIEW_DEPTH_EU, 0.0, 0.0, 0.0),
        }
    }
}

/// `struct Draw` in `refraction.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct DrawBlock {
    pub model: Mat4,
    /// Inverse transpose of `model`, for the card's normal.
    pub normal_matrix: Mat4,
    /// xy: offset, zw: scale of layer A.
    pub uv_a: Vec4,
    pub uv_b: Vec4,
    /// x: strength, y: 1 when the vertex alpha scales the distortion.
    pub params: Vec4,
}

impl DrawBlock {
    /// `None` for a singular model matrix, which has no normal matrix.
    pub fn new(
        model: Mat4,
        vertex_colors: bool,
        strength: f32,
        layers: LayerTransforms,
    ) -> Option<Self> {
        if model.determinant().abs() < f32::EPSILON {
            return None;
        }
        let pack = |uv: super::offset::UvTransform| {
            Vec4::new(uv.offset.x, uv.offset.y, uv.scale.x, uv.scale.y)
        };
        Some(Self {
            model,
            normal_matrix: model.inverse().transpose(),
            uv_a: pack(layers.a),
            uv_b: pack(layers.b),
            params: Vec4::new(strength, f32::from(u8::from(vertex_colors)), 0.0, 0.0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draw_block_fits_its_stride() {
        assert!(std::mem::size_of::<DrawBlock>() as u64 <= DRAW_STRIDE);
    }

    #[test]
    fn blocks_have_no_padding_the_shader_does_not_declare() {
        assert_eq!(std::mem::size_of::<FrameBlock>(), 64 + 5 * 16);
        assert_eq!(std::mem::size_of::<DrawBlock>(), 2 * 64 + 3 * 16);
    }
}
