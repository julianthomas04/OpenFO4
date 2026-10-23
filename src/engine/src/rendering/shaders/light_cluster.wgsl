// Mirrors `LightClusterUniforms` in local_lights/light_cluster.rs.
struct LightClusterUniforms {
    view: mat4x4<f32>,
    // x: projection x scale, y: projection y scale.
    projection_scale: vec4<f32>,
    // x: first slice far depth, y: cluster far depth, z: log2 scale, w: log2 bias.
    cluster_z: vec4<f32>,
};

// One workgroup per screen tile (the dispatch is the tile grid). Threads
// compact the visible lights touching the tile 64 at a time; thread `s` then
// owns depth slice `s` and appends the batch's lights touching its cluster in
// ascending index order, so every list is sorted and deterministic.
const WORKGROUP_SIZE = 64u;
// The last slice is open-ended: it takes every fragment beyond the cluster
// far depth, which lights inside the fade distance can still reach.
const OPEN_SLICE_FAR_DEPTH = 1.0e6;
const SPHERE_DRAW_INSTANCE_COUNT = 1u;
const HEMISPHERE_CONE_COS = 0.0;

@group(0) @binding(0) var<storage, read> visible_lights: array<VisibleLight>;
@group(0) @binding(1) var<storage, read_write> light_counters: LightCounters;
@group(0) @binding(2) var<storage, read_write> cluster_light_counts: array<u32>;
@group(0) @binding(3) var<storage, read_write> cluster_light_indices: array<u32>;
@group(0) @binding(4) var<storage, read_write> sphere_draw_args: array<u32, 4>;
@group(0) @binding(5) var<uniform> light_cluster: LightClusterUniforms;

struct ViewLight {
    // Bounding sphere of the lit volume.
    bounds: vec4<f32>,
    // Position and range.
    apex: vec4<f32>,
    // Axis and cosine of the half-angle; meaningful only with has_cone.
    cone: vec4<f32>,
    has_cone: u32,
    index: u32,
};

struct Box {
    min: vec3<f32>,
    max: vec3<f32>,
};

var<workgroup> shared_light_count: u32;
var<workgroup> scan: array<u32, WORKGROUP_SIZE>;
var<workgroup> batch_size: u32;
var<workgroup> batch: array<ViewLight, WORKGROUP_SIZE>;

fn to_view(position: vec3<f32>) -> vec3<f32> {
    return (light_cluster.view * vec4<f32>(position, 1.0)).xyz;
}

// Everything in view space (camera at the origin looking down -Z).
fn view_light(index: u32) -> ViewLight {
    let light = visible_lights[index];
    let bounds = light_bounding_sphere(light.position, light.radius, light.flags, light.direction, light.spot_cos_outer);
    let axis = (light_cluster.view * vec4<f32>(light.direction, 0.0)).xyz;
    let is_spot = is_spot_light(light.flags);
    let cone_cos = select(HEMISPHERE_CONE_COS, light.spot_cos_outer, is_spot);
    return ViewLight(
        vec4<f32>(to_view(bounds.xyz), bounds.w),
        vec4<f32>(to_view(light.position), light.radius),
        vec4<f32>(axis, cone_cos),
        u32(is_spot || is_hemisphere_light(light.flags)),
        index,
    );
}

fn tile_ndc_min(tile: vec2<u32>, tiles: vec2<u32>) -> vec2<f32> {
    let step = 2.0 / vec2<f32>(tiles);
    return vec2<f32>(-1.0 + f32(tile.x) * step.x, 1.0 - f32(tile.y + 1u) * step.y);
}

fn tile_ndc_max(tile: vec2<u32>, tiles: vec2<u32>) -> vec2<f32> {
    let step = 2.0 / vec2<f32>(tiles);
    return vec2<f32>(-1.0 + f32(tile.x + 1u) * step.x, 1.0 - f32(tile.y) * step.y);
}

// The tile's four side planes through the eye; inside is dot(n, p) >= 0.
fn tile_planes(ndc_min: vec2<f32>, ndc_max: vec2<f32>) -> array<vec3<f32>, 4> {
    let scale = light_cluster.projection_scale.xy;
    return array<vec3<f32>, 4>(
        normalize(vec3<f32>(scale.x, 0.0, ndc_min.x)),
        normalize(vec3<f32>(-scale.x, 0.0, -ndc_max.x)),
        normalize(vec3<f32>(0.0, scale.y, ndc_min.y)),
        normalize(vec3<f32>(0.0, -scale.y, -ndc_max.y)),
    );
}

fn sphere_in_tile(sphere: vec4<f32>, planes: array<vec3<f32>, 4>) -> bool {
    if (sphere.z - sphere.w >= 0.0) {
        return false;
    }
    for (var i = 0u; i < 4u; i = i + 1u) {
        if (dot(planes[i], sphere.xyz) < -sphere.w) {
            return false;
        }
    }
    return true;
}

