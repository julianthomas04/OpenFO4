// Mirrors `MAX_SHADOW_VIEWS` and `ViewKind` in `shadows/views.rs` and
// `CasterKind` in `shadows/shadow_cull.rs`, which asserts the two sizes below.
const MAX_SHADOW_VIEWS = 16u;
const VIEW_UNUSED = 0u;
const VIEW_ORTHO = 1u;
const VIEW_PERSPECTIVE = 2u;
const KIND_OPAQUE = 0u;
const KIND_ALPHA_TEST = 1u;
const KIND_TERRAIN = 2u;
// Mirrors `VIEW_FLAG_SKIP_ALPHA` in `shadows/views.rs`.
const VIEW_FLAG_SKIP_ALPHA = 1u;

// Mirror `INSTANCE_FLAGS_*` in `gpu_driven.rs`.
const INSTANCE_FLAGS_BLEND = 1u;
const INSTANCE_FLAGS_MARKER = 4u;
const INSTANCE_FLAGS_EFFECT = 8u;
const INSTANCE_FLAGS_DECAL = 16u;
const INSTANCE_FLAGS_WATER = 32u;
const INSTANCE_FLAGS_HIDDEN_FX = 128u;

// Blend, marker, effect, decal, water and hidden-FX instances never cast shadows.
const NON_CASTER_FLAGS = INSTANCE_FLAGS_BLEND | INSTANCE_FLAGS_MARKER | INSTANCE_FLAGS_EFFECT | INSTANCE_FLAGS_DECAL | INSTANCE_FLAGS_WATER | INSTANCE_FLAGS_HIDDEN_FX;

// Mirror `DEBUG_*` in `shadow_cull.rs`.
const DEBUG_ONE_TRIANGLE = 1u;
const DEBUG_SKIP_ALPHA = 2u;

struct ViewCull {
    planes: array<vec4<f32>, 6>,
    // Ortho: cascade sphere. Perspective: light position and range.
    bound: vec4<f32>,
    // Ortho: direction to the light.
    axis: vec4<f32>,
    // x: texel size (ortho) or texel size per unit distance (perspective),
    // y: perspective near plane distance, z: minimum caster size in texels.
    metric: vec4<f32>,
    // x: VIEW_* kind, y: VIEW_FLAG_* bits.
    kind: vec4<u32>,
};

struct ShadowCullUniforms {
    views: array<ViewCull, MAX_SHADOW_VIEWS>,
    object_count: u32,
    terrain_count: u32,
    // Number of views in use, and their slots packed four per element.
    active_count: u32,
    // Slots before this one are cascades; the rest are lamp views.
    cascade_slots: u32,
    // Per caster kind, the draw capacity of one cascade and one lamp view.
    cascade_capacities: vec4<u32>,
    lamp_capacities: vec4<u32>,
    // x: DEBUG_* bits.
    debug: vec4<u32>,
    active_slots: array<vec4<u32>, 4>,
};

// 6 storage buffers per stage: both entry points share one layout.
@group(0) @binding(0) var<storage, read> instances: array<Instance>;
@group(0) @binding(1) var<storage, read> terrain_instances: array<TerrainInstance>;
@group(0) @binding(2) var<storage, read_write> opaque_draws: array<DrawIndexedIndirect>;
@group(0) @binding(3) var<storage, read_write> alpha_test_draws: array<DrawIndexedIndirect>;
@group(0) @binding(4) var<storage, read_write> terrain_draws: array<DrawIndexedIndirect>;
@group(0) @binding(5) var<storage, read_write> draw_counts: array<atomic<u32>>;
@group(0) @binding(6) var<uniform> shadow_cull: ShadowCullUniforms;

fn inside_planes(slot: u32, aabb_min: vec3<f32>, aabb_max: vec3<f32>) -> bool {
    for (var i = 0u; i < 6u; i = i + 1u) {
        let plane = shadow_cull.views[slot].planes[i];
        let farthest = select(aabb_min, aabb_max, plane.xyz >= vec3<f32>(0.0));
        if (dot(plane.xyz, farthest) + plane.w < 0.0) {
            return false;
        }
    }
    return true;
}

