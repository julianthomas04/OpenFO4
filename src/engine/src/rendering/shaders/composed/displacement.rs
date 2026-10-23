use crate::rendering::shaders::composition::{ComposedShaderSource, ShaderPart};

pub static WATER_DISPLACEMENT: ComposedShaderSource = ComposedShaderSource {
    label: "water_displacement_shader",
    parts: &[
        ShaderPart::vertex("disp_fullscreen_vs"),
        ShaderPart::fragment("disp_tex_offset_ps"),
        ShaderPart::vertex("disp_splat_vs"),
        ShaderPart::fragment("disp_splat_ps"),
        ShaderPart::vertex("disp_step_vs"),
        ShaderPart::fragment("disp_step_ps"),
        ShaderPart::Static(include_str!("../fullscreen.wgsl")),
        ShaderPart::Static(include_str!("../displacement_passes.wgsl")),
    ],
};

pub static MODULES: &[&ComposedShaderSource] = &[&WATER_DISPLACEMENT];

#[cfg(test)]
pub fn stubs() -> Vec<crate::rendering::shaders::translated::StubShader> {
    use crate::rendering::shaders::translated::ShaderStage::{Fragment, Vertex};

    let vertex = |name: &str, outputs: &str, body: &str| {
        format!(
            "struct {name}_in {{ v0: vec4<f32>, v1: vec4<f32>, }}\n\
             struct {name}_out {{ o0: vec4<f32>, {outputs} }}\n\
             fn {name}(i: {name}_in) -> {name}_out {{\n\
                 var o: {name}_out;\n\
                 o.o0 = i.v0 + constants[1];\n\
                 {body}\n\
                 return o;\n\
             }}\n"
        )
    };
    let tex_offset = "struct disp_tex_offset_ps_in { v1: vec4<f32>, }\n\
         struct disp_tex_offset_ps_out { o0: vec4<f32>, }\n\
         fn disp_tex_offset_ps(i: disp_tex_offset_ps_in) -> disp_tex_offset_ps_out {\n\
             var o: disp_tex_offset_ps_out;\n\
             o.o0 = textureSample(state, point_sampler, i.v1.xy + constants[0].zw);\n\
             return o;\n\
         }\n";
    let splat = "struct disp_splat_ps_in { unused: u32, }\n\
         struct disp_splat_ps_out { o0: vec4<f32>, }\n\
         fn disp_splat_ps(i: disp_splat_ps_in) -> disp_splat_ps_out {\n\
             var o: disp_splat_ps_out;\n\
             o.o0 = vec4<f32>(1.0);\n\
             return o;\n\
         }\n";
    let step = "struct disp_step_ps_in { v1: vec4<f32>, v2: vec4<f32>, v3: vec4<f32>, }\n\
         struct disp_step_ps_out { o0: vec4<f32>, }\n\
         fn disp_step_ps(i: disp_step_ps_in) -> disp_step_ps_out {\n\
             var o: disp_step_ps_out;\n\
             o.o0 = textureSample(state, linear_sampler, i.v1.xy + i.v2.xy + i.v3.xy) * constants[2];\n\
             return o;\n\
         }\n";
    vec![
        (
            "disp_fullscreen_vs",
            Vertex,
            vertex("disp_fullscreen_vs", "o1: vec4<f32>,", "o.o1 = i.v1;"),
        ),
        ("disp_tex_offset_ps", Fragment, tex_offset.to_owned()),
        (
            "disp_splat_vs",
            Vertex,
            vertex("disp_splat_vs", "o1: vec4<f32>,", "o.o1 = i.v1;"),
        ),
        ("disp_splat_ps", Fragment, splat.to_owned()),
        (
            "disp_step_vs",
            Vertex,
            vertex(
                "disp_step_vs",
                "o1: vec4<f32>, o2: vec4<f32>, o3: vec4<f32>,",
                "o.o1 = i.v1; o.o2 = i.v1; o.o3 = i.v1;",
            ),
        ),
        ("disp_step_ps", Fragment, step.to_owned()),
    ]
}
