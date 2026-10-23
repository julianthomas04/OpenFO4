struct CullUniforms {
    frustum_planes: array<vec4<f32>, 5>,
    total_instances: u32,
    show_markers: u32,
    decal_bias_enabled: u32,
    show_fx: u32,
    view_proj: mat4x4<f32>,
    screen_size: vec2<f32>,
    hiz_mip_count: u32,
    hiz_valid: u32,
};

// NOTE: 1 instances + 12 draws + 1 counts = 14 storage buffers, past wgpu's
// conservative default of 8. device.rs requests exactly DRAW_BUCKET_COUNT + 2 and
// verifies the adapter supports it. Binding order MUST match gpu_driven.rs bucket
// constants and gpu_cull.rs's bind-group layout (draw_buffers[i] -> binding i+1).
@group(0) @binding(0) var<storage, read> instances: array<Instance>;
@group(0) @binding(1) var<storage, read_write> draws_opaque_culled: array<DrawIndexedIndirect>;
@group(0) @binding(2) var<storage, read_write> draws_opaque_twosided: array<DrawIndexedIndirect>;
@group(0) @binding(3) var<storage, read_write> draws_alphatest_culled: array<DrawIndexedIndirect>;
@group(0) @binding(4) var<storage, read_write> draws_alphatest_twosided: array<DrawIndexedIndirect>;
@group(0) @binding(5) var<storage, read_write> draws_decal_alphatest_culled: array<DrawIndexedIndirect>;
@group(0) @binding(6) var<storage, read_write> draws_decal_alphatest_twosided: array<DrawIndexedIndirect>;
@group(0) @binding(7) var<storage, read_write> draws_blend_culled: array<DrawIndexedIndirect>;
@group(0) @binding(8) var<storage, read_write> draws_blend_twosided: array<DrawIndexedIndirect>;
@group(0) @binding(9) var<storage, read_write> draws_decal_blend_culled: array<DrawIndexedIndirect>;
@group(0) @binding(10) var<storage, read_write> draws_decal_blend_twosided: array<DrawIndexedIndirect>;
@group(0) @binding(11) var<storage, read_write> draws_multiply_culled: array<DrawIndexedIndirect>;
@group(0) @binding(12) var<storage, read_write> draws_multiply_twosided: array<DrawIndexedIndirect>;
@group(0) @binding(13) var<storage, read_write> draw_counts: array<atomic<u32>>;
@group(0) @binding(14) var<uniform> cull: CullUniforms;
// Previous frame's depth pyramid (R32Float, min-reduced = farthest under
// reverse-Z). Texture binding — doesn't count against the storage-buffer limit.
@group(0) @binding(15) var hiz: texture_2d<f32>;

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

