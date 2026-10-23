use bytemuck::{Pod, Zeroable};

/// One placed grass instance, in the GPU instance-buffer layout (48 bytes).
///
/// The blade frame is the model-to-world rotation of the
/// instance (`grass_placement::basis::blade_rotation`), given as the images of
/// the model's +X and +Y axes. The third column is
/// `cross(right, up)` (the image of the model's +Z axis; the frame is
/// orthonormal and right-handed), so a shader builds the rotation as
/// `mat3(right, up, cross(right, up))`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct GrassBlade {
    /// World position in engine units (Y-up).
    pub position: [f32; 3],
    /// Signed height-range factor `k`; the shader scales by `1 + k * mask`.
    pub height_k: f32,
    /// Unit image of the model's +X axis in engine space.
    pub right: [f32; 3],
    /// Brightness multiplier from the terrain vertex colour and the colour range.
    pub tint: f32,
    /// Unit image of the model's +Y (Bethesda Z, up) axis in engine space.
    pub up: [f32; 3],
    pad: f32,
}

impl GrassBlade {
    pub fn new(
        position: [f32; 3],
        height_k: f32,
        right: [f32; 3],
        tint: f32,
        up: [f32; 3],
    ) -> Self {
        Self {
            position,
            height_k,
            right,
            tint,
            up,
            pad: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrassFlags(pub u8);

impl GrassFlags {
    pub const VERTEX_LIGHTING: u8 = 0x1;
    pub const UNIFORM_SCALING: u8 = 0x2;
    pub const FIT_TO_SLOPE: u8 = 0x4;

    pub fn vertex_lighting(self) -> bool {
        self.0 & Self::VERTEX_LIGHTING != 0
    }

    pub fn uniform_scaling(self) -> bool {
        self.0 & Self::UNIFORM_SCALING != 0
    }

    pub fn fit_to_slope(self) -> bool {
        self.0 & Self::FIT_TO_SLOPE != 0
    }
}

/// All blades of one grass type placed in one exterior cell.
#[derive(Clone, Debug)]
pub struct CellGrass {
    /// GLB asset key as `glb_key` builds it from the record's model path:
    /// `meshes\<path>.glb`, original case below the root, backslash separators.
    pub model_path: String,
    pub flags: GrassFlags,
    pub wave_period: f32,
    pub blades: Vec<GrassBlade>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_blade_layout_is_48_bytes_with_the_documented_offsets() {
        assert_eq!(std::mem::size_of::<GrassBlade>(), 48);
        assert_eq!(std::mem::offset_of!(GrassBlade, position), 0);
        assert_eq!(std::mem::offset_of!(GrassBlade, height_k), 12);
        assert_eq!(std::mem::offset_of!(GrassBlade, right), 16);
        assert_eq!(std::mem::offset_of!(GrassBlade, tint), 28);
        assert_eq!(std::mem::offset_of!(GrassBlade, up), 32);
    }
}
