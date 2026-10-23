struct Instance {
    transform: mat4x4<f32>,
    aabb_min: vec3<f32>,
    _pad0: f32,
    aabb_max: vec3<f32>,
    _pad1: f32,
    index_offset: u32,
    index_count: u32,
    vertex_offset: u32,
    material_id: u32,
    alpha_cutoff: f32,
    alpha_mult: f32,
    flags: u32,
    _pad2: u32,
};

struct Material {
    diffuse_index: u32,
    normal_index: u32,
    specular_index: u32,
    glow_index: u32,
    specular_color: u32,
    specular_scale: f32,
    smoothness: f32,
    fresnel_power: f32,
    emissive: vec4<f32>,
    effect_color: vec4<f32>,
    falloff: vec4<f32>,
    env_cube_index: u32,
    env_mask_index: u32,
    env_scale: f32,
    effect_flags: u32,
    palette_index: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

const EFFECT_FLAG_EFFECT: u32 = 1u;
const EFFECT_FLAG_ADDITIVE: u32 = 2u;
const EFFECT_FLAG_MULTIPLY: u32 = 4u;
const EFFECT_FLAG_FALLOFF: u32 = 8u;
const EFFECT_FLAG_FALLOFF_COLOR: u32 = 16u;
const EFFECT_FLAG_PALETTE_COLOR: u32 = 32u;
const EFFECT_FLAG_PALETTE_ALPHA: u32 = 64u;
const EFFECT_FLAG_ENV: u32 = 128u;

struct TerrainInstance {
    aabb_min: vec3<f32>,
    _pad0: f32,
    aabb_max: vec3<f32>,
    _pad1: f32,
    index_offset: u32,
    index_count: u32,
    vertex_offset: u32,
    splat_layer: u32,
    layer_indices: vec4<u32>,
    flags: vec4<u32>,
};