// Sphere against the cascade's light-aligned cylinder, then the AABB against
// the ortho box, then casters too small to cover the view's minimum texels.
fn casts_into_ortho(slot: u32, center: vec3<f32>, radius: f32, aabb_min: vec3<f32>, aabb_max: vec3<f32>) -> bool {
    let offset = center - shadow_cull.views[slot].bound.xyz;
    let axis = shadow_cull.views[slot].axis.xyz;
    let across_light = offset - axis * dot(offset, axis);
    if (length(across_light) > shadow_cull.views[slot].bound.w + radius) {
        return false;
    }
    if (2.0 * radius < shadow_cull.views[slot].metric.z * shadow_cull.views[slot].metric.x) {
        return false;
    }
    return inside_planes(slot, aabb_min, aabb_max);
}

// Sphere against the light's range, then the AABB against the view frustum,
// then casters too small to cover the view's minimum texels where they stand.
fn casts_into_perspective(slot: u32, center: vec3<f32>, radius: f32, aabb_min: vec3<f32>, aabb_max: vec3<f32>) -> bool {
    let light_distance = distance(center, shadow_cull.views[slot].bound.xyz);
    if (light_distance > shadow_cull.views[slot].bound.w + radius) {
        return false;
    }
    let nearest = max(light_distance - radius, shadow_cull.views[slot].metric.y);
    if (2.0 * radius < shadow_cull.views[slot].metric.z * shadow_cull.views[slot].metric.x * nearest) {
        return false;
    }
    return inside_planes(slot, aabb_min, aabb_max);
}

fn casts_into(kind: u32, slot: u32, center: vec3<f32>, radius: f32, aabb_min: vec3<f32>, aabb_max: vec3<f32>) -> bool {
    if (kind == KIND_ALPHA_TEST && (shadow_cull.views[slot].kind.y & VIEW_FLAG_SKIP_ALPHA) != 0u) {
        return false;
    }
    switch shadow_cull.views[slot].kind.x {
        case VIEW_ORTHO: { return casts_into_ortho(slot, center, radius, aabb_min, aabb_max); }
        case VIEW_PERSPECTIVE: { return casts_into_perspective(slot, center, radius, aabb_min, aabb_max); }
        default: { return false; }
    }
}

// Mirrors `ShadowDraws::draw_offset`: the cascade regions, then the lamp
// regions.
fn region_base(kind: u32, slot: u32) -> u32 {
    let cascade_capacity = shadow_cull.cascade_capacities[kind];
    if (slot < shadow_cull.cascade_slots) {
        return slot * cascade_capacity;
    }
    let lamp_slot = slot - shadow_cull.cascade_slots;
    return shadow_cull.cascade_slots * cascade_capacity + lamp_slot * shadow_cull.lamp_capacities[kind];
}

fn region_capacity(kind: u32, slot: u32) -> u32 {
    return select(shadow_cull.lamp_capacities[kind], shadow_cull.cascade_capacities[kind], slot < shadow_cull.cascade_slots);
}

// Per-workgroup draw counters per (kind, view), kind-major like `draw_counts`.
// A first pass counts the hits, one global atomicAdd per used counter reserves
// the workgroup's range, and a second pass hands each hit its slot in it.
const COUNTER_COUNT = 48u; // KIND_COUNT * MAX_SHADOW_VIEWS
var<workgroup> local_counts: array<atomic<u32>, COUNTER_COUNT>;
var<workgroup> local_cursors: array<atomic<u32>, COUNTER_COUNT>;
var<workgroup> region_starts: array<u32, COUNTER_COUNT>;

fn active_slot(i: u32) -> u32 {
    return shadow_cull.active_slots[i / 4u][i % 4u];
}

