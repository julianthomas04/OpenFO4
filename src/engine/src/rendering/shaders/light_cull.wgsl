// Mirrors `GpuPlacedLight` in placed_light.rs; a zeroed slot is a slab hole.
struct PlacedLight {
    position: vec3<f32>,
    radius: f32,
    color: vec3<f32>,
    flags: u32,
    attenuation: vec3<f32>,
    spot_exponent: f32,
    direction: vec3<f32>,
    spot_cos_outer: f32,
    flicker: vec4<f32>,
};

// Mirrors `LightCullUniforms` in local_lights/light_cull.rs.
struct LightCullUniforms {
    view_proj: mat4x4<f32>,
    frustum_planes: array<vec4<f32>, 5>,
    camera_pos: vec4<f32>,
    screen_size: vec2<f32>,
    hiz_mip_count: u32,
    hiz_valid: u32,
    slot_count: u32,
    fade_start: f32,
    fade_range: f32,
    intensity: f32,
    shadow_lamp_count: u32,
    // x: placed-light slab slot, y: lamp-shadow table slot.
    shadow_lamps: array<vec4<u32>, MAX_SHADOW_SLOTS>,
};

const SKIPPED_FLAGS = LIGHT_FLAG_OFF_BY_DEFAULT | LIGHT_FLAG_AMBIENT_ONLY;
const MIN_FADE_RANGE = 1e-4;

@group(0) @binding(0) var<storage, read> placed_lights: array<PlacedLight>;
@group(0) @binding(1) var<storage, read_write> visible_lights: array<VisibleLight>;
@group(0) @binding(2) var<storage, read_write> light_counters: LightCounters;
@group(0) @binding(3) var<uniform> light_cull: LightCullUniforms;
@group(0) @binding(4) var hiz: texture_2d<f32>;

fn distance_fade(flags: u32, camera_distance: f32) -> f32 {
    if ((flags & LIGHT_FLAG_REF_NEVER_FADES) != 0u) {
        return 1.0;
    }
    let t = saturate((camera_distance - light_cull.fade_start) / max(light_cull.fade_range, MIN_FADE_RANGE));
    return 1.0 - t * t * (3.0 - 2.0 * t);
}

fn shadow_slot(slab_slot: u32, flags: u32) -> u32 {
    if ((flags & LIGHT_FLAG_SHADOW_TYPES) == 0u) {
        return SHADOW_SLOT_NONE;
    }
    for (var i = 0u; i < light_cull.shadow_lamp_count; i = i + 1u) {
        if (light_cull.shadow_lamps[i].x == slab_slot) {
            return light_cull.shadow_lamps[i].y;
        }
    }
    return SHADOW_SLOT_NONE;
}

fn sphere_in_frustum(sphere: vec4<f32>) -> bool {
    for (var i = 0u; i < 5u; i = i + 1u) {
        let plane = light_cull.frustum_planes[i];
        if (dot(plane.xyz, sphere.xyz) + plane.w < -sphere.w) {
            return false;
        }
    }
    return true;
}

@compute @workgroup_size(64)
fn light_cull_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let slot = gid.x;
    if (slot >= light_cull.slot_count) {
        return;
    }
    let light = placed_lights[slot];
    if (light.radius <= 0.0 || (light.flags & SKIPPED_FLAGS) != 0u) {
        return;
    }

    let camera_distance = distance(light_cull.camera_pos.xyz, light.position);
    let fade = distance_fade(light.flags, camera_distance);
    if (fade <= 0.0) {
        return;
    }

    let sphere = light_bounding_sphere(light.position, light.radius, light.flags, light.direction, light.spot_cos_outer);
    if (!sphere_in_frustum(sphere)) {
        return;
    }
    let camera_inside = camera_distance < light.radius;
    if (!camera_inside && light_cull.hiz_valid != 0u
        && hiz_occluded(sphere.xyz - sphere.w, sphere.xyz + sphere.w, light_cull.view_proj, light_cull.screen_size, light_cull.hiz_mip_count)) {
        return;
    }

    let index = atomicAdd(&light_counters.visible, 1u);
    if (index >= MAX_VISIBLE_LIGHTS) {
        return;
    }
    visible_lights[index] = VisibleLight(
        light.position,
        light.radius,
        light.color * (fade * light_cull.intensity),
        light.flags,
        light.attenuation,
        light.spot_exponent,
        light.direction,
        light.spot_cos_outer,
        shadow_slot(slot, light.flags),
    );
}
