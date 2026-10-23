//! Composition of install-time translated effect shaders into shader modules.
//!
//! A module holds the static wrapper (`effect_wrapper.wgsl`: the register arrays, textures and
//! samplers the translated functions read), one translated vertex function, one translated
//! pixel function and generated glue. The glue is built from the shaders' interface metadata
//! in the translator's manifest: the vertex entry fills each vertex-function input register
//! from the vertex attribute with the matching semantic, each pixel-function input register
//! is fed from the vertex output register that writes the same semantic, and the position
//! and colour target connect to the rasteriser and the render target.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use super::constants::parse_descriptor_digits;
use crate::assets::types::effect_material::EffectMaterial;
use crate::rendering::shaders::{
    composition::MODULE_PRELUDE,
    startup::{self, WgslFault},
    translated::{IoElement, ShaderInterface, ShaderLoadError, ShaderStage, TranslatedShaders},
};

pub const EFFECT_WRAPPER: &str = include_str!("../shaders/effect_wrapper.wgsl");

/// Every translated shader of the effect family starts with this.
pub const EFFECT_SHADER_PREFIX: &str = "effect_";
const VERTEX_PREFIX: &str = "effect_vs_";
const PIXEL_PREFIX: &str = "effect_ps_";

pub const VERTEX_ENTRY: &str = "vs_main";
pub const FRAGMENT_ENTRY: &str = "fs_main";

/// Descriptor bit of the pixel shaders that selects the decal variant.
const DECAL_BIT: u32 = 0x1000_0000;

/// Bind groups and bindings of `effect_wrapper.wgsl`.
pub mod bindings {
    pub const SCENE_GROUP: u32 = 0;
    pub const FRAME_GROUP: u32 = 1;
    pub const MATERIAL_GROUP: u32 = 2;
    pub const DRAW_GROUP: u32 = 3;

    /// Binding of the scene depth copy inside the scene copy's layout.
    pub const SCENE_DEPTH_BINDING: u32 = 1;
    pub const FRAME_BINDING: u32 = 0;
    pub const DRAW_BINDING: u32 = 0;

    /// Texture slots of the material group, indexed by the translated shaders' `tN`
    /// register. Slot 3 is the scene depth copy and is not part of the material.
    pub const MATERIAL_TEXTURE_SLOTS: [u32; 8] = [0, 1, 2, 4, 5, 6, 7, 8];
    /// The one slot that is a cube map.
    pub const CUBE_TEXTURE_SLOT: u32 = 5;
    const SAMPLER_BINDING_BASE: u32 = 10;

    /// Binding of texture slot `slot` in the material group.
    pub const fn texture_binding(slot: u32) -> u32 {
        slot
    }

    /// Binding of the sampler `sN` in the material group.
    pub const fn sampler_binding(slot: u32) -> u32 {
        SAMPLER_BINDING_BASE + slot
    }

    pub use crate::rendering::effects::constants::{
        DRAW_CONSTANTS_STRIDE, DRAW_ROWS, DRAW_UNIFORM_SIZE, FRAME_ROWS, FRAME_UNIFORM_SIZE,
    };
}

#[derive(Debug, thiserror::Error)]
pub enum EffectShaderError {
    #[error("fatal: {0}")]
    Load(#[from] ShaderLoadError),
    #[error(
        "fatal: {name:?} is not an effect shader name (`effect_vs_` or `effect_ps_` and 8 hex digits)"
    )]
    BadName { name: String },
    #[error(
        "fatal: vertex shader {shader:?} reads {semantic}{index} (register v{register}), \
         which {layout:?} vertices do not provide"
    )]
    UnsupportedVertexInput {
        shader: String,
        register: u32,
        semantic: String,
        index: u32,
        layout: VertexLayoutKind,
    },
    #[error(
        "fatal: vertex shader {shader:?} reads register v{register} through several signature elements"
    )]
    PackedVertexInput { shader: String, register: u32 },
    #[error("fatal: vertex shader {shader:?} writes no SV_Position")]
    NoPosition { shader: String },
    #[error(
        "fatal: pixel shader {pixel:?} reads {semantic}{index} (components {mask:#x}) \
         which vertex shader {vertex:?} does not write"
    )]
    PixelInputNotProduced {
        pixel: String,
        vertex: String,
        semantic: String,
        index: u32,
        mask: u8,
    },
    #[error(
        "fatal: pixel shader {pixel:?} reads register v{register} from several vertex output \
         registers of {vertex:?}"
    )]
    SplitPixelInput {
        pixel: String,
        vertex: String,
        register: u32,
    },
    #[error("fatal: pixel shader {shader:?} writes no SV_Target0")]
    NoColourOutput { shader: String },
    #[error("fatal: effect shader pair {vertex:?} + {pixel:?} is invalid:\n{message}")]
    Invalid {
        vertex: String,
        pixel: String,
        message: String,
    },
}

/// The vertex and pixel shader of one effect permutation, as numbers of the names
/// `effect_vs_<vs:x8>` and `effect_ps_<ps:x8>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EffectDescriptor {
    pub vs: u32,
    pub ps: u32,
}

impl EffectDescriptor {
    pub fn from_names(vs: &str, ps: &str) -> Result<Self, EffectShaderError> {
        Ok(Self {
            vs: parse_descriptor(vs, VERTEX_PREFIX)?,
            ps: parse_descriptor(ps, PIXEL_PREFIX)?,
        })
    }

    pub fn from_material(material: &EffectMaterial) -> Result<Self, EffectShaderError> {
        Self::from_names(&material.vs, &material.ps)
    }

    pub fn vs_name(&self) -> String {
        format!("{VERTEX_PREFIX}{:08x}", self.vs)
    }

    pub fn ps_name(&self) -> String {
        format!("{PIXEL_PREFIX}{:08x}", self.ps)
    }

    /// The permutation is the decal variant, which draws with a depth bias.
    pub fn is_decal(&self) -> bool {
        self.ps & DECAL_BIT != 0
    }
}

