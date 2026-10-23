@group(0) @binding(0) var<uniform> ssr_blur_cb0: array<vec4<f32>, 1>;
@group(0) @binding(1) var ssr_blur_source: texture_2d<f32>;
@group(0) @binding(2) var ssr_blur_sampler: sampler;

struct SsrBlurVarying {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv_0: vec2<f32>,
    @location(1) uv_1: vec2<f32>,
    @location(2) uv_2: vec2<f32>,
    @location(3) uv_3: vec2<f32>,
    @location(4) uv_4: vec2<f32>,
}

@vertex
fn vs_blur_h(@builtin(vertex_index) vertex_index: u32) -> SsrBlurVarying {
    let vertex = ssr_fullscreen_vertex(vertex_index);
    var input: ssr_blur_h_vs_in;
    input.v0 = vec4<f32>(vertex.ndc, 0.0, 0.0);
    input.v1 = vec4<f32>(vertex.uv, 0.0, 0.0);
    let o = ssr_blur_h_vs(input);
    return SsrBlurVarying(o.o0, o.o1.xy, o.o2.xy, o.o3.xy, o.o4.xy, o.o5.xy);
}

@vertex
fn vs_blur_v(@builtin(vertex_index) vertex_index: u32) -> SsrBlurVarying {
    let vertex = ssr_fullscreen_vertex(vertex_index);
    var input: ssr_blur_v_vs_in;
    input.v0 = vec4<f32>(vertex.ndc, 0.0, 0.0);
    input.v1 = vec4<f32>(vertex.uv, 0.0, 0.0);
    let o = ssr_blur_v_vs(input);
    return SsrBlurVarying(o.o0, o.o1.xy, o.o2.xy, o.o3.xy, o.o4.xy, o.o5.xy);
}

@fragment
fn fs_blur(frag: SsrBlurVarying) -> @location(0) vec4<f32> {
    var input: ssr_blur_ps_in;
    input.v1 = vec4<f32>(frag.uv_0, 0.0, 0.0);
    input.v2 = vec4<f32>(frag.uv_1, 0.0, 0.0);
    input.v3 = vec4<f32>(frag.uv_2, 0.0, 0.0);
    input.v4 = vec4<f32>(frag.uv_3, 0.0, 0.0);
    input.v5 = vec4<f32>(frag.uv_4, 0.0, 0.0);
    return ssr_blur_ps(input).o0;
}
