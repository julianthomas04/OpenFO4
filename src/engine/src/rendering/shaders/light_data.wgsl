// Mirrors local_lights/cluster_grid.rs, the flag constants of placed_light.rs
// and the shadow-slot constants of shadows/lamps/table.rs.
const MAX_VISIBLE_LIGHTS = 4096u;
const MAX_LIGHTS_PER_CLUSTER = 128u;
const CLUSTER_SLICES = 32u;

const LIGHT_FLAG_OFF_BY_DEFAULT = 0x20u;
const LIGHT_FLAG_SHADOW_SPOT = 0x400u;
const LIGHT_FLAG_SHADOW_HEMISPHERE = 0x800u;
const LIGHT_FLAG_SHADOW_OMNI = 0x1000u;
const LIGHT_FLAG_SHADOW_TYPES = LIGHT_FLAG_SHADOW_SPOT | LIGHT_FLAG_SHADOW_HEMISPHERE | LIGHT_FLAG_SHADOW_OMNI;
const LIGHT_FLAG_SPOT_MASK = 0x4400u;
const LIGHT_FLAG_NON_SPECULAR = 0x8000u;
const LIGHT_FLAG_ATTENUATION_ONLY = 0x10000u;
const LIGHT_FLAG_IGNORE_ROUGHNESS = 0x40000u;
const LIGHT_FLAG_NO_RIM_LIGHTING = 0x80000u;
const LIGHT_FLAG_AMBIENT_ONLY = 0x100000u;
const LIGHT_FLAG_REF_NEVER_FADES = 0x2000000u;
const LIGHT_FLAG_REF_DOES_NOT_LIGHT_LANDSCAPE = 0x4000000u;

const MAX_SHADOW_SLOTS = 32u;
const SHADOW_SLOT_NONE = 0xffffffffu;

// A culled, faded light ready to shade. `color` is linear and final;
// `shadow_slot` indexes the lamp-shadow table or is SHADOW_SLOT_NONE.
struct VisibleLight {
    position: vec3<f32>,
    radius: f32,
    color: vec3<f32>,
    flags: u32,
    attenuation: vec3<f32>,
    spot_exponent: f32,
    direction: vec3<f32>,
    spot_cos_outer: f32,
    shadow_slot: u32,
};

// Mirrors local_lights/buffers.rs. `visible` counts every append, including
// those past MAX_VISIBLE_LIGHTS.
struct LightCounters {
    visible: atomic<u32>,
    overflowing_clusters: atomic<u32>,
};

fn is_spot_light(flags: u32) -> bool {
    return (flags & LIGHT_FLAG_SPOT_MASK) != 0u;
}

fn is_hemisphere_light(flags: u32) -> bool {
    return (flags & LIGHT_FLAG_SHADOW_HEMISPHERE) != 0u;
}

// Spot and hemisphere lights, and any light that has a lamp-shadow slot; every
// other light is a plain point light.
fn is_directional_or_shadowed(light: VisibleLight) -> bool {
    return is_spot_light(light.flags) || is_hemisphere_light(light.flags) || light.shadow_slot != SHADOW_SLOT_NONE;
}

const COS_45_DEGREES = 0.70710678;

// Smallest sphere around the lit volume: the range sphere, or for a spot the
// sphere around its cone (Wronski, "Cull that cone!").
fn light_bounding_sphere(position: vec3<f32>, radius: f32, flags: u32, direction: vec3<f32>, cos_half: f32) -> vec4<f32> {
    if (!is_spot_light(flags)) {
        return vec4<f32>(position, radius);
    }
    if (cos_half < COS_45_DEGREES) {
        let sin_half = sqrt(1.0 - cos_half * cos_half);
        return vec4<f32>(position + direction * (radius * cos_half), radius * sin_half);
    }
    let cone_radius = radius / (2.0 * cos_half);
    return vec4<f32>(position + direction * cone_radius, cone_radius);
}

// Clusters are tile-major, so one tile's slices are contiguous.
fn cluster_index(tile: vec2<u32>, slice: u32, tiles_x: u32) -> u32 {
    return (tile.y * tiles_x + tile.x) * CLUSTER_SLICES + slice;
}
