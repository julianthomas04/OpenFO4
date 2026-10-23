struct Params {
    uv_min: vec2<f32>,
    uv_max: vec2<f32>,
    alpha: f32,
    target_is_srgb: f32,
    padding: vec2<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var overlay: texture_2d<f32>;
@group(0) @binding(2) var overlay_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u));
    var out: VertexOutput;
    out.position = vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
    out.uv = mix(params.uv_min, params.uv_max, corner);
    return out;
}

fn srgb_to_linear(encoded: vec3<f32>) -> vec3<f32> {
    let low = encoded / 12.92;
    let high = pow((encoded + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, encoded <= vec3<f32>(0.04045));
}

// The blend state multiplies the framebuffer by this colour: dst * (1 - a + a * overlay).
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(overlay, overlay_sampler, in.uv);
    let coverage = texel.a * params.alpha;
    var factor = vec3<f32>(1.0 - coverage) + coverage * texel.rgb;
    if params.target_is_srgb > 0.5 {
        factor = srgb_to_linear(factor);
    }
    return vec4<f32>(factor, 1.0);
}