@compute @workgroup_size(256)
fn cull_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let idx = gid.x;
    if (idx >= cull.total_instances) {
        return;
    }

    let inst = instances[idx];

    if (inst.index_count == 0u) {
        return;
    }

    // Water surfaces (flag bit 32): drawn by the water stage, never by a bucket.
    if ((inst.flags & 32u) != 0u) {
        return;
    }

    // Engine markers (flag bit 4): hidden unless the debug toggle is on.
    if ((inst.flags & 4u) != 0u && cull.show_markers == 0u) {
        return;
    }

    // Hidden FX (flag bit 128): shown only while the debug toggle is on.
    if ((inst.flags & 128u) != 0u && cull.show_fx == 0u) {
        return;
    }

    if (!frustum_test(inst.aabb_min, inst.aabb_max)) {
        return;
    }

    if (cull.hiz_valid != 0u && hiz_occluded(inst.aabb_min, inst.aabb_max, cull.view_proj, cull.screen_size, cull.hiz_mip_count)) {
        return;
    }

    var cmd: DrawIndexedIndirect;
    cmd.index_count = inst.index_count;
    cmd.instance_count = 1u;
    cmd.first_index = inst.index_offset;
    cmd.base_vertex = i32(inst.vertex_offset);
    cmd.first_instance = idx;

    // Bucket order (must match gpu_driven.rs bucket constants and
    // ObjectPipelines::pipelines_in_bucket_order):
    //   0 opaque,        1 opaque_2s,        2 alphatest,       3 alphatest_2s,
    //   4 decal_alphat., 5 decal_alphat_2s,  6 blend,           7 blend_2s,
    //   8 decal_blend,   9 decal_blend_2s,
    //   10 multiply,     11 multiply_2s.
    // Alpha-tested geometry gets its own buckets so the opaque pipelines can use
    // a discard-free fragment shader (keeps early depth-write enabled). Decal-flagged
    // geometry routes to the decal buckets, whose pipelines carry a forward depth
    // bias that kills z-fighting against the coplanar surface the decal sits on. A
    // non-blend decal (alpha-tested OR opaque) routes to the biased alpha-test decal
    // bucket: fs_alphatest with alpha_cutoff 0 discards nothing, so an opaque decal
    // renders identically but still gains the bias — no separate opaque-decal bucket.
    // Multiplicative blend (flag bit 64, set with blend) routes to the multiply
    // buckets and ignores the decal bias. Within blend buckets, atomicAdd emission order is nondeterministic frame to
    // frame and there is no back-to-front sort — overlapping transparents may
    // shimmer. Known limitation; the fix is a depth-keyed sort of these buckets.
    let is_blend = (inst.flags & 1u) != 0u;
    let is_two_sided = (inst.flags & 2u) != 0u;
    let is_decal = (inst.flags & 16u) != 0u && cull.decal_bias_enabled != 0u;
    let is_multiply = (inst.flags & 64u) != 0u;
    let is_alphatest = !is_blend && inst.alpha_cutoff > 0.0;

    var bucket = 0u;
    if (is_blend) {
        bucket = select(select(6u, 8u, is_decal), 10u, is_multiply);
    } else if (is_decal) {
        bucket = 4u;
    } else if (is_alphatest) {
        bucket = 2u;
    }
    if (is_two_sided) {
        bucket += 1u;
    }

    // Each per-bucket buffer is sized to the CPU-tracked resident potential — a
    // hard upper bound on how many draws can route here this frame — so `slot`
    // is in range by construction. The arrayLength guard is defense in depth:
    // if the CPU routing mirror (gpu_driven::potential_buckets) ever drifts from
    // this routing and a bucket is undersized, an excess draw is dropped rather
    // than written out of bounds past the buffer.
    let slot = atomicAdd(&draw_counts[bucket], 1u);
    switch bucket {
        case 0u: { if (slot < arrayLength(&draws_opaque_culled)) { draws_opaque_culled[slot] = cmd; } }
        case 1u: { if (slot < arrayLength(&draws_opaque_twosided)) { draws_opaque_twosided[slot] = cmd; } }
        case 2u: { if (slot < arrayLength(&draws_alphatest_culled)) { draws_alphatest_culled[slot] = cmd; } }
        case 3u: { if (slot < arrayLength(&draws_alphatest_twosided)) { draws_alphatest_twosided[slot] = cmd; } }
        case 4u: { if (slot < arrayLength(&draws_decal_alphatest_culled)) { draws_decal_alphatest_culled[slot] = cmd; } }
        case 5u: { if (slot < arrayLength(&draws_decal_alphatest_twosided)) { draws_decal_alphatest_twosided[slot] = cmd; } }
        case 6u: { if (slot < arrayLength(&draws_blend_culled)) { draws_blend_culled[slot] = cmd; } }
        case 7u: { if (slot < arrayLength(&draws_blend_twosided)) { draws_blend_twosided[slot] = cmd; } }
        case 8u: { if (slot < arrayLength(&draws_decal_blend_culled)) { draws_decal_blend_culled[slot] = cmd; } }
        case 9u: { if (slot < arrayLength(&draws_decal_blend_twosided)) { draws_decal_blend_twosided[slot] = cmd; } }
        case 10u: { if (slot < arrayLength(&draws_multiply_culled)) { draws_multiply_culled[slot] = cmd; } }
        default: { if (slot < arrayLength(&draws_multiply_twosided)) { draws_multiply_twosided[slot] = cmd; } }
    }
}
