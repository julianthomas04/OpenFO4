struct FrameUniform {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    to_sun: vec4<f32>,
    sun_illuminance: vec4<f32>,
    sun_disc_illuminance: vec4<f32>,
    // xyz: unit direction towards the sun disc (the sun's own place; to_sun follows the moon at
    // night), w: the sun's alpha around sunrise and sunset.
    sun_disc: vec4<f32>,
    // Linear ambient colour of each cube face, order +X, -X, +Y, -Y, +Z, -Z (engine axes); w unused.
    ambient_cube: array<vec4<f32>, 6>,
    sky_zenith: vec4<f32>,
    sky_horizon: vec4<f32>,
    sky_ground: vec4<f32>,
    fog_near_color: vec4<f32>,
    fog_far_color: vec4<f32>,
    fog_params: vec4<f32>,
    cascade_view_proj: array<mat4x4<f32>, 4>,
    cascade_spheres: array<vec4<f32>, 4>,
    shadow_params: vec4<f32>,
    cascade_texel_world: vec4<f32>,
    shadow_filter: vec4<u32>,
    flags: vec4<u32>,
    camera_forward: vec4<f32>,
    // xy: viewport size in pixels, zw: its reciprocal.
    viewport: vec4<f32>,
    // xy: cluster tiles, z: depth slices, w: 1 while local lights shade.
    cluster_grid: vec4<u32>,
    // x: first slice far depth, y: cluster far depth, z: log2 scale, w: log2 bias.
    cluster_z: vec4<f32>,
    // x: tonemap operator, y: white point, z: saturation.
    tonemap: vec4<f32>,
    // x: texture feedback on, y: frame index, z: pixel sample mask,
    // w: iTexMipMapSkip.
    texture_feedback: vec4<u32>,
    // x: seconds since start wrapped at 4096, y: water surface height at the
    // camera, z: 1 while the camera is underwater, w: the share
    // of the near plane's height under the surface (0 to 1).
    time_water: vec4<f32>,
};

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

@group(1) @binding(0) var<uniform> frame: FrameUniform;
@group(1) @binding(1) var sun_shadow_maps: texture_depth_2d_array;
@group(1) @binding(2) var shadow_comparison_sampler: sampler_comparison;
@group(1) @binding(3) var<storage, read> visible_lights: array<VisibleLight>;
@group(1) @binding(4) var<storage, read> cluster_light_counts: array<u32>;
@group(1) @binding(5) var<storage, read> cluster_light_indices: array<u32>;
@group(1) @binding(6) var lamp_shadow_atlas: texture_depth_2d;
@group(1) @binding(7) var<uniform> lamp_shadows: ShadowLampTable;

fn debug_view() -> u32 {
    return frame.flags.z;
}

fn local_lights_enabled() -> bool {
    return frame.cluster_grid.w != 0u;
}
