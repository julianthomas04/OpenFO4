// SSR ray setup of the water surfaces, appended to `water_wrapper.wgsl` (it reuses the
// bind groups and the vertex plumbing). The draw targets the half-resolution pair of
// anchor uv (`Rg16Float`) and view-space z (`R32Float`), cleared to 0, which the raytrace
// pass reads. The frame group is the SSR copy of the water frame constants
// (`rendering/water/ssr/constants.rs`).
//
// The translated pixel shaders have no depth test of their own, the draw is not depth tested
// against a half-resolution buffer here either: `water_visible` tests the fragment against the
// scene depth of its 2x2 full-resolution block instead, so hidden water leaves both targets at 0
// and costs no ray march.

struct RaySetupOutput {
    @location(0) anchor_uv: vec2<f32>,
    @location(1) view_z: vec4<f32>,
}

// Raw scene depth in the 2x2 block of full-resolution texels covering the half-resolution
// pixel `half_pixel`; texels past the edge read as the far plane.
fn scene_block_min_depth(half_pixel: vec2<i32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(scene_depth));
    var farthest = 1.0;
    for (var dy = 0; dy < 2; dy = dy + 1) {
        for (var dx = 0; dx < 2; dx = dx + 1) {
            let texel = half_pixel * 2 + vec2<i32>(dx, dy);
            if (all(texel < dims)) {
                farthest = min(farthest, textureLoad(scene_depth, texel, 0));
            }
        }
    }
    return farthest;
}

// Reverse-Z: the fragment is visible when it is at least as near as the farthest scene depth
// of its block.
fn water_visible(frag: vec4<f32>) -> bool {
    return frag.z >= scene_block_min_depth(vec2<i32>(frag.xy));
}

fn ray_setup_output(anchor_uv: vec4<f32>, view_z: vec4<f32>) -> RaySetupOutput {
    return RaySetupOutput(anchor_uv.xy, view_z);
}

@vertex
fn vs_ray_setup(v: WaterVertexInput) -> WaterVarying {
    var i: water_vs_ray_setup_in;
    i.v0 = position_input(v.position);
    let o = water_vs_ray_setup(i);
    return WaterVarying(o.o0, o.o1, o.o2, o.o3, o.o4, o.o5, o.o6, vec4<f32>(0.0));
}

@vertex
fn vs_ray_setup_wading(v: WaterVertexInput) -> WaterVarying {
    var i: water_vs_ray_setup_wading_in;
    i.v0 = position_input(v.position);
    i.v1 = texcoord_input(v.tex_coords);
    let o = water_vs_ray_setup_wading(i);
    return WaterVarying(o.o0, o.o1, o.o2, o.o3, o.o4, o.o5, with_displacement_uv(o.o6, o.o1.xy), o.o7);
}

@vertex
fn vs_ray_setup_lod(v: WaterVertexInput) -> WaterVarying {
    var i: water_vs_ray_setup_lod_in;
    i.v0 = position_input(v.position);
    let o = water_vs_ray_setup_lod(i);
    return WaterVarying(o.o0, o.o1, o.o2, o.o3, o.o4, vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
}

struct RaySetupInput {
    @builtin(position) pos: vec4<f32>,
    @location(1) v1: vec4<f32>,
    @location(4) v4: vec4<f32>,
    @location(5) v5: vec4<f32>,
    @location(6) v6: vec4<f32>,
}

@fragment
fn fs_ray_setup(f: RaySetupInput) -> RaySetupOutput {
    var i: water_ps_ray_setup_in;
    i.v0 = f.pos;
    i.v1 = f.v1;
    i.v4 = f.v4;
    i.v5 = f.v5;
    i.v6 = f.v6;
    let o = water_ps_ray_setup(i);
    if (!water_visible(f.pos)) {
        discard;
    }
    return ray_setup_output(o.o0, o.o1);
}

struct RaySetupWadingInput {
    @builtin(position) pos: vec4<f32>,
    @location(1) v1: vec4<f32>,
    @location(4) v4: vec4<f32>,
    @location(5) v5: vec4<f32>,
    @location(6) v6: vec4<f32>,
    @location(7) v7: vec4<f32>,
}

@fragment
fn fs_ray_setup_wading(f: RaySetupWadingInput) -> RaySetupOutput {
    var i: water_ps_ray_setup_wading_in;
    i.v0 = f.pos;
    i.v1 = f.v1;
    i.v4 = f.v4;
    i.v5 = f.v5;
    i.v6 = f.v6;
    i.v7 = f.v7;
    let o = water_ps_ray_setup_wading(i);
    if (!water_visible(f.pos)) {
        discard;
    }
    return ray_setup_output(o.o0, o.o1);
}

struct RaySetupLodInput {
    @builtin(position) pos: vec4<f32>,
    @location(3) v3: vec4<f32>,
    @location(4) v4: vec4<f32>,
}

@fragment
fn fs_ray_setup_lod(f: RaySetupLodInput) -> RaySetupOutput {
    var i: water_ps_ray_setup_lod_in;
    i.v0 = f.pos;
    i.v3 = f.v3;
    i.v4 = f.v4;
    let o = water_ps_ray_setup_lod(i);
    if (!water_visible(f.pos)) {
        discard;
    }
    return ray_setup_output(o.o0, o.o1);
}
