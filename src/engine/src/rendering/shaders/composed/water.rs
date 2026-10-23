use crate::rendering::shaders::composition::{ComposedShaderSource, ShaderPart};

const WRAPPER: ShaderPart = ShaderPart::Static(include_str!("../water_wrapper.wgsl"));
const RAY_SETUP_WRAPPER: ShaderPart =
    ShaderPart::Static(include_str!("../water_ray_setup_wrapper.wgsl"));

pub static WATER: ComposedShaderSource = ComposedShaderSource {
    label: "water_shader",
    parts: &[
        WRAPPER,
        ShaderPart::vertex("water_vs_basic"),
        ShaderPart::vertex("water_vs_wading"),
        ShaderPart::vertex("water_vs_underwater"),
        ShaderPart::vertex("water_vs_lod"),
        ShaderPart::vertex("water_vs_lod_ssr"),
        ShaderPart::vertex("water_vs_position"),
        ShaderPart::fragment("water_ps_depth"),
        ShaderPart::fragment("water_ps_depth_interior"),
        ShaderPart::fragment("water_ps_refraction"),
        ShaderPart::fragment("water_ps_underwater"),
        ShaderPart::fragment("water_ps_lod"),
        ShaderPart::fragment("water_ps_lod_ssr"),
        ShaderPart::fragment("water_ps_depth_fog"),
        ShaderPart::fragment("water_ps_mask_surface"),
        ShaderPart::fragment("water_ps_mask_wading"),
    ],
};

pub static WATER_SSR_RAY_SETUP: ComposedShaderSource = ComposedShaderSource {
    label: "water_ssr_ray_setup_shader",
    parts: &[
        WRAPPER,
        RAY_SETUP_WRAPPER,
        ShaderPart::vertex("water_vs_basic"),
        ShaderPart::vertex("water_vs_wading"),
        ShaderPart::vertex("water_vs_underwater"),
        ShaderPart::vertex("water_vs_lod"),
        ShaderPart::vertex("water_vs_lod_ssr"),
        ShaderPart::vertex("water_vs_position"),
        ShaderPart::fragment("water_ps_depth"),
        ShaderPart::fragment("water_ps_depth_interior"),
        ShaderPart::fragment("water_ps_refraction"),
        ShaderPart::fragment("water_ps_underwater"),
        ShaderPart::fragment("water_ps_lod"),
        ShaderPart::fragment("water_ps_lod_ssr"),
        ShaderPart::fragment("water_ps_depth_fog"),
        ShaderPart::fragment("water_ps_mask_surface"),
        ShaderPart::fragment("water_ps_mask_wading"),
        ShaderPart::vertex("water_vs_ray_setup"),
        ShaderPart::vertex("water_vs_ray_setup_wading"),
        ShaderPart::vertex("water_vs_ray_setup_lod"),
        ShaderPart::fragment("water_ps_ray_setup"),
        ShaderPart::fragment("water_ps_ray_setup_wading"),
        ShaderPart::fragment("water_ps_ray_setup_lod"),
    ],
};

pub static MODULES: &[&ComposedShaderSource] = &[&WATER, &WATER_SSR_RAY_SETUP];

#[cfg(test)]
mod stub_data {
    use crate::rendering::shaders::translated::ShaderStage::{self, Fragment, Vertex};

    /// (name, stage, input registers, output registers) of every translated water function.
    pub const FUNCTIONS: &[(&str, ShaderStage, &[&str], &[&str])] = &[
        (
            "water_vs_basic",
            Vertex,
            &["v0"],
            &["o0", "o1", "o2", "o3", "o4", "o5", "o6"],
        ),
        (
            "water_vs_wading",
            Vertex,
            &["v0", "v1"],
            &["o0", "o1", "o2", "o3", "o4", "o5", "o6", "o7"],
        ),
        (
            "water_vs_underwater",
            Vertex,
            &["v0", "v1"],
            &["o0", "o1", "o2", "o3", "o4", "o5", "o6", "o7"],
        ),
        (
            "water_vs_lod",
            Vertex,
            &["v0"],
            &["o0", "o1", "o2", "o3", "o4"],
        ),
        (
            "water_vs_lod_ssr",
            Vertex,
            &["v0"],
            &["o0", "o1", "o2", "o3", "o4"],
        ),
        (
            "water_vs_position",
            Vertex,
            &["v0"],
            &["o0", "o1", "o2", "o3"],
        ),
        (
            "water_vs_ray_setup",
            Vertex,
            &["v0"],
            &["o0", "o1", "o2", "o3", "o4", "o5", "o6"],
        ),
        (
            "water_vs_ray_setup_wading",
            Vertex,
            &["v0", "v1"],
            &["o0", "o1", "o2", "o3", "o4", "o5", "o6", "o7"],
        ),
        (
            "water_vs_ray_setup_lod",
            Vertex,
            &["v0"],
            &["o0", "o1", "o2", "o3", "o4"],
        ),
        (
            "water_ps_depth",
            Fragment,
            &["v0", "v1", "v4", "v5", "v6", "v7"],
            &["o0"],
        ),
        (
            "water_ps_depth_interior",
            Fragment,
            &["v0", "v1", "v4", "v5", "v6", "v7"],
            &["o0"],
        ),
        (
            "water_ps_refraction",
            Fragment,
            &["v0", "v1", "v4", "v5", "v6"],
            &["o0"],
        ),
        (
            "water_ps_underwater",
            Fragment,
            &["v0", "v1", "v4", "v5", "v6"],
            &["o0"],
        ),
        ("water_ps_lod", Fragment, &["v1", "v3", "v4"], &["o0"]),
        (
            "water_ps_lod_ssr",
            Fragment,
            &["v0", "v1", "v3", "v4"],
            &["o0"],
        ),
        ("water_ps_depth_fog", Fragment, &["v0", "v1"], &["o0"]),
        ("water_ps_mask_surface", Fragment, &[], &["o0"]),
        ("water_ps_mask_wading", Fragment, &[], &["o0"]),
        (
            "water_ps_ray_setup",
            Fragment,
            &["v0", "v1", "v4", "v5", "v6"],
            &["o0", "o1"],
        ),
        (
            "water_ps_ray_setup_wading",
            Fragment,
            &["v0", "v1", "v4", "v5", "v6", "v7"],
            &["o0", "o1"],
        ),
        (
            "water_ps_ray_setup_lod",
            Fragment,
            &["v0", "v3", "v4"],
            &["o0", "o1"],
        ),
    ];
}

