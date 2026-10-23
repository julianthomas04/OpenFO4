// Single-phase occlusion against the previous frame's depth pyramid
// (R32Float, min-reduced = farthest under reverse-Z), bound by the consumer
// as `hiz`. Conservative everywhere except fast camera cuts, where a freshly
// revealed box can be culled for one frame (accepted limitation).
fn hiz_occluded(aabb_min: vec3<f32>, aabb_max: vec3<f32>, view_proj: mat4x4<f32>, screen_size: vec2<f32>, mip_count: u32) -> bool {
    var min_ndc = vec2<f32>(1e30, 1e30);
    var max_ndc = vec2<f32>(-1e30, -1e30);
    var nearest = 0.0; // max NDC z = nearest under reverse-Z

    for (var i = 0u; i < 8u; i = i + 1u) {
        let corner = vec3<f32>(
            select(aabb_min.x, aabb_max.x, (i & 1u) != 0u),
            select(aabb_min.y, aabb_max.y, (i & 2u) != 0u),
            select(aabb_min.z, aabb_max.z, (i & 4u) != 0u),
        );
        let clip = view_proj * vec4<f32>(corner, 1.0);
        if (clip.w <= 0.0) {
            // Crosses the camera plane: projection is unreliable, keep it.
            return false;
        }
        let ndc = clip.xyz / clip.w;
        min_ndc = min(min_ndc, ndc.xy);
        max_ndc = max(max_ndc, ndc.xy);
        nearest = max(nearest, ndc.z);
    }

    // NDC -> pixel rect at mip 0 (y flips).
    let px_min = clamp(
        vec2<f32>(min_ndc.x * 0.5 + 0.5, -max_ndc.y * 0.5 + 0.5) * screen_size,
        vec2<f32>(0.0),
        screen_size - 1.0,
    );
    let px_max = clamp(
        vec2<f32>(max_ndc.x * 0.5 + 0.5, -min_ndc.y * 0.5 + 0.5) * screen_size,
        vec2<f32>(0.0),
        screen_size - 1.0,
    );

    // Pick the mip where the rect spans <= ~2 texels per axis (<= 4x4 loads
    // worst case after floor/ceil straddling).
    let size_px = max(px_max.x - px_min.x, px_max.y - px_min.y);
    let mip = u32(clamp(
        ceil(log2(max(size_px * 0.5, 1.0))),
        0.0,
        f32(mip_count - 1u),
    ));

    let mip_w = max(u32(screen_size.x) >> mip, 1u);
    let mip_h = max(u32(screen_size.y) >> mip, 1u);
    let x0 = min(u32(px_min.x) >> mip, mip_w - 1u);
    let x1 = min(u32(px_max.x) >> mip, mip_w - 1u);
    let y0 = min(u32(px_min.y) >> mip, mip_h - 1u);
    let y1 = min(u32(px_max.y) >> mip, mip_h - 1u);

    var farthest = 1.0;
    for (var y = y0; y <= y1; y = y + 1u) {
        for (var x = x0; x <= x1; x = x + 1u) {
            farthest = min(farthest, textureLoad(hiz, vec2<u32>(x, y), i32(mip)).r);
        }
    }

    // Occluded iff the box's nearest point is farther than the farthest
    // occluder depth across the covered texels.
    return nearest < farthest;
}
