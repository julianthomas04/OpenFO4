// Oversized triangle covering clip space; vertices (-1,-1), (3,-1), (-1,3).
fn fullscreen_triangle_ndc(vertex_index: u32) -> vec2<f32> {
    let uv = vec2<f32>(f32((vertex_index << 1u) & 2u), f32(vertex_index & 2u));
    return uv * 2.0 - 1.0;
}

// Texture coordinates of a clip-space position; v grows downwards like the target's rows.
fn fullscreen_uv(ndc: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}