/// Stand-ins with the interface the emitter produces (`<name>_in`, `<name>_out`, `fn <name>`).
#[cfg(test)]
pub fn stubs() -> Vec<crate::rendering::shaders::translated::StubShader> {
    stub_data::FUNCTIONS
        .iter()
        .map(|&(name, stage, inputs, outputs)| {
            (
                name,
                stage,
                stub_source(name, inputs, outputs, &stub_body(name)),
            )
        })
        .collect()
}

/// The vertex stand-ins transform the position by the per-draw rows (clip position in `o0`,
/// camera-relative position in `o1`).
#[cfg(test)]
const VERTEX_TRANSFORM: &str = concat!(
    "    let p = vec4<f32>(i.v0.xyz, 1.0);\n",
    "    o.o0 = vec4<f32>(dot(geometry.vs_cb2[3], p), dot(geometry.vs_cb2[4], p), dot(geometry.vs_cb2[5], p), dot(geometry.vs_cb2[6], p));\n",
    "    o.o1 = vec4<f32>(dot(geometry.vs_cb2[0], p), dot(geometry.vs_cb2[1], p), dot(geometry.vs_cb2[2], p), 0.0);\n",
);

/// What the position vertex stand-in adds to `o1.w`, and the depth-fog stand-in adds to its
/// blue channel when it arrives: it tells which vertex entry fed a depth-fog draw.
#[cfg(test)]
pub const POSITION_ENTRY_MARKER: f32 = 0.125;

/// The little behaviour the plumbing tests rely on: vertex stand-ins transform the position by
/// the per-draw rows, the position stand-in marks its output, the mask passes write their tag
/// (2/255 surface, 3/255 wading) into alpha and the depth-fog pass writes a marker colour whose
/// blue channel carries the vertex marker and whose alpha is the fragment depth, which the tests
/// vary.
#[cfg(test)]
fn stub_body(name: &str) -> String {
    match name {
        "water_ps_depth_fog" => {
            "    o.o0 = vec4<f32>(0.25, 0.5, 0.75 + i.v1.w, i.v0.z);\n".to_owned()
        }
        "water_ps_mask_surface" => "    o.o0.w = 0.0078431373;\n".to_owned(),
        "water_ps_mask_wading" => "    o.o0.w = 0.0117647059;\n".to_owned(),
        "water_vs_position" => format!("{VERTEX_TRANSFORM}    o.o1.w = {POSITION_ENTRY_MARKER};\n"),
        _ if name.starts_with("water_vs_") => VERTEX_TRANSFORM.to_owned(),
        _ => String::new(),
    }
}

#[cfg(test)]
fn stub_source(name: &str, inputs: &[&str], outputs: &[&str], body: &str) -> String {
    let fields = |registers: &[&str]| -> String {
        if registers.is_empty() {
            "unused: u32,".to_owned()
        } else {
            registers
                .iter()
                .map(|register| format!("{register}: vec4<f32>,"))
                .collect::<Vec<_>>()
                .join("\n")
        }
    };
    format!(
        "struct {name}_in {{\n{}\n}}\nstruct {name}_out {{\n{}\n}}\nfn {name}(i: {name}_in) -> {name}_out {{\n    var o: {name}_out;\n{body}    return o;\n}}\n",
        fields(inputs),
        fields(outputs),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_translated_part_of_the_modules_has_a_stub_of_the_same_stage() {
        for module in MODULES {
            for part in module.parts {
                if let ShaderPart::Translated { name, stage } = part {
                    let stub = stub_data::FUNCTIONS.iter().find(|f| f.0 == *name);
                    assert_eq!(stub.map(|f| f.1), Some(*stage), "{name}");
                }
            }
        }
    }
}
