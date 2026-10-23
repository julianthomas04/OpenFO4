
// Bokeh depth of field: entry points, bind groups and input adaptors around the
// install-time translated passes (`dof_coc`, `dof_blur_h`, `dof_blur_diag`,
// `dof_resolve`, `dof_resolve_fogged`). Each pass is a fullscreen triangle:
//   fs_merge_depth              scene depth + surface depth -> one colour depth texture
//   fs_circle_of_confusion      scene + depth -> A: rgb = scene, a = blur radius
//   fs_blur_horizontal          A -> B
//   fs_blur_diagonals           B -> A
//   fs_resolve_plain / _fogged  scene copy + A + depth -> scene
//
// The translated passes expect a standard-depth buffer; the merge pass and the
// registers of `rendering/post/depth_of_field.rs` adapt the engine's reverse-Z
// depth to them: `dof_depth` holds the nearer of the two reverse-Z depths and
// `cb2[3] = (near, 0.5, -0.5, far)` makes the depth they decode equal `near / depth`.

// The smallest depth treated as finite (the horizon of the infinite far plane).
const MIN_DEPTH = 1e-7;

// The parameter registers every pass reads.
@group(0) @binding(0) var<uniform> dof_cb2: array<vec4<f32>, 6>;
// The scene as the effect starts (a pass cannot sample its own target).
@group(0) @binding(1) var dof_sharp: texture_2d<f32>;
// The blurred scene (w = blur coverage).
@group(0) @binding(2) var dof_blurred: texture_2d<f32>;
// The merged reverse-Z depth written by `fs_merge_depth`.
@group(0) @binding(3) var dof_depth: texture_2d<f32>;
// The underwater surface depth (0 where there is none).
@group(0) @binding(4) var dof_surface_depth: texture_2d<f32>;
// The input and the point-clamp sampler of the two blur passes.
@group(0) @binding(5) var dof_blur_source: texture_2d<f32>;
@group(0) @binding(6) var dof_blur_sampler: sampler;
// The live scene depth, read by the merge pass only.
@group(0) @binding(7) var dof_scene_depth: texture_depth_2d;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(fullscreen_triangle_ndc(vertex_index), 0.0, 1.0);
    return out;
}

// (uv, whole pixel coordinates) of a fragment of a scene-sized target.
fn scene_coords(frag: vec4<f32>) -> vec4<f32> {
    let uv = frag.xy / vec2<f32>(textureDimensions(dof_sharp));
    return vec4<f32>(uv, floor(frag.xy));
}

// The pixel of the blurred texture under the scene uv.
fn blurred_pixel(uv: vec2<f32>) -> vec4<f32> {
    let dims = vec2<i32>(textureDimensions(dof_blurred));
    let pixel = clamp(vec2<i32>(uv * vec2<f32>(dims)), vec2<i32>(0), dims - vec2<i32>(1));
    return vec4<f32>(vec2<f32>(pixel), 0.0, 0.0);
}

fn blur_uv(frag: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(frag.xy / vec2<f32>(textureDimensions(dof_blur_source)), 0.0, 0.0);
}

// Reverse-Z: the nearer surface has the larger depth.
@fragment
fn fs_merge_depth(@builtin(position) frag: vec4<f32>) -> @location(0) f32 {
    let pixel = vec2<i32>(frag.xy);
    let nearer = max(textureLoad(dof_scene_depth, pixel, 0), textureLoad(dof_surface_depth, pixel, 0).x);
    return max(nearer, MIN_DEPTH);
}

@fragment
fn fs_circle_of_confusion(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    var input: dof_coc_in;
    input.v1 = scene_coords(frag);
    return dof_coc(input).o0;
}

@fragment
fn fs_blur_horizontal(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    var input: dof_blur_h_in;
    input.v1 = blur_uv(frag);
    return dof_blur_h(input).o0;
}

@fragment
fn fs_blur_diagonals(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    var input: dof_blur_diag_in;
    input.v1 = blur_uv(frag);
    return dof_blur_diag(input).o0;
}

@fragment
fn fs_resolve_plain(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    var input: dof_resolve_in;
    input.v1 = scene_coords(frag);
    input.v2 = blurred_pixel(input.v1.xy);
    return dof_resolve(input).o0;
}

@fragment
fn fs_resolve_fogged(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    var input: dof_resolve_fogged_in;
    input.v1 = scene_coords(frag);
    input.v2 = blurred_pixel(input.v1.xy);
    return dof_resolve_fogged(input).o0;
}
