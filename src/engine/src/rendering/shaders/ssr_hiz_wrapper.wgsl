@group(0) @binding(0) var ssr_hiz_depth: texture_depth_2d;
@group(0) @binding(1) var<uniform> ssr_linear_z: array<vec4<f32>, 1>;
@group(0) @binding(2) var ssr_hiz_mip0: texture_storage_2d<r32float, write>;
@group(0) @binding(3) var ssr_hiz_mip1: texture_storage_2d<r32float, write>;
@group(0) @binding(4) var ssr_hiz_mip2: texture_storage_2d<r32float, write>;
@group(0) @binding(5) var ssr_hiz_mip2_in: texture_2d<f32>;
@group(0) @binding(6) var ssr_hiz_mip3: texture_storage_2d<r32float, write>;
@group(0) @binding(7) var ssr_hiz_mip4: texture_storage_2d<r32float, write>;

@compute @workgroup_size(ssr_hiz_camera_cs_workgroup_size_x, ssr_hiz_camera_cs_workgroup_size_y, ssr_hiz_camera_cs_workgroup_size_z)
fn hiz_camera_and_mips(
    @builtin(global_invocation_id) thread: vec3<u32>,
    @builtin(local_invocation_id) thread_in_group: vec3<u32>,
) {
    var input: ssr_hiz_camera_cs_in;
    input.vThreadId = thread;
    input.vThreadIdInGroup = thread_in_group;
    _ = ssr_hiz_camera_cs(input);
}

@compute @workgroup_size(ssr_hiz_mips_cs_workgroup_size_x, ssr_hiz_mips_cs_workgroup_size_y, ssr_hiz_mips_cs_workgroup_size_z)
fn hiz_mips(
    @builtin(global_invocation_id) thread: vec3<u32>,
    @builtin(local_invocation_id) thread_in_group: vec3<u32>,
) {
    var input: ssr_hiz_mips_cs_in;
    input.vThreadId = thread;
    input.vThreadIdInGroup = thread_in_group;
    _ = ssr_hiz_mips_cs(input);
}