fn slice_depth_range(slice: u32) -> vec2<f32> {
    let first = light_cluster.cluster_z.x;
    let scale = light_cluster.cluster_z.z;
    let near = select(first * exp2(f32(slice) / scale - 1.0 / scale), 0.0, slice == 0u);
    let far = select(first * exp2(f32(slice) / scale), OPEN_SLICE_FAR_DEPTH, slice == CLUSTER_SLICES - 1u);
    return vec2<f32>(near, far);
}

// View-space box around the tile's frustum segment between two depths.
fn cluster_box(ndc_min: vec2<f32>, ndc_max: vec2<f32>, depths: vec2<f32>) -> Box {
    let scale = light_cluster.projection_scale.xy;
    let lo = min(ndc_min * depths.x, ndc_min * depths.y) / scale;
    let hi = max(ndc_max * depths.x, ndc_max * depths.y) / scale;
    return Box(vec3<f32>(lo, -depths.y), vec3<f32>(hi, -depths.x));
}

fn sphere_touches_box(sphere: vec4<f32>, box: Box) -> bool {
    let offset = sphere.xyz - clamp(sphere.xyz, box.min, box.max);
    return dot(offset, offset) <= sphere.w * sphere.w;
}

// Conservative cone test against the box's bounding sphere (Wronski, "Cull
// that cone!"): the cone misses when the sphere lies outside its angle, past
// its range or behind its apex.
fn cone_touches_box(light: ViewLight, box: Box) -> bool {
    let center = (box.min + box.max) * 0.5;
    let radius = length(box.max - box.min) * 0.5;
    let to_center = center - light.apex.xyz;
    let along_axis = dot(to_center, light.cone.xyz);
    let across_axis = sqrt(max(dot(to_center, to_center) - along_axis * along_axis, 0.0));
    let cos_half = light.cone.w;
    let sin_half = sqrt(1.0 - cos_half * cos_half);
    let outside_angle = cos_half * across_axis - along_axis * sin_half > radius;
    let past_range = along_axis > radius + light.apex.w;
    let behind_apex = along_axis < -radius;
    return !(outside_angle || past_range || behind_apex);
}

fn light_touches_cluster(light: ViewLight, box: Box) -> bool {
    return sphere_touches_box(light.bounds, box) && (light.has_cone == 0u || cone_touches_box(light, box));
}

// Inclusive Hillis-Steele prefix sum of `scan` across the workgroup.
fn prefix_sum(lid: u32) {
    for (var offset = 1u; offset < WORKGROUP_SIZE; offset = offset << 1u) {
        let has_left = lid >= offset;
        let left = scan[select(0u, lid - offset, has_left)];
        workgroupBarrier();
        scan[lid] += select(0u, left, has_left);
        workgroupBarrier();
    }
}

@compute @workgroup_size(64)
fn light_cluster_main(
    @builtin(workgroup_id) workgroup: vec3<u32>,
    @builtin(num_workgroups) workgroups: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    let tile = workgroup.xy;
    let tiles = workgroups.xy;
    if (lid == 0u) {
        shared_light_count = min(atomicLoad(&light_counters.visible), MAX_VISIBLE_LIGHTS);
    }
    let light_count = workgroupUniformLoad(&shared_light_count);
    if (lid == 0u && all(tile == vec2<u32>(0u))) {
        sphere_draw_args[SPHERE_DRAW_INSTANCE_COUNT] = light_count;
    }

    let ndc_min = tile_ndc_min(tile, tiles);
    let ndc_max = tile_ndc_max(tile, tiles);
    let planes = tile_planes(ndc_min, ndc_max);
    let owns_slice = lid < CLUSTER_SLICES;
    let slice = min(lid, CLUSTER_SLICES - 1u);
    let box = cluster_box(ndc_min, ndc_max, slice_depth_range(slice));
    let list_start = cluster_index(tile, slice, tiles.x) * MAX_LIGHTS_PER_CLUSTER;
    var listed = 0u;
    var overflowed = false;

    for (var batch_start = 0u; batch_start < light_count; batch_start = batch_start + WORKGROUP_SIZE) {
        let index = batch_start + lid;
        var light: ViewLight;
        var in_tile = false;
        if (index < light_count) {
            light = view_light(index);
            in_tile = sphere_in_tile(light.bounds, planes);
        }
        scan[lid] = u32(in_tile);
        workgroupBarrier();
        prefix_sum(lid);
        if (in_tile) {
            batch[scan[lid] - 1u] = light;
        }
        if (lid == WORKGROUP_SIZE - 1u) {
            batch_size = scan[lid];
        }
        let compacted = workgroupUniformLoad(&batch_size);

        if (owns_slice) {
            for (var i = 0u; i < compacted; i = i + 1u) {
                if (!light_touches_cluster(batch[i], box)) {
                    continue;
                }
                if (listed == MAX_LIGHTS_PER_CLUSTER) {
                    overflowed = true;
                    break;
                }
                cluster_light_indices[list_start + listed] = batch[i].index;
                listed = listed + 1u;
            }
        }
        workgroupBarrier();
    }

    if (owns_slice) {
        cluster_light_counts[cluster_index(tile, slice, tiles.x)] = listed;
        if (overflowed) {
            atomicAdd(&light_counters.overflowing_clusters, 1u);
        }
    }
}
