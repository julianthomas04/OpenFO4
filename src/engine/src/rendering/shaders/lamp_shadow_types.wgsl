// The lamp-shadow table as the GPU reads it: the world binds it in the frame group
// (frame.wgsl), the loading art in a group of its own. Needs light_data.wgsl.

// Mirrors `GpuShadowLamp` in shadows/lamps/table.rs.
struct ShadowLamp {
    faces: array<mat4x4<f32>, 6>,
    // Per face: atlas uv origin (xy) and side (z); z = 0 without a tile.
    tiles: array<vec4<f32>, 6>,
    // The lamp basis: forward, up, side.
    axes: array<vec4<f32>, 3>,
    // xyz: position, w: near clip.
    position: vec4<f32>,
    // x: strength, y: world texel size per unit of face-axis distance,
    // z: 1 for cube storage, 0 for one spot tile.
    params: vec4<f32>,
};

// Mirrors `GpuShadowLampTable` in shadows/lamps/table.rs.
struct ShadowLampTable {
    lamps: array<ShadowLamp, MAX_SHADOW_SLOTS>,
    // Constant, slope and normal-offset bias in texels, w: squared receiver
    // fade distance.
    bias: vec4<f32>,
    // x: PCF taps per axis.
    filter_taps: vec4<u32>,
};