fn parse_descriptor(name: &str, prefix: &str) -> Result<u32, EffectShaderError> {
    let bad = || EffectShaderError::BadName {
        name: name.to_owned(),
    };
    name.strip_prefix(prefix)
        .and_then(parse_descriptor_digits)
        .ok_or_else(bad)
}

/// Per-vertex data of an effect mesh: what the vertex attributes of an effect shape carry.
/// The glue derives the packed encodings the shaders expect (normal, tangent and bitangent
/// in 0..1) from these.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct EffectVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub normal: [f32; 3],
    /// Tangent in `xyz`, handedness in `w` (the bitangent is `cross(normal, tangent) * w`).
    pub tangent: [f32; 4],
    /// Vertex colour exactly as stored (gamma-encoded bytes, read as `0..1`).
    pub colour: [u8; 4],
}

/// Per-vertex data of a particle quad corner.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ParticleVertex {
    /// Simulation-space position.
    pub position: [f32; 3],
    /// Index of the sub-texture rectangle, an integer held as a float (0 without sub-textures).
    pub sub_texture: f32,
    /// Quad corner in `0..1`.
    pub uv: [f32; 2],
    /// Per-particle distortion amount (neutral 1.0).
    pub distortion: f32,
    /// Particle colour in the same convention as [`EffectVertex::colour`].
    pub colour: [u8; 4],
}

/// The vertex buffer layouts the effect glue can feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VertexLayoutKind {
    /// [`EffectVertex`]
    Mesh,
    /// [`ParticleVertex`]
    Particle,
}

static MESH_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x3,
    1 => Float32x2,
    2 => Float32x3,
    3 => Float32x4,
    4 => Unorm8x4,
];

static PARTICLE_ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
    0 => Float32x4,
    1 => Float32x2,
    2 => Float32,
    3 => Unorm8x4,
];

const MESH_VERTEX_INPUT: &str = "
struct EffectVertexIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) tangent: vec4<f32>,
    @location(4) colour: vec4<f32>,
}

fn effect_bitangent(vin: EffectVertexIn) -> vec3<f32> {
    return cross(vin.normal, vin.tangent.xyz) * vin.tangent.w;
}
";

const PARTICLE_VERTEX_INPUT: &str = "
struct EffectVertexIn {
    @location(0) position: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) distortion: f32,
    @location(3) colour: vec4<f32>,
}
";

const COMPONENT_W: u8 = 0b1000;

impl VertexLayoutKind {
    pub fn buffer_layout(self) -> wgpu::VertexBufferLayout<'static> {
        let (array_stride, attributes): (usize, &'static [wgpu::VertexAttribute]) = match self {
            Self::Mesh => (std::mem::size_of::<EffectVertex>(), &MESH_ATTRIBUTES),
            Self::Particle => (std::mem::size_of::<ParticleVertex>(), &PARTICLE_ATTRIBUTES),
        };
        wgpu::VertexBufferLayout {
            array_stride: array_stride as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes,
        }
    }

    fn input_declarations(self) -> &'static str {
        match self {
            Self::Mesh => MESH_VERTEX_INPUT,
            Self::Particle => PARTICLE_VERTEX_INPUT,
        }
    }

    /// The WGSL `vec4<f32>` expression over `vin` that fills an input register of the
    /// translated vertex function with the attribute of `semantic`; `mask` holds the
    /// components the function declares.
    fn attribute_expression(self, semantic: &str, index: u32, mask: u8) -> Option<String> {
        let semantic = semantic.to_ascii_uppercase();
        let expression = match (self, semantic.as_str(), index) {
            (Self::Mesh, "POSITION", 0) if mask & COMPONENT_W != 0 => {
                "vec4<f32>(vin.position, effect_bitangent(vin).x)"
            }
            (Self::Mesh, "POSITION", 0) => "vec4<f32>(vin.position, 1.0)",
            (Self::Mesh, "NORMAL", 0) => {
                "vec4<f32>(vin.normal * 0.5 + 0.5, effect_bitangent(vin).y * 0.5 + 0.5)"
            }
            (Self::Mesh, "BINORMAL", 0) => {
                "vec4<f32>(vin.tangent.xyz * 0.5 + 0.5, effect_bitangent(vin).z * 0.5 + 0.5)"
            }
            (Self::Particle, "POSITION", 0) => "vin.position",
            (Self::Particle, "TEXCOORD", 2) => "vec4<f32>(vin.distortion, 0.0, 0.0, 0.0)",
            (_, "TEXCOORD", 0) => "vec4<f32>(vin.uv, 0.0, 0.0)",
            (_, "COLOR", 0) => "vin.colour",
            _ => return None,
        };
        Some(expression.to_owned())
    }
}

/// Composes the module of a vertex/pixel pair: prelude, wrapper, both translated functions and
/// the glue (`vs_main`, `fs_main`) for `vertex`.
pub fn compose_pair(
    descriptor: EffectDescriptor,
    vertex: VertexLayoutKind,
    translated: &TranslatedShaders,
) -> Result<String, EffectShaderError> {
    let vs_name = descriptor.vs_name();
    let ps_name = descriptor.ps_name();
    let vs_wgsl = translated.wgsl(&vs_name, ShaderStage::Vertex)?;
    let ps_wgsl = translated.wgsl(&ps_name, ShaderStage::Fragment)?;
    let vs_io = translated.interface(&vs_name, ShaderStage::Vertex)?;
    let ps_io = translated.interface(&ps_name, ShaderStage::Fragment)?;

    let glue = Glue {
        vs_name: &vs_name,
        ps_name: &ps_name,
        vertex_layout: vertex,
        vs_io,
        ps_io,
    }
    .generate()?;

    let mut module = String::from(MODULE_PRELUDE);
    module.push_str(EFFECT_WRAPPER);
    module.push_str(vs_wgsl);
    module.push_str(ps_wgsl);
    module.push_str(vertex.input_declarations());
    module.push_str(&glue);
    Ok(module)
}