fn write_draw(kind: u32, index: u32, cmd: DrawIndexedIndirect) {
    switch kind {
        case KIND_OPAQUE: { opaque_draws[index] = cmd; }
        case KIND_ALPHA_TEST: { alpha_test_draws[index] = cmd; }
        default: { terrain_draws[index] = cmd; }
    }
}

fn draw_command(index_count: u32, index_offset: u32, vertex_offset: u32, instance: u32) -> DrawIndexedIndirect {
    var cmd: DrawIndexedIndirect;
    cmd.index_count = select(index_count, 3u, (shadow_cull.debug.x & DEBUG_ONE_TRIANGLE) != 0u);
    cmd.instance_count = 1u;
    cmd.first_index = index_offset;
    cmd.base_vertex = i32(vertex_offset);
    cmd.first_instance = instance;
    return cmd;
}

// Must run in uniform control flow by every invocation of the workgroup:
// `valid` false means the thread has no caster but still joins the barriers.
// The global counts keep counting past the capacity; writes past it are
// dropped and the draw call is capped at the capacity too.
fn append_to_views(
    local_index: u32,
    valid: bool,
    kind: u32,
    aabb_min: vec3<f32>,
    aabb_max: vec3<f32>,
    cmd: DrawIndexedIndirect,
) {
    let center = (aabb_min + aabb_max) * 0.5;
    let radius = length(aabb_max - aabb_min) * 0.5;

    var hits = 0u;
    if (valid) {
        for (var i = 0u; i < shadow_cull.active_count; i = i + 1u) {
            let slot = active_slot(i);
            if (casts_into(kind, slot, center, radius, aabb_min, aabb_max)) {
                hits = hits | (1u << slot);
                atomicAdd(&local_counts[kind * MAX_SHADOW_VIEWS + slot], 1u);
            }
        }
    }
    workgroupBarrier();

    if (local_index < COUNTER_COUNT) {
        let count = atomicLoad(&local_counts[local_index]);
        if (count > 0u) {
            region_starts[local_index] = atomicAdd(&draw_counts[local_index], count);
        }
    }
    workgroupBarrier();

    for (var i = 0u; i < shadow_cull.active_count; i = i + 1u) {
        let slot = active_slot(i);
        if ((hits & (1u << slot)) != 0u) {
            let counter = kind * MAX_SHADOW_VIEWS + slot;
            let draw = region_starts[counter] + atomicAdd(&local_cursors[counter], 1u);
            if (draw < region_capacity(kind, slot)) {
                write_draw(kind, region_base(kind, slot) + draw, cmd);
            }
        }
    }
}

@compute @workgroup_size(256)
fn shadow_cull_objects(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_index) local_index: u32,
) {
    let idx = gid.x;
    var valid = idx < shadow_cull.object_count;
    var inst: Instance;
    if (valid) {
        inst = instances[idx];
        valid = inst.index_count != 0u && (inst.flags & NON_CASTER_FLAGS) == 0u;
    }
    let kind = select(KIND_OPAQUE, KIND_ALPHA_TEST, inst.alpha_cutoff > 0.0);
    valid = valid && !(kind == KIND_ALPHA_TEST && (shadow_cull.debug.x & DEBUG_SKIP_ALPHA) != 0u);
    let cmd = draw_command(inst.index_count, inst.index_offset, inst.vertex_offset, idx);
    append_to_views(local_index, valid, kind, inst.aabb_min, inst.aabb_max, cmd);
}

@compute @workgroup_size(64)
fn shadow_cull_terrain(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_index) local_index: u32,
) {
    let idx = gid.x;
    var valid = idx < shadow_cull.terrain_count;
    var inst: TerrainInstance;
    if (valid) {
        inst = terrain_instances[idx];
        valid = inst.index_count != 0u;
    }
    let cmd = draw_command(inst.index_count, inst.index_offset, inst.vertex_offset, idx);
    append_to_views(local_index, valid, KIND_TERRAIN, inst.aabb_min, inst.aabb_max, cmd);
}
