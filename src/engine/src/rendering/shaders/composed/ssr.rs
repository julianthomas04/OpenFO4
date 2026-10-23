use crate::rendering::shaders::composition::{ComposedShaderSource, ShaderPart};

const FULLSCREEN: ShaderPart = ShaderPart::Static(include_str!("../fullscreen.wgsl"));
const SSR_FULLSCREEN: ShaderPart = ShaderPart::Static(include_str!("../ssr_fullscreen.wgsl"));

pub static WATER_SSR_RAYTRACE: ComposedShaderSource = ComposedShaderSource {
    label: "water_ssr_raytrace_shader",
    parts: &[
        FULLSCREEN,
        SSR_FULLSCREEN,
        ShaderPart::vertex("ssr_raytrace_vs"),
        ShaderPart::fragment("ssr_raytrace_ps"),
        ShaderPart::Static(include_str!("../ssr_raytrace_wrapper.wgsl")),
    ],
};

pub static WATER_SSR_BLUR: ComposedShaderSource = ComposedShaderSource {
    label: "water_ssr_blur_shader",
    parts: &[
        FULLSCREEN,
        SSR_FULLSCREEN,
        ShaderPart::vertex("ssr_blur_h_vs"),
        ShaderPart::vertex("ssr_blur_v_vs"),
        ShaderPart::fragment("ssr_blur_ps"),
        ShaderPart::Static(include_str!("../ssr_blur_wrapper.wgsl")),
    ],
};

pub static WATER_SSR_HIZ: ComposedShaderSource = ComposedShaderSource {
    label: "water_ssr_hiz_shader",
    parts: &[
        ShaderPart::compute("ssr_hiz_camera_cs"),
        ShaderPart::compute("ssr_hiz_mips_cs"),
        ShaderPart::Static(include_str!("../ssr_hiz_wrapper.wgsl")),
    ],
};

pub static MODULES: &[&ComposedShaderSource] =
    &[&WATER_SSR_RAYTRACE, &WATER_SSR_BLUR, &WATER_SSR_HIZ];

#[cfg(test)]
pub fn stubs() -> Vec<crate::rendering::shaders::translated::StubShader> {
    use crate::rendering::shaders::translated::ShaderStage::{Compute, Fragment, Vertex};

    const BLUR_VERTEX: &str = "
struct NAME_in { v0: vec4<f32>, v1: vec4<f32> }
struct NAME_out { o0: vec4<f32>, o1: vec4<f32>, o2: vec4<f32>, o3: vec4<f32>, o4: vec4<f32>, o5: vec4<f32> }
fn NAME(i: NAME_in) -> NAME_out {
    var o: NAME_out;
    let step = vec4<f32>(AXIS, 0.0, 0.0) * ssr_blur_cb0[0].x;
    o.o0 = vec4<f32>(i.v0.xyz, 1.0);
    o.o1 = i.v1 - step;
    o.o2 = i.v1 - step * 0.5;
    o.o3 = i.v1;
    o.o4 = i.v1 + step * 0.5;
    o.o5 = i.v1 + step;
    return o;
}
";
    let blur_vertex =
        |name: &str, axis: &str| BLUR_VERTEX.replace("NAME", name).replace("AXIS", axis);
    let hiz_stub = |name: &str, body: &str| {
        format!(
            "
struct {name}_in {{ vThreadId: vec3<u32>, vThreadIdInGroup: vec3<u32> }}
struct {name}_out {{ unused: u32 }}
const {name}_workgroup_size_x: u32 = 8u;
const {name}_workgroup_size_y: u32 = 8u;
const {name}_workgroup_size_z: u32 = 1u;
fn {name}(i: {name}_in) -> {name}_out {{
    let texel = i.vThreadId.xy;
{body}
    return {name}_out(0u);
}}
"
        )
    };
    vec![
        (
            "ssr_raytrace_vs",
            Vertex,
            "
struct ssr_raytrace_vs_in { v0: vec4<f32>, v1: vec4<f32> }
struct ssr_raytrace_vs_out { o0: vec4<f32>, o1: vec4<f32> }
fn ssr_raytrace_vs(i: ssr_raytrace_vs_in) -> ssr_raytrace_vs_out {
    return ssr_raytrace_vs_out(vec4<f32>(i.v0.xyz, 1.0), i.v1);
}
"
            .to_owned(),
        ),
        (
            "ssr_raytrace_ps",
            Fragment,
            "
struct ssr_raytrace_ps_in { v1: vec4<f32> }
struct ssr_raytrace_ps_out { o0: vec4<f32> }
fn ssr_raytrace_ps(i: ssr_raytrace_ps_in) -> ssr_raytrace_ps_out {
    let anchor = textureSampleLevel(ssr_ray_anchor, ssr_point_sampler, i.v1.xy, 0.0);
    let view_z = textureSampleLevel(ssr_ray_view_z, ssr_point_sampler, i.v1.xy, 0.0).x;
    let hiz = textureLoad(ssr_hiz, vec2<i32>(0), 1).x;
    let colour = textureSampleLevel(ssr_scene_color, ssr_scene_sampler, anchor.xy, 0.0);
    return ssr_raytrace_ps_out(colour * (view_z + hiz + ssr_raytrace_cb0[0].x));
}
"
            .to_owned(),
        ),
        ("ssr_blur_h_vs", Vertex, blur_vertex("ssr_blur_h_vs", "1.0, 0.0")),
        ("ssr_blur_v_vs", Vertex, blur_vertex("ssr_blur_v_vs", "0.0, 1.0")),
        (
            "ssr_blur_ps",
            Fragment,
            "
struct ssr_blur_ps_in { v1: vec4<f32>, v2: vec4<f32>, v3: vec4<f32>, v4: vec4<f32>, v5: vec4<f32> }
struct ssr_blur_ps_out { o0: vec4<f32> }
fn ssr_blur_ps(i: ssr_blur_ps_in) -> ssr_blur_ps_out {
    let sum = textureSampleLevel(ssr_blur_source, ssr_blur_sampler, i.v1.xy, 0.0)
        + textureSampleLevel(ssr_blur_source, ssr_blur_sampler, i.v2.xy, 0.0)
        + textureSampleLevel(ssr_blur_source, ssr_blur_sampler, i.v3.xy, 0.0)
        + textureSampleLevel(ssr_blur_source, ssr_blur_sampler, i.v4.xy, 0.0)
        + textureSampleLevel(ssr_blur_source, ssr_blur_sampler, i.v5.xy, 0.0);
    return ssr_blur_ps_out(sum * 0.2);
}
"
            .to_owned(),
        ),
        (
            "ssr_hiz_camera_cs",
            Compute,
            hiz_stub(
                "ssr_hiz_camera_cs",
                "    let z = ssr_linear_z[0].x / (textureLoad(ssr_hiz_depth, vec2<i32>(texel * 2u), 0) + ssr_linear_z[0].z);
    textureStore(ssr_hiz_mip0, texel * 2u, vec4<f32>(z));
    textureStore(ssr_hiz_mip1, texel, vec4<f32>(z));
    textureStore(ssr_hiz_mip2, texel / 2u, vec4<f32>(z));",
            ),
        ),
        (
            "ssr_hiz_mips_cs",
            Compute,
            hiz_stub(
                "ssr_hiz_mips_cs",
                "    let z = textureLoad(ssr_hiz_mip2_in, vec2<i32>(texel * 2u), 0).x;
    textureStore(ssr_hiz_mip3, texel, vec4<f32>(z));
    textureStore(ssr_hiz_mip4, texel / 2u, vec4<f32>(z));",
            ),
        ),
    ]
}
