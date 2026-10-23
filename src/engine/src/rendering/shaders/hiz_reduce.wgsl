@group(0) @binding(0) var input_mip: texture_2d<f32>;
@group(0) @binding(1) var output_mip: texture_storage_2d<r32float, write>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let out_size = textureDimensions(output_mip);
    if gid.x >= out_size.x || gid.y >= out_size.y {
        return;
    }

    let src = gid.xy * 2u;
    let in_size = textureDimensions(input_mip);

    // When an input dimension is odd, its last row/column has no parent texel
    // (out = in >> 1 floors); fold it into the last output texel, or the
    // pyramid would claim screen edges are nearer than reality and over-cull.
    let extra_x = select(0u, 1u, (in_size.x & 1u) != 0u && gid.x == out_size.x - 1u);
    let extra_y = select(0u, 1u, (in_size.y & 1u) != 0u && gid.y == out_size.y - 1u);

    // Reverse-Z: min = farthest = most conservative for occlusion
    var farthest = 1.0;
    for (var dy = 0u; dy <= 1u + extra_y; dy = dy + 1u) {
        for (var dx = 0u; dx <= 1u + extra_x; dx = dx + 1u) {
            let p = min(src + vec2(dx, dy), in_size - 1u);
            farthest = min(farthest, textureLoad(input_mip, p, 0).r);
        }
    }

    textureStore(output_mip, gid.xy, vec4(farthest, 0.0, 0.0, 0.0));
}
