use crate::rendering::shaders::composition::{ComposedShaderSource, ShaderPart};

pub static DEPTH_OF_FIELD: ComposedShaderSource = ComposedShaderSource {
    label: "depth_of_field_shader",
    parts: &[
        ShaderPart::Static(include_str!("../fullscreen.wgsl")),
        ShaderPart::Static(include_str!("../depth_of_field_wrapper.wgsl")),
        ShaderPart::fragment("dof_coc"),
        ShaderPart::fragment("dof_blur_h"),
        ShaderPart::fragment("dof_blur_diag"),
        ShaderPart::fragment("dof_resolve"),
        ShaderPart::fragment("dof_resolve_fogged"),
    ],
};

pub static MODULES: &[&ComposedShaderSource] = &[&DEPTH_OF_FIELD];

/// Stand-ins with the translated functions' signatures that read the registers and
/// textures the wrapper declares, so the module's types are checked without game data.
#[cfg(test)]
pub fn stubs() -> Vec<crate::rendering::shaders::translated::StubShader> {
    use crate::rendering::shaders::translated::ShaderStage;

    let stub = |name: &'static str, inputs: &str, body: &str| {
        let wgsl = format!(
            "struct {name}_in {{ {inputs} }}\n\
             struct {name}_out {{ o0: vec4<f32>, }}\n\
             fn {name}(i: {name}_in) -> {name}_out {{ return {name}_out({body}); }}\n"
        );
        (name, ShaderStage::Fragment, wgsl)
    };
    let scene_inputs = "v1: vec4<f32>, v2: vec4<f32>,";
    let resolve_body = "textureLoad(dof_blurred, vec2<i32>(i.v2.xy), 0) \
         + textureLoad(dof_sharp, vec2<i32>(i.v1.zw), 0) \
         + textureLoad(dof_depth, vec2<i32>(i.v1.zw), 0)";
    let fogged_body = format!("dof_cb2[4] + {resolve_body}");
    let blur_body = "dof_cb2[1] + textureSample(dof_blur_source, dof_blur_sampler, i.v1.xy)";
    vec![
        stub(
            "dof_coc",
            "v1: vec4<f32>,",
            "dof_cb2[3] + textureLoad(dof_sharp, vec2<i32>(i.v1.zw), 0) \
             + textureLoad(dof_depth, vec2<i32>(i.v1.zw), 0)",
        ),
        stub("dof_blur_h", "v1: vec4<f32>,", blur_body),
        stub("dof_blur_diag", "v1: vec4<f32>,", blur_body),
        stub("dof_resolve", scene_inputs, resolve_body),
        stub("dof_resolve_fogged", scene_inputs, &fogged_body),
    ]
}
