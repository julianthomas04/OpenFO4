#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum EffectBlend {
    Alpha,
    Additive,
    Multiply,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Falloff {
    /// dot(n,v) facing-ratio threshold in [-1, 1], as authored.
    pub start_facing: f32,
    pub stop_facing: f32,
    pub start_opacity: f32,
    pub stop_opacity: f32,
    pub scales_color: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub path: String,
    pub recolours_color: bool,
    pub drives_alpha: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EffectParams {
    pub blend: EffectBlend,
    pub base_color_srgb: [u8; 3],
    pub base_color_scale: f32,
    pub falloff: Option<Falloff>,
    /// `None` = unlit effect.
    pub lighting_influence: Option<f32>,
    pub palette: Option<Palette>,
}

/// Hashable identity of an [`EffectParams`]: floats compare by bit pattern.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EffectKey {
    blend: EffectBlend,
    base_color_srgb: [u8; 3],
    base_color_scale: u32,
    falloff: Option<([u32; 4], bool)>,
    lighting_influence: Option<u32>,
    palette: Option<(String, bool, bool)>,
}

impl EffectParams {
    pub fn key(&self) -> EffectKey {
        EffectKey {
            blend: self.blend,
            base_color_srgb: self.base_color_srgb,
            base_color_scale: self.base_color_scale.to_bits(),
            falloff: self.falloff.map(|falloff| {
                (
                    [
                        falloff.start_facing.to_bits(),
                        falloff.stop_facing.to_bits(),
                        falloff.start_opacity.to_bits(),
                        falloff.stop_opacity.to_bits(),
                    ],
                    falloff.scales_color,
                )
            }),
            lighting_influence: self.lighting_influence.map(f32::to_bits),
            palette: self.palette.as_ref().map(|palette| {
                (
                    palette.path.clone(),
                    palette.recolours_color,
                    palette.drives_alpha,
                )
            }),
        }
    }
}

impl EffectParams {
    /// Effect shape whose name carries no effect tokens: lit diffuse times alpha.
    pub const LEGACY: Self = Self {
        blend: EffectBlend::Alpha,
        base_color_srgb: [255, 255, 255],
        base_color_scale: 1.0,
        falloff: None,
        lighting_influence: Some(1.0),
        palette: None,
    };
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnvParams {
    /// Path below the textures directory, as `CubeArray::acquire` takes it.
    pub cube_path: String,
    /// Texture cache key (`textures\...`).
    pub mask_path: Option<String>,
    pub mask_scale: f32,
}

/// Hashable identity of an [`EnvParams`]: floats compare by bit pattern.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EnvKey {
    cube_path: String,
    mask_path: Option<String>,
    mask_scale: u32,
}

impl EnvParams {
    pub fn key(&self) -> EnvKey {
        EnvKey {
            cube_path: self.cube_path.clone(),
            mask_path: self.mask_path.clone(),
            mask_scale: self.mask_scale.to_bits(),
        }
    }
}
