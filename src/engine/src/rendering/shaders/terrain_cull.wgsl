struct CullUniforms {
    frustum_planes: array<vec4<f32>, 5>,
    total_instances: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0) var<storage, read> instances: array<TerrainInstance>;
@group(0) @binding(1) var<storage, read_write> draw_commands: array<DrawIndexedIndirect>;
@group(0) @binding(2) var<storage, read_write> draw_count: atomic<u32>;
@group(0) @binding(3) var<uniform> cull: CullUniforms;

fn frustum_test(aabb_min: vec3<f32>, aabb_max: vec3<f32>) -> bool {
    for (var i = 0u; i < 5u; i = i + 1u) {
        let plane = cull.frustum_planes[i];
        let n = plane.xyz;
        let d = plane.w;

        let p = vec3<f32>(
            select(aabb_min.x, aabb_max.x, n.x >= 0.0),
            select(aabb_min.y, aabb_max.y, n.y >= 0.0),
            select(aabb_min.z, aabb_max.z, n.z >= 0.0),
        );

        if (dot(n, p) + d < 0.0) {
            return false;
        }
    }
    return true;
}

@compute @workgroup_size(64)
fn terrain_cull_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let idx = gid.x;
    if (idx >= cull.total_instances) {
        return;
    }

    let inst = instances[idx];

    if (inst.index_count == 0u) {
        return;
    }

    if (!frustum_test(inst.aabb_min, inst.aabb_max)) {
        return;
    }

    var cmd: DrawIndexedIndirect;
    cmd.index_count = inst.index_count;
    cmd.instance_count = 1u;
    cmd.first_index = inst.index_offset;
    cmd.base_vertex = i32(inst.vertex_offset);
    cmd.first_instance = idx;

    let slot = atomicAdd(&draw_count, 1u);
    draw_commands[slot] = cmd;
}
