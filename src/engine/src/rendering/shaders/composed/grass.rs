use crate::rendering::shaders::composition::{ComposedShaderSource, ShaderPart};

/// Instanced grass: the object shading path, the wrapper (`grass.wgsl`) and the translated
/// instancing vertex shader `grass_vs`.
pub static GRASS: ComposedShaderSource = ComposedShaderSource {
    label: "grass_shader",
    parts: &[
        ShaderPart::Static(include_str!("../binding_array.wgsl")),
        ShaderPart::Static(include_str!("../light_data.wgsl")),
        ShaderPart::Static(include_str!("../frame.wgsl")),
        ShaderPart::Static(include_str!("../instance.wgsl")),
        ShaderPart::Static(include_str!("../scene_bindings.wgsl")),
        ShaderPart::Static(include_str!("../normal_mapping.wgsl")),
        ShaderPart::Static(include_str!("../shading_math.wgsl")),
        ShaderPart::Static(include_str!("../ambient.wgsl")),
        ShaderPart::Static(include_str!("../surface_shading.wgsl")),
        ShaderPart::Static(include_str!("../lamp_shadow.wgsl")),
        ShaderPart::Static(include_str!("../mip_debug.wgsl")),
        ShaderPart::Static(include_str!("../lighting.wgsl")),
        ShaderPart::Static(include_str!("../surface.wgsl")),
        ShaderPart::Static(include_str!("../grass.wgsl")),
        ShaderPart::vertex("grass_vs"),
    ],
};

pub static MODULES: &[&ComposedShaderSource] = &[&GRASS];

/// Stand-in for the translated `grass_vs`: same input and output structs and constant register
/// globals (read through a dynamic `grass_cb2` index like the real one), no shader logic.
#[cfg(test)]
pub fn stubs() -> Vec<crate::rendering::shaders::translated::StubShader> {
    use crate::rendering::shaders::translated::ShaderStage;

    const GRASS_VS: &str = "
struct grass_vs_in {
    v0: vec4<f32>, v1: vec4<f32>, v2: vec4<f32>, v3: vec4<f32>, v4: vec4<f32>,
    v5: vec4<f32>, v6: vec4<f32>, v7: vec4<f32>, v8: vec4<f32>,
}
struct grass_vs_out {
    o0: vec4<f32>, o1: vec4<f32>, o2: vec4<f32>, o3: vec4<f32>,
    o4: vec4<f32>, o5: vec4<f32>, o6: vec4<f32>,
}
fn grass_vs(i: grass_vs_in) -> grass_vs_out {
    var o: grass_vs_out;
    let pushed = grass_cb2[13u + u32(i.v8.x)];
    o.o0 = grass_cb12[8] * i.v0 + pushed;
    o.o1 = i.v1 + grass_cb1[0];
    o.o2 = i.v2 + i.v6;
    o.o3 = i.v3 + i.v7;
    o.o4 = i.v4 + i.v5;
    o.o5 = i.v8;
    o.o6 = grass_cb12[35];
    return o;
}
";
    vec![("grass_vs", ShaderStage::Vertex, GRASS_VS.to_owned())]
}