/// Composes the wrapper with one translated effect function and nothing else; what the
/// startup validation checks for every shader of the family.
pub fn compose_standalone(
    name: &str,
    translated: &TranslatedShaders,
) -> Result<String, ShaderLoadError> {
    let stage = if name.starts_with(VERTEX_PREFIX) {
        ShaderStage::Vertex
    } else if name.starts_with(PIXEL_PREFIX) {
        ShaderStage::Fragment
    } else {
        return Err(ShaderLoadError::UnknownShader {
            name: name.to_owned(),
        });
    };
    let mut module = String::from(MODULE_PRELUDE);
    module.push_str(EFFECT_WRAPPER);
    module.push_str(translated.wgsl(name, stage)?);
    Ok(module)
}

/// Composes, validates with naga and compiles the module of a pair. A module naga rejects is
/// a fatal error naming both shaders.
pub fn compile_pair(
    device: &wgpu::Device,
    descriptor: EffectDescriptor,
    vertex: VertexLayoutKind,
    translated: &TranslatedShaders,
) -> Result<wgpu::ShaderModule, EffectShaderError> {
    let wgsl = compose_pair(descriptor, vertex, translated)?;
    startup::check_wgsl(&wgsl, startup::capabilities_of(device.features())).map_err(|fault| {
        let (kind, message) = match fault {
            WgslFault::Parse(message) => ("failed WGSL parsing", message),
            WgslFault::Validation(message) => ("failed WGSL validation", message),
        };
        EffectShaderError::Invalid {
            vertex: descriptor.vs_name(),
            pixel: descriptor.ps_name(),
            message: format!("{kind}:\n{message}"),
        }
    })?;
    Ok(device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("effect_shader"),
        source: wgpu::ShaderSource::Wgsl(wgsl.into()),
    }))
}

struct Glue<'a> {
    vs_name: &'a str,
    ps_name: &'a str,
    vertex_layout: VertexLayoutKind,
    vs_io: &'a ShaderInterface,
    ps_io: &'a ShaderInterface,
}

fn by_register(elements: &[IoElement]) -> BTreeMap<u32, Vec<&IoElement>> {
    let mut registers: BTreeMap<u32, Vec<&IoElement>> = BTreeMap::new();
    for element in elements {
        registers.entry(element.register).or_default().push(element);
    }
    registers
}

impl Glue<'_> {
    fn generate(&self) -> Result<String, EffectShaderError> {
        let vertex_assignments = self.vertex_input_assignments()?;
        let position_register = self
            .vs_io
            .outputs
            .iter()
            .find(|element| element.is_position())
            .map(|element| element.register)
            .ok_or_else(|| EffectShaderError::NoPosition {
                shader: self.vs_name.to_owned(),
            })?;
        let connections = self.pixel_connections()?;
        let colour_register = self.colour_register()?;

        let mut source =
            String::from("\nstruct EffectVarying {\n    @builtin(position) position: vec4<f32>,\n");
        for location in &connections.locations {
            let _ = writeln!(source, "    @location({location}) v{location}: vec4<f32>,");
        }
        source.push_str("}\n\n");

        let _ = writeln!(
            source,
            "@vertex\nfn {VERTEX_ENTRY}(vin: EffectVertexIn) -> EffectVarying {{"
        );
        let _ = writeln!(source, "    var i: {}_in;", self.vs_name);
        for assignment in &vertex_assignments {
            let _ = writeln!(source, "    {assignment}");
        }
        let _ = writeln!(source, "    let o = {}(i);", self.vs_name);
        source.push_str("    var out: EffectVarying;\n");
        let _ = writeln!(source, "    out.position = o.o{position_register};");
        for location in &connections.locations {
            let _ = writeln!(source, "    out.v{location} = o.o{location};");
        }
        source.push_str("    return out;\n}\n\n");

        let _ = writeln!(
            source,
            "@fragment\nfn {FRAGMENT_ENTRY}(f: EffectVarying) -> @location(0) vec4<f32> {{"
        );
        let _ = writeln!(source, "    var i: {}_in;", self.ps_name);
        for assignment in &connections.assignments {
            let _ = writeln!(source, "    {assignment}");
        }
        let _ = writeln!(
            source,
            "    return {}(i).o{colour_register};\n}}",
            self.ps_name
        );
        Ok(source)
    }

    fn vertex_input_assignments(&self) -> Result<Vec<String>, EffectShaderError> {
        by_register(&self.vs_io.inputs)
            .into_iter()
            .map(|(register, elements)| {
                let [element] = elements.as_slice() else {
                    return Err(EffectShaderError::PackedVertexInput {
                        shader: self.vs_name.to_owned(),
                        register,
                    });
                };
                let expression = self
                    .vertex_layout
                    .attribute_expression(&element.semantic, element.index, element.mask)
                    .ok_or_else(|| EffectShaderError::UnsupportedVertexInput {
                        shader: self.vs_name.to_owned(),
                        register,
                        semantic: element.semantic.clone(),
                        index: element.index,
                        layout: self.vertex_layout,
                    })?;
                Ok(format!("i.v{register} = {expression};"))
            })
            .collect()
    }

    fn pixel_connections(&self) -> Result<PixelConnections, EffectShaderError> {
        let mut connections = PixelConnections::default();
        for (register, elements) in by_register(&self.ps_io.inputs) {
            if elements.iter().all(|element| element.is_position()) {
                connections
                    .assignments
                    .push(format!("i.v{register} = f.position;"));
                continue;
            }
            let mut sources = BTreeSet::new();
            for element in &elements {
                sources.insert(self.vertex_output_for(element)?.register);
            }
            let (Some(&source), 1) = (sources.first(), sources.len()) else {
                return Err(EffectShaderError::SplitPixelInput {
                    pixel: self.ps_name.to_owned(),
                    vertex: self.vs_name.to_owned(),
                    register,
                });
            };
            connections.locations.insert(source);
            connections
                .assignments
                .push(format!("i.v{register} = f.v{source};"));
        }
        Ok(connections)
    }

    /// The vertex output that writes every component the pixel input element reads.
    fn vertex_output_for(&self, input: &IoElement) -> Result<&IoElement, EffectShaderError> {
        self.vs_io
            .outputs
            .iter()
            .find(|output| {
                !output.is_position()
                    && output.semantic.eq_ignore_ascii_case(&input.semantic)
                    && output.index == input.index
                    && output.mask & input.mask == input.mask
            })
            .ok_or_else(|| EffectShaderError::PixelInputNotProduced {
                pixel: self.ps_name.to_owned(),
                vertex: self.vs_name.to_owned(),
                semantic: input.semantic.clone(),
                index: input.index,
                mask: input.mask,
            })
    }

    fn colour_register(&self) -> Result<u32, EffectShaderError> {
        self.ps_io
            .outputs
            .iter()
            .find(|element| {
                element.semantic.eq_ignore_ascii_case("SV_TARGET") && element.index == 0
            })
            .map(|element| element.register)
            .ok_or_else(|| EffectShaderError::NoColourOutput {
                shader: self.ps_name.to_owned(),
            })
    }
}

