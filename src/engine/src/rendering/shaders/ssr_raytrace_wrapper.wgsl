// Scene copy group (`SceneCopy::layout`): colour and its sampler; depth is unused.
@group(0) @binding(0) var ssr_scene_color: texture_2d<f32>;
@group(0) @binding(2) var ssr_scene_sampler: sampler;

@group(1) @binding(0) var<uniform> ssr_raytrace_cb0: array<vec4<f32>, 1>;
@group(1) @binding(1) var ssr_hiz: texture_2d<f32>;
@group(1) @binding(2) var ssr_ray_anchor: texture_2d<f32>;
@group(1) @binding(3) var ssr_ray_view_z: texture_2d<f32>;
@group(1) @binding(4) var ssr_point_sampler: sampler;

struct SsrRaytraceVarying {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> SsrRaytraceVarying {
    let vertex = ssr_fullscreen_vertex(vertex_index);
    var input: ssr_raytrace_vs_in;
    input.v0 = vec4<f32>(vertex.ndc, 0.0, 0.0);
    input.v1 = vec4<f32>(vertex.uv, 0.0, 0.0);
    let output = ssr_raytrace_vs(input);
    return SsrRaytraceVarying(output.o0, output.o1.xy);
}

@fragment
fn fs_raytrace(frag: SsrRaytraceVarying) -> @location(0) vec4<f32> {
    var input: ssr_raytrace_ps_in;
    input.v1 = vec4<f32>(frag.uv, 0.0, 0.0);
    return ssr_raytrace_ps(input).o0;
}
