// Entry points and bindings of the wading water displacement passes. The
// per-pass vertex and fragment functions are the install-time translated
// `disp_*` shaders (see docs/engine/rendering/surfaces-and-effects.md); this file only
// supplies their inputs and the resources they read.
//
// The state texture is RGBA with every channel biased by 0.5: R height,
// G velocity, B and A the slope the water shader reads. The targets are
// UNORM, so the stores clamp to [0, 1].

// Register file of the pass constants, one dynamic-offset slot per pass:
//   advect: constants[0].zw is the texture offset
//   splat:  constants[1].xyz and constants[2].xyz are the two affine rows
//   step:   constants[2].xyz is stiffness, velocity scale, damping
@group(0) @binding(0) var<uniform> constants: array<vec4<f32>, 3>;
@group(0) @binding(1) var state: texture_2d<f32>;
@group(0) @binding(2) var point_sampler: sampler;
@group(0) @binding(3) var linear_sampler: sampler;

struct UvVertex {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec4<f32>,
};

struct StepVertex {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) taps_a: vec4<f32>,
    @location(1) taps_b: vec4<f32>,
    @location(2) taps_c: vec4<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> UvVertex {
    let clip = fullscreen_triangle_ndc(index);
    var input: disp_fullscreen_vs_in;
    input.v0 = vec4<f32>(clip, 0.0, 0.0);
    input.v1 = vec4<f32>(fullscreen_uv(clip), 0.0, 0.0);
    let output = disp_fullscreen_vs(input);
    return UvVertex(output.o0, output.o1);
}

@fragment
fn fs_tex_offset(vertex: UvVertex) -> @location(0) vec4<f32> {
    var input: disp_tex_offset_ps_in;
    input.v1 = vertex.uv;
    return disp_tex_offset_ps(input).o0;
}

// The splat's quad, [0,1] x [0,0.01], as two triangles.
const SPLAT_CORNERS = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(0.0, 0.01),
    vec2<f32>(0.0, 0.01),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(1.0, 0.01),
);

@vertex
fn vs_splat(@builtin(vertex_index) index: u32) -> UvVertex {
    let corner = SPLAT_CORNERS[index];
    var input: disp_splat_vs_in;
    input.v0 = vec4<f32>(corner, 0.0, 0.0);
    input.v1 = vec4<f32>(corner, 0.0, 0.0);
    let output = disp_splat_vs(input);
    return UvVertex(output.o0, output.o1);
}

@fragment
fn fs_splat() -> @location(0) vec4<f32> {
    var input: disp_splat_ps_in;
    return disp_splat_ps(input).o0;
}

@vertex
fn vs_step(@builtin(vertex_index) index: u32) -> StepVertex {
    let clip = fullscreen_triangle_ndc(index);
    var input: disp_step_vs_in;
    input.v0 = vec4<f32>(clip, 0.0, 0.0);
    input.v1 = vec4<f32>(fullscreen_uv(clip), 0.0, 0.0);
    let output = disp_step_vs(input);
    return StepVertex(output.o0, output.o1, output.o2, output.o3);
}

@fragment
fn fs_step(vertex: StepVertex) -> @location(0) vec4<f32> {
    var input: disp_step_ps_in;
    input.v1 = vertex.taps_a;
    input.v2 = vertex.taps_b;
    input.v3 = vertex.taps_c;
    return disp_step_ps(input).o0;
}