#[derive(Default)]
struct PixelConnections {
    /// Vertex output registers the pixel function reads; they double as varying locations.
    locations: BTreeSet<u32>,
    assignments: Vec<String>,
}

/// Synthetic translated effect shaders for tests: same struct shapes, register globals and
/// interface metadata as the real family, no shader logic.
#[cfg(test)]
pub(crate) mod stubs {
    use super::*;

    pub const MESH_VS: &str = "effect_vs_00000005";
    pub const MESH_PS: &str = "effect_ps_00002005";
    /// The mesh permutation with the environment bit: reflects the `t5` cube along +X, scaled
    /// by the environment register `ps_cb1[3].x`.
    pub const ENV_PS: &str = "effect_ps_00082005";
    pub const PARTICLE_VS: &str = "effect_vs_00000085";
    pub const PARTICLE_PS: &str = "effect_ps_00000085";
    pub const SKINNED_VS: &str = "effect_vs_00000002";

    pub fn element(register: u32, semantic: &str, index: u32, mask: u8) -> IoElement {
        IoElement {
            register,
            semantic: semantic.to_owned(),
            index,
            mask,
            system_value: u32::from(semantic == "SV_POSITION"),
        }
    }

    const MESH_VS_WGSL: &str = "
struct effect_vs_00000005_in { v0: vec4<f32>, v1: vec4<f32>, v2: vec4<f32>, }
struct effect_vs_00000005_out { o0: vec4<f32>, o1: vec4<f32>, o2: vec4<f32>, }
fn effect_vs_00000005(i: effect_vs_00000005_in) -> effect_vs_00000005_out {
    var o: effect_vs_00000005_out;
    let p = vec4<f32>(i.v0.xyz, 1.0);
    o.o0 = vec4<f32>(
        dot(effect_draw.vs_cb2[0], p),
        dot(effect_draw.vs_cb2[1], p),
        dot(effect_draw.vs_cb2[2], p),
        dot(effect_draw.vs_cb2[3], p),
    );
    let uv = i.v1.xy * effect_draw.vs_cb1[0].zw + effect_draw.vs_cb1[0].xy;
    o.o1 = vec4<f32>(uv, effect_frame.vs_cb0[0].x * 0.0, 0.0);
    o.o2 = i.v2 + effect_frame.cb12[41] * 0.0;
    return o;
}
";

    const MESH_PS_WGSL: &str = "
struct effect_ps_00002005_in { v0: vec4<f32>, v1: vec4<f32>, v2: vec4<f32>, }
struct effect_ps_00002005_out { o0: vec4<f32>, }
fn effect_ps_00002005(i: effect_ps_00002005_in) -> effect_ps_00002005_out {
    var o: effect_ps_00002005_out;
    let texel = textureSample(effect_t0, effect_s0, i.v1.xy);
    let palette = textureSample(effect_t4, effect_s4, vec2<f32>(texel.r, 0.5));
    let depth = textureLoad(effect_t3, vec2<i32>(i.v0.xy), 0);
    o.o0 = palette * i.v2 * effect_draw.ps_cb1[0] * effect_frame.ps_cb0[0].y + vec4<f32>(depth * 0.0);
    return o;
}
";

    const ENV_PS_WGSL: &str = "
struct effect_ps_00082005_in { v0: vec4<f32>, v1: vec4<f32>, v2: vec4<f32>, }
struct effect_ps_00082005_out { o0: vec4<f32>, }
fn effect_ps_00082005(i: effect_ps_00082005_in) -> effect_ps_00082005_out {
    var o: effect_ps_00082005_out;
    let reflected = textureSample(effect_t5, effect_s5, vec3<f32>(1.0, 0.0, 0.0));
    o.o0 = vec4<f32>(reflected.rgb * effect_draw.ps_cb1[3].x, 1.0);
    return o;
}
";

    const PARTICLE_VS_WGSL: &str = "
struct effect_vs_00000085_in { v0: vec4<f32>, v1: vec4<f32>, v2: vec4<f32>, v3: vec4<f32>, }
struct effect_vs_00000085_out { o0: vec4<f32>, o1: vec4<f32>, o2: vec4<f32>, }
fn effect_vs_00000085(i: effect_vs_00000085_in) -> effect_vs_00000085_out {
    var o: effect_vs_00000085_out;
    let rect = effect_draw.vs_cb11[u32(i.v0.w)];
    let p = vec4<f32>(i.v0.xyz, 1.0);
    o.o0 = vec4<f32>(
        dot(effect_draw.vs_cb2[0], p),
        dot(effect_draw.vs_cb2[1], p),
        dot(effect_draw.vs_cb2[2], p),
        dot(effect_draw.vs_cb2[3], p),
    );
    o.o1 = vec4<f32>(i.v1.x * rect.y + rect.x, i.v1.y * rect.w + rect.z, i.v2.x, 0.0);
    o.o2 = i.v3;
    return o;
}
";

