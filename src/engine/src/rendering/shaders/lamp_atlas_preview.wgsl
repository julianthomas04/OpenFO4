// Debug view of the lamp shadow atlas: reverse-Z depth brightened towards the
// far range, black where a tile is cleared or unused.
const PREVIEW_DEPTH_EXPONENT = 0.2;

@group(0) @binding(0) var lamp_atlas: texture_depth_2d;

struct PreviewVaryings {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_preview(@builtin(vertex_index) vertex_index: u32) -> PreviewVaryings {
    let ndc = fullscreen_triangle_ndc(vertex_index);
    var out: PreviewVaryings;
    out.clip_position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = ndc * vec2<f32>(0.5, -0.5) + 0.5;
    return out;
}

@fragment
fn fs_preview(in: PreviewVaryings) -> @location(0) vec4<f32> {
    let size = textureDimensions(lamp_atlas);
    let texel = min(vec2<u32>(in.uv * vec2<f32>(size)), size - 1u);
    let depth = textureLoad(lamp_atlas, texel, 0);
    return vec4<f32>(vec3<f32>(pow(depth, PREVIEW_DEPTH_EXPONENT)), 1.0);
}