    const PARTICLE_PS_WGSL: &str = "
struct effect_ps_00000085_in { v1: vec4<f32>, v2: vec4<f32>, }
struct effect_ps_00000085_out { o0: vec4<f32>, }
fn effect_ps_00000085(i: effect_ps_00000085_in) -> effect_ps_00000085_out {
    var o: effect_ps_00000085_out;
    o.o0 = textureSample(effect_t0, effect_s0, i.v1.xy) * i.v2 * i.v1.z;
    if (o.o0.a < effect_draw.ps_cb2[14].x) { discard; }
    return o;
}
";

    /// A skinned variant: reads the bone matrices, which the wrapper only declares.
    const SKINNED_VS_WGSL: &str = "
struct effect_vs_00000002_in { v0: vec4<f32>, v1: vec4<f32>, }
struct effect_vs_00000002_out { o0: vec4<f32>, }
fn effect_vs_00000002(i: effect_vs_00000002_in) -> effect_vs_00000002_out {
    var o: effect_vs_00000002_out;
    let bone = effect_bones.cb10[u32(i.v1.x)];
    o.o0 = i.v0 + bone + vec4<f32>(f32(effect_vtx_t8[0]));
    return o;
}
";

    pub fn translated() -> TranslatedShaders {
        translated_with_mesh_vertex_wgsl(MESH_VS_WGSL)
    }

    /// The stubs with the mesh vertex function replaced by `wgsl` (same interface).
    pub fn translated_with_mesh_vertex_wgsl(wgsl: &str) -> TranslatedShaders {
        let io = |inputs: &[IoElement], outputs: &[IoElement]| ShaderInterface {
            inputs: inputs.to_vec(),
            outputs: outputs.to_vec(),
        };
        let target = element(0, "SV_TARGET", 0, 15);
        TranslatedShaders::from_sources([
            (MESH_VS, ShaderStage::Vertex, wgsl),
            (MESH_PS, ShaderStage::Fragment, MESH_PS_WGSL),
            (ENV_PS, ShaderStage::Fragment, ENV_PS_WGSL),
            (PARTICLE_VS, ShaderStage::Vertex, PARTICLE_VS_WGSL),
            (PARTICLE_PS, ShaderStage::Fragment, PARTICLE_PS_WGSL),
            (SKINNED_VS, ShaderStage::Vertex, SKINNED_VS_WGSL),
        ])
        .with_interface(
            MESH_VS,
            io(
                &[
                    element(0, "POSITION", 0, 7),
                    element(1, "TEXCOORD", 0, 3),
                    element(2, "COLOR", 0, 15),
                ],
                &[
                    element(0, "SV_POSITION", 0, 15),
                    element(1, "TEXCOORD", 0, 15),
                    element(2, "COLOR", 0, 15),
                ],
            ),
        )
        .with_interface(
            MESH_PS,
            io(
                &[
                    element(0, "SV_POSITION", 0, 3),
                    element(1, "TEXCOORD", 0, 3),
                    element(2, "COLOR", 0, 15),
                ],
                std::slice::from_ref(&target),
            ),
        )
        .with_interface(
            ENV_PS,
            io(
                &[
                    element(0, "SV_POSITION", 0, 3),
                    element(1, "TEXCOORD", 0, 3),
                    element(2, "COLOR", 0, 15),
                ],
                std::slice::from_ref(&target),
            ),
        )
        .with_interface(
            PARTICLE_VS,
            io(
                &[
                    element(0, "POSITION", 0, 15),
                    element(1, "TEXCOORD", 0, 3),
                    element(2, "TEXCOORD", 2, 1),
                    element(3, "COLOR", 0, 15),
                ],
                &[
                    element(0, "SV_POSITION", 0, 15),
                    element(1, "TEXCOORD", 0, 7),
                    element(2, "COLOR", 0, 15),
                ],
            ),
        )
        .with_interface(
            PARTICLE_PS,
            io(
                &[element(1, "TEXCOORD", 0, 7), element(2, "COLOR", 0, 15)],
                std::slice::from_ref(&target),
            ),
        )
        .with_interface(
            SKINNED_VS,
            io(
                &[
                    element(0, "POSITION", 0, 15),
                    element(1, "BLENDINDICES", 0, 15),
                ],
                &[element(0, "SV_POSITION", 0, 15)],
            ),
        )
    }

    pub fn mesh() -> EffectDescriptor {
        EffectDescriptor::from_names(MESH_VS, MESH_PS).unwrap()
    }

    pub fn particle() -> EffectDescriptor {
        EffectDescriptor::from_names(PARTICLE_VS, PARTICLE_PS).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::stubs::{self, element};
    use super::*;
    use crate::rendering::device::REQUIRED_FEATURES;
    use crate::rendering::shaders::startup::{capabilities_of, check_wgsl};

    fn validate(wgsl: &str) {
        if let Err(fault) = check_wgsl(wgsl, capabilities_of(REQUIRED_FEATURES)) {
            panic!("{fault:?}\n{wgsl}");
        }
    }

    fn entry_points(wgsl: &str) -> Vec<(String, naga::ShaderStage)> {
        naga::front::wgsl::parse_str(wgsl)
            .unwrap()
            .entry_points
            .iter()
            .map(|entry| (entry.name.clone(), entry.stage))
            .collect()
    }

    fn compose(descriptor: EffectDescriptor, vertex: VertexLayoutKind) -> String {
        compose_pair(descriptor, vertex, &stubs::translated()).unwrap()
    }

    #[test]
    fn descriptors_round_trip_through_the_shader_names() {
        let descriptor =
            EffectDescriptor::from_names("effect_vs_00a00015", "effect_ps_10a06015").unwrap();

        assert_eq!(
            descriptor,
            EffectDescriptor {
                vs: 0x00a0_0015,
                ps: 0x10a0_6015
            }
        );
        assert_eq!(descriptor.vs_name(), "effect_vs_00a00015");
        assert_eq!(descriptor.ps_name(), "effect_ps_10a06015");
        assert!(descriptor.is_decal());
        assert!(!stubs::mesh().is_decal());
    }

    #[test]
    fn malformed_shader_names_are_rejected() {
        for (vs, ps) in [
            ("effect_ps_00000005", "effect_ps_00000005"),
            ("effect_vs_00000005", "effect_vs_00000005"),
            ("effect_vs_5", "effect_ps_00000005"),
            ("effect_vs_0000000g", "effect_ps_00000005"),
            ("effect_vs_000000050", "effect_ps_00000005"),
            ("water_vs_basic", "effect_ps_00000005"),
            ("", ""),
        ] {
            let error = EffectDescriptor::from_names(vs, ps).unwrap_err();
            assert!(
                matches!(error, EffectShaderError::BadName { .. }),
                "{vs} {ps}"
            );
        }
    }

    #[test]
    fn the_mesh_pair_composes_to_a_valid_module_with_both_entries() {
        let wgsl = compose(stubs::mesh(), VertexLayoutKind::Mesh);

        validate(&wgsl);
        let entries = entry_points(&wgsl);
        assert!(entries.contains(&(VERTEX_ENTRY.to_owned(), naga::ShaderStage::Vertex)));
        assert!(entries.contains(&(FRAGMENT_ENTRY.to_owned(), naga::ShaderStage::Fragment)));
    }

    #[test]
    fn the_particle_pair_composes_to_a_valid_module() {
        validate(&compose(stubs::particle(), VertexLayoutKind::Particle));
    }

    #[test]
    fn the_module_starts_with_the_prelude() {
        let wgsl = compose(stubs::mesh(), VertexLayoutKind::Mesh);
        assert!(wgsl.starts_with(MODULE_PRELUDE));
        assert_eq!(wgsl.matches(MODULE_PRELUDE).count(), 1);
    }

    #[test]
    fn vertex_registers_are_filled_from_the_attribute_with_the_matching_semantic() {
        let mesh = compose(stubs::mesh(), VertexLayoutKind::Mesh);
        assert!(
            mesh.contains("i.v0 = vec4<f32>(vin.position, 1.0);"),
            "{mesh}"
        );
        assert!(mesh.contains("i.v1 = vec4<f32>(vin.uv, 0.0, 0.0);"));
        assert!(mesh.contains("i.v2 = vin.colour;"));

        let particle = compose(stubs::particle(), VertexLayoutKind::Particle);
        assert!(particle.contains("i.v0 = vin.position;"));
        assert!(particle.contains("i.v2 = vec4<f32>(vin.distortion, 0.0, 0.0, 0.0);"));
    }

    #[test]
    fn the_position_w_of_a_mesh_carries_the_bitangent_only_when_the_shader_declares_it() {
        let mut translated = stubs::translated();
        translated = translated.with_interface(
            stubs::MESH_VS,
            ShaderInterface {
                inputs: vec![
                    element(0, "POSITION", 0, 15),
                    element(1, "NORMAL", 0, 15),
                    element(2, "BINORMAL", 0, 15),
                ],
                outputs: vec![
                    element(0, "SV_POSITION", 0, 15),
                    element(1, "TEXCOORD", 0, 15),
                    element(2, "COLOR", 0, 15),
                ],
            },
        );

        let wgsl = compose_pair(stubs::mesh(), VertexLayoutKind::Mesh, &translated).unwrap();

        assert!(wgsl.contains("i.v0 = vec4<f32>(vin.position, effect_bitangent(vin).x);"));
        assert!(wgsl.contains(
            "i.v1 = vec4<f32>(vin.normal * 0.5 + 0.5, effect_bitangent(vin).y * 0.5 + 0.5);"
        ));
        assert!(wgsl.contains(
            "i.v2 = vec4<f32>(vin.tangent.xyz * 0.5 + 0.5, effect_bitangent(vin).z * 0.5 + 0.5);"
        ));
    }

    #[test]
    fn varyings_follow_the_vertex_output_register_that_writes_the_semantic() {
        let mut translated = stubs::translated();
        translated = translated.with_interface(
            stubs::PARTICLE_VS,
            ShaderInterface {
                inputs: vec![
                    element(0, "POSITION", 0, 15),
                    element(1, "TEXCOORD", 0, 3),
                    element(2, "TEXCOORD", 2, 1),
                    element(3, "COLOR", 0, 15),
                ],
                outputs: vec![
                    element(0, "SV_POSITION", 0, 15),
                    element(4, "TEXCOORD", 0, 7),
                    element(5, "COLOR", 0, 15),
                ],
            },
        );

        let wgsl = compose_pair(stubs::particle(), VertexLayoutKind::Particle, &translated);
        let wgsl = wgsl.unwrap();

        assert!(wgsl.contains("@location(4) v4: vec4<f32>,"));
        assert!(wgsl.contains("@location(5) v5: vec4<f32>,"));
        assert!(wgsl.contains("i.v1 = f.v4;"), "{wgsl}");
        assert!(wgsl.contains("i.v2 = f.v5;"));
        assert!(wgsl.contains("out.v4 = o.o4;"));
    }

    #[test]
    fn the_pixel_position_input_reads_the_builtin_position() {
        let wgsl = compose(stubs::mesh(), VertexLayoutKind::Mesh);
        assert!(wgsl.contains("i.v0 = f.position;"));
        assert!(wgsl.contains("out.position = o.o0;"));
        assert!(wgsl.contains("return effect_ps_00002005(i).o0;"));
    }

    #[test]
    fn a_vertex_attribute_the_layout_lacks_is_a_fatal_error() {
        let error = compose_pair(
            stubs::particle(),
            VertexLayoutKind::Mesh,
            &stubs::translated(),
        )
        .unwrap_err();

        assert!(
            matches!(
                &error,
                EffectShaderError::UnsupportedVertexInput { shader, semantic, index: 2, .. }
                    if shader == stubs::PARTICLE_VS && semantic == "TEXCOORD"
            ),
            "{error}"
        );
    }

    #[test]
    fn system_value_vertex_inputs_are_unsupported() {
        let translated = stubs::translated().with_interface(
            stubs::MESH_VS,
            ShaderInterface {
                inputs: vec![element(0, "SV_VertexID", 0, 1)],
                outputs: vec![element(0, "SV_POSITION", 0, 15)],
            },
        );
        let error = compose_pair(stubs::mesh(), VertexLayoutKind::Mesh, &translated).unwrap_err();
        assert!(matches!(
            error,
            EffectShaderError::UnsupportedVertexInput { .. }
        ));
    }

    #[test]
    fn a_register_shared_by_vertex_input_elements_is_rejected() {
        let translated = stubs::translated().with_interface(
            stubs::MESH_VS,
            ShaderInterface {
                inputs: vec![element(0, "POSITION", 0, 7), element(0, "TEXCOORD", 0, 8)],
                outputs: vec![element(0, "SV_POSITION", 0, 15)],
            },
        );
        let error = compose_pair(stubs::mesh(), VertexLayoutKind::Mesh, &translated).unwrap_err();
        assert!(matches!(
            error,
            EffectShaderError::PackedVertexInput { register: 0, .. }
        ));
    }

    #[test]
    fn a_pixel_input_the_vertex_shader_does_not_write_is_a_fatal_error() {
        let translated = stubs::translated().with_interface(
            stubs::MESH_PS,
            ShaderInterface {
                inputs: vec![element(1, "TEXCOORD", 3, 3)],
                outputs: vec![element(0, "SV_TARGET", 0, 15)],
            },
        );
        let error = compose_pair(stubs::mesh(), VertexLayoutKind::Mesh, &translated).unwrap_err();
        assert!(
            matches!(&error, EffectShaderError::PixelInputNotProduced { semantic, index: 3, .. } if semantic == "TEXCOORD"),
            "{error}"
        );
    }

    #[test]
    fn a_pixel_input_wider_than_the_vertex_output_is_a_fatal_error() {
        let translated = stubs::translated().with_interface(
            stubs::PARTICLE_PS,
            ShaderInterface {
                inputs: vec![element(1, "TEXCOORD", 0, 15)],
                outputs: vec![element(0, "SV_TARGET", 0, 15)],
            },
        );
        let error =
            compose_pair(stubs::particle(), VertexLayoutKind::Particle, &translated).unwrap_err();
        assert!(matches!(
            error,
            EffectShaderError::PixelInputNotProduced { mask: 15, .. }
        ));
    }

    #[test]
    fn a_pixel_register_fed_by_two_vertex_registers_is_rejected() {
        let translated = stubs::translated().with_interface(
            stubs::MESH_PS,
            ShaderInterface {
                inputs: vec![element(1, "TEXCOORD", 0, 3), element(1, "COLOR", 0, 15)],
                outputs: vec![element(0, "SV_TARGET", 0, 15)],
            },
        );
        let error = compose_pair(stubs::mesh(), VertexLayoutKind::Mesh, &translated).unwrap_err();
        assert!(matches!(
            error,
            EffectShaderError::SplitPixelInput { register: 1, .. }
        ));
    }

    #[test]
    fn a_pixel_register_with_two_elements_from_one_vertex_register_is_connected() {
        let translated = stubs::translated()
            .with_interface(
                stubs::MESH_VS,
                ShaderInterface {
                    inputs: vec![element(0, "POSITION", 0, 7)],
                    outputs: vec![
                        element(0, "SV_POSITION", 0, 15),
                        element(1, "TEXCOORD", 0, 3),
                        element(1, "TEXCOORD", 1, 12),
                    ],
                },
            )
            .with_interface(
                stubs::MESH_PS,
                ShaderInterface {
                    inputs: vec![element(3, "TEXCOORD", 0, 3), element(3, "TEXCOORD", 1, 12)],
                    outputs: vec![element(0, "SV_TARGET", 0, 15)],
                },
            );

        let wgsl = compose_pair(stubs::mesh(), VertexLayoutKind::Mesh, &translated).unwrap();

        assert!(wgsl.contains("i.v3 = f.v1;"), "{wgsl}");
        assert_eq!(wgsl.matches("@location(1) v1").count(), 1);
    }

    #[test]
    fn a_vertex_shader_without_position_output_is_rejected() {
        let translated = stubs::translated().with_interface(
            stubs::MESH_VS,
            ShaderInterface {
                inputs: vec![element(0, "POSITION", 0, 7)],
                outputs: vec![element(1, "TEXCOORD", 0, 15), element(2, "COLOR", 0, 15)],
            },
        );
        let error = compose_pair(stubs::mesh(), VertexLayoutKind::Mesh, &translated).unwrap_err();
        assert!(matches!(error, EffectShaderError::NoPosition { .. }));
    }

    #[test]
    fn a_pixel_shader_without_colour_target_is_rejected() {
        let translated = stubs::translated().with_interface(
            stubs::MESH_PS,
            ShaderInterface {
                inputs: vec![element(1, "TEXCOORD", 0, 3)],
                outputs: vec![element(0, "SV_TARGET", 1, 15)],
            },
        );
        let error = compose_pair(stubs::mesh(), VertexLayoutKind::Mesh, &translated).unwrap_err();
        assert!(matches!(error, EffectShaderError::NoColourOutput { .. }));
    }

    #[test]
    fn a_missing_translated_shader_is_a_fatal_load_error_naming_it() {
        let only_vertex =
            TranslatedShaders::from_sources([(stubs::MESH_VS, ShaderStage::Vertex, "")]);
        let error = compose_pair(stubs::mesh(), VertexLayoutKind::Mesh, &only_vertex).unwrap_err();
        assert!(
            matches!(
                &error,
                EffectShaderError::Load(ShaderLoadError::UnknownShader { name }) if name == stubs::MESH_PS
            ),
            "{error}"
        );
        assert!(error.to_string().contains("shader-translate"));
    }

    #[test]
    fn a_function_of_the_wrong_stage_is_rejected() {
        let swapped = TranslatedShaders::from_sources([
            (stubs::MESH_VS, ShaderStage::Fragment, ""),
            (stubs::MESH_PS, ShaderStage::Fragment, ""),
        ]);
        let error = compose_pair(stubs::mesh(), VertexLayoutKind::Mesh, &swapped).unwrap_err();
        assert!(matches!(
            error,
            EffectShaderError::Load(ShaderLoadError::StageMismatch { .. })
        ));
    }

    #[test]
    fn every_stub_validates_on_its_own_with_the_wrapper() {
        let translated = stubs::translated();
        for name in [
            stubs::MESH_VS,
            stubs::MESH_PS,
            stubs::PARTICLE_VS,
            stubs::PARTICLE_PS,
            stubs::SKINNED_VS,
        ] {
            validate(&compose_standalone(name, &translated).unwrap());
        }
    }

    #[test]
    fn standalone_composition_rejects_names_outside_the_family() {
        let translated =
            TranslatedShaders::from_sources([("water_vs_basic", ShaderStage::Vertex, "")]);
        let error = compose_standalone("water_vs_basic", &translated).unwrap_err();
        assert!(matches!(error, ShaderLoadError::UnknownShader { .. }));
    }

    #[test]
    fn the_wrapper_declares_exactly_the_documented_bindings() {
        use bindings::*;
        let module = naga::front::wgsl::parse_str(EFFECT_WRAPPER).unwrap();
        let declared: BTreeSet<(u32, u32)> = module
            .global_variables
            .iter()
            .filter_map(|(_, variable)| variable.binding.map(|b| (b.group, b.binding)))
            .collect();
        let mut documented = BTreeSet::from([
            (SCENE_GROUP, SCENE_DEPTH_BINDING),
            (FRAME_GROUP, FRAME_BINDING),
            (DRAW_GROUP, DRAW_BINDING),
            // the bone matrices and the four vertex-pulling buffers of the never-drawn variants
            (DRAW_GROUP, 1),
            (DRAW_GROUP, 2),
            (DRAW_GROUP, 3),
            (DRAW_GROUP, 4),
            (DRAW_GROUP, 5),
            // the unused s3 declared for the translated shaders' register numbering
            (MATERIAL_GROUP, sampler_binding(3)),
        ]);
        for slot in MATERIAL_TEXTURE_SLOTS {
            documented.insert((MATERIAL_GROUP, texture_binding(slot)));
            documented.insert((MATERIAL_GROUP, sampler_binding(slot)));
        }
        assert_eq!(declared, documented);
    }

    #[test]
    fn the_uniform_sizes_match_the_wrapper_structs() {
        let module = naga::front::wgsl::parse_str(EFFECT_WRAPPER).unwrap();
        let size_of = |name: &str| {
            module
                .types
                .iter()
                .find_map(|(_, ty)| match (&ty.name, &ty.inner) {
                    (Some(n), naga::TypeInner::Struct { span, .. }) if n == name => Some(*span),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{name} is not declared"))
        };
        assert_eq!(
            u64::from(size_of("EffectFrame")),
            bindings::FRAME_UNIFORM_SIZE
        );
        assert_eq!(
            u64::from(size_of("EffectDraw")),
            bindings::DRAW_UNIFORM_SIZE
        );
        const { assert!(bindings::DRAW_CONSTANTS_STRIDE.is_multiple_of(256)) };
        const { assert!(bindings::DRAW_CONSTANTS_STRIDE >= bindings::DRAW_UNIFORM_SIZE) };
    }

    #[test]
    fn the_vertex_layouts_match_the_vertex_structs() {
        let mesh = VertexLayoutKind::Mesh.buffer_layout();
        assert_eq!(mesh.array_stride, 52);
        assert_eq!(
            mesh.array_stride as usize,
            std::mem::size_of::<EffectVertex>()
        );
        let offsets: Vec<_> = mesh.attributes.iter().map(|a| a.offset).collect();
        assert_eq!(offsets, [0, 12, 20, 32, 48]);

        let particle = VertexLayoutKind::Particle.buffer_layout();
        assert_eq!(particle.array_stride, 32);
        assert_eq!(
            particle.array_stride as usize,
            std::mem::size_of::<ParticleVertex>()
        );
        let offsets: Vec<_> = particle.attributes.iter().map(|a| a.offset).collect();
        assert_eq!(offsets, [0, 16, 24, 28]);
    }

    #[test]
    fn the_vertex_entry_locations_match_the_layout_attributes() {
        for (descriptor, kind) in [
            (stubs::mesh(), VertexLayoutKind::Mesh),
            (stubs::particle(), VertexLayoutKind::Particle),
        ] {
            let module = naga::front::wgsl::parse_str(&compose(descriptor, kind)).unwrap();
            let entry = module
                .entry_points
                .iter()
                .find(|entry| entry.name == VERTEX_ENTRY)
                .unwrap();
            let argument = &entry.function.arguments[0];
            let naga::TypeInner::Struct { members, .. } = &module.types[argument.ty].inner else {
                panic!("the vertex input is a struct");
            };
            let locations: Vec<u32> = members
                .iter()
                .map(|member| match member.binding {
                    Some(naga::Binding::Location { location, .. }) => location,
                    ref other => panic!("{other:?}"),
                })
                .collect();
            let attributes: Vec<u32> = kind
                .buffer_layout()
                .attributes
                .iter()
                .map(|attribute| attribute.shader_location)
                .collect();
            assert_eq!(locations, attributes, "{kind:?}");
        }
    }
}
