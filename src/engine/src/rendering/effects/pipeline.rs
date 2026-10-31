//! Bind group layouts and the lazily created, cached render pipelines of the effect shaders.
//!
//! A pipeline is created the first time a draw asks for its key and then reused; the shader
//! module of a pair is shared by every pipeline of the pair that differs only in blend, cull or
//! depth state. A missing or invalid translated shader is a fatal [`EffectShaderError`], never
//! a fallback.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};

use super::shader::{
    EffectDescriptor, EffectShaderError, FRAGMENT_ENTRY, VERTEX_ENTRY, VertexLayoutKind, bindings,
    compile_pair,
};
use crate::assets::types::effect_material::{
    BlendFactor as MaterialFactor, BlendMode, EffectMaterial,
};
use crate::rendering::{
    post::hdr_target::HDR_FORMAT,
    shaders::{startup, translated::TranslatedShaders},
    texture::Texture,
};

/// Source and destination colour factor of a blended permutation.
pub type BlendFactors = (wgpu::BlendFactor, wgpu::BlendFactor);

/// The state a draw's pipeline depends on besides its shaders and vertex layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EffectPipelineKey {
    pub descriptor: EffectDescriptor,
    pub vertex: VertexLayoutKind,
    /// `None` writes the colour unblended.
    pub blend: Option<BlendFactors>,
    pub double_sided: bool,
    pub depth_test: bool,
    pub depth_write: bool,
}

impl EffectPipelineKey {
    pub fn from_material(
        material: &EffectMaterial,
        vertex: VertexLayoutKind,
    ) -> Result<Self, EffectShaderError> {
        Ok(Self {
            descriptor: EffectDescriptor::from_material(material)?,
            vertex,
            blend: blend_factors(&material.blend),
            double_sided: material.double_sided,
            depth_test: material.z_test,
            depth_write: material.z_write,
        })
    }
}

/// The factors of a material's alpha property; a mode without stored factors uses the factors
/// that mode is defined by.
fn blend_factors(blend: &crate::assets::types::effect_material::Blend) -> Option<BlendFactors> {
    let (default_src, default_dst) = match blend.mode {
        BlendMode::None => return None,
        BlendMode::Add => (wgpu::BlendFactor::SrcAlpha, wgpu::BlendFactor::One),
        BlendMode::Alpha => (
            wgpu::BlendFactor::SrcAlpha,
            wgpu::BlendFactor::OneMinusSrcAlpha,
        ),
        BlendMode::Mult => (wgpu::BlendFactor::Dst, wgpu::BlendFactor::Zero),
    };
    Some((
        blend.src.map_or(default_src, wgpu_factor),
        blend.dst.map_or(default_dst, wgpu_factor),
    ))
}

pub(crate) fn wgpu_factor(factor: MaterialFactor) -> wgpu::BlendFactor {
    match factor {
        MaterialFactor::One => wgpu::BlendFactor::One,
        MaterialFactor::Zero => wgpu::BlendFactor::Zero,
        MaterialFactor::SrcColor => wgpu::BlendFactor::Src,
        MaterialFactor::InvSrcColor => wgpu::BlendFactor::OneMinusSrc,
        MaterialFactor::DstColor => wgpu::BlendFactor::Dst,
        MaterialFactor::InvDstColor => wgpu::BlendFactor::OneMinusDst,
        MaterialFactor::SrcAlpha => wgpu::BlendFactor::SrcAlpha,
        MaterialFactor::InvSrcAlpha => wgpu::BlendFactor::OneMinusSrcAlpha,
        MaterialFactor::DstAlpha => wgpu::BlendFactor::DstAlpha,
        MaterialFactor::InvDstAlpha => wgpu::BlendFactor::OneMinusDstAlpha,
        MaterialFactor::SrcAlphaSaturate => wgpu::BlendFactor::SrcAlphaSaturated,
    }
}

/// The destination alpha is left alone: effects tint the colour of the scene only.
const KEEP_DESTINATION_ALPHA: wgpu::BlendComponent = wgpu::BlendComponent {
    src_factor: wgpu::BlendFactor::Zero,
    dst_factor: wgpu::BlendFactor::One,
    operation: wgpu::BlendOperation::Add,
};

pub(crate) fn blend_state((src_factor, dst_factor): BlendFactors) -> wgpu::BlendState {
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor,
            dst_factor,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: KEEP_DESTINATION_ALPHA,
    }
}

/// Reverse-Z: larger depth is closer, so a positive bias pulls a decal towards the camera and
/// lets it win the `GreaterEqual` test against the surface it lies on.
const DECAL_DEPTH_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: 2,
    slope_scale: 2.0,
    clamp: 0.0,
};

fn texture_entry(slot: u32) -> wgpu::BindGroupLayoutEntry {
    let view_dimension = if slot == bindings::CUBE_TEXTURE_SLOT {
        wgpu::TextureViewDimension::Cube
    } else {
        wgpu::TextureViewDimension::D2
    };
    wgpu::BindGroupLayoutEntry {
        binding: bindings::texture_binding(slot),
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(slot: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: bindings::sampler_binding(slot),
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn uniform_entry(binding: u32, dynamic: bool, size: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: dynamic,
            min_binding_size: NonZeroU64::new(size),
        },
        count: None,
    }
}

/// The bind group layouts of `effect_wrapper.wgsl` and the pipeline layout that joins them
/// with the scene copy's layout (group 0).
pub struct EffectLayouts {
    /// Group 1: the per-frame constants.
    pub frame: wgpu::BindGroupLayout,
    /// Group 2: texture slots `t0 t1 t2 t4..t8` and samplers `s0 s1 s2 s4..s8`.
    pub material: wgpu::BindGroupLayout,
    /// Group 3: the per-draw constants at a dynamic offset of
    /// [`bindings::DRAW_CONSTANTS_STRIDE`] bytes.
    pub draw: wgpu::BindGroupLayout,
    pub pipeline: wgpu::PipelineLayout,
}

impl EffectLayouts {
    pub fn new(device: &wgpu::Device, scene_layout: &wgpu::BindGroupLayout) -> Self {
        let frame = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("effect_frame_layout"),
            entries: &[uniform_entry(
                bindings::FRAME_BINDING,
                false,
                bindings::FRAME_UNIFORM_SIZE,
            )],
        });
        let material_entries: Vec<_> = bindings::MATERIAL_TEXTURE_SLOTS
            .iter()
            .flat_map(|&slot| [texture_entry(slot), sampler_entry(slot)])
            .collect();
        let material = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("effect_material_layout"),
            entries: &material_entries,
        });
        let draw = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("effect_draw_layout"),
            entries: &[uniform_entry(
                bindings::DRAW_BINDING,
                true,
                bindings::DRAW_UNIFORM_SIZE,
            )],
        });
        let pipeline = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("effect_pipeline_layout"),
            bind_group_layouts: &[
                Some(scene_layout),
                Some(&frame),
                Some(&material),
                Some(&draw),
            ],
            immediate_size: 0,
        });
        Self {
            frame,
            material,
            draw,
            pipeline,
        }
    }
}

type ModuleKey = (EffectDescriptor, VertexLayoutKind);

/// Builds effect pipelines from the translated shaders. Shared with the warm-up worker thread:
/// everything it holds is thread safe and the module cache sits behind a lock.
pub struct PipelineCompiler {
    device: wgpu::Device,
    translated: &'static TranslatedShaders,
    layout: wgpu::PipelineLayout,
    modules: Mutex<HashMap<ModuleKey, wgpu::ShaderModule>>,
}

impl PipelineCompiler {
    /// The pipeline of `key`; the shader module of its pair is composed once and shared.
    pub fn build(
        &self,
        key: &EffectPipelineKey,
    ) -> Result<wgpu::RenderPipeline, EffectShaderError> {
        let module = {
            let mut modules = self
                .modules
                .lock()
                .expect("the module cache lock is not poisoned");
            match modules.entry((key.descriptor, key.vertex)) {
                Entry::Occupied(module) => module.get().clone(),
                Entry::Vacant(module) => module
                    .insert(compile_pair(
                        &self.device,
                        key.descriptor,
                        key.vertex,
                        self.translated,
                    )?)
                    .clone(),
            }
        };
        Ok(create_pipeline(&self.device, &module, &self.layout, key))
    }

    /// Shader modules composed so far (one per vertex/pixel pair and vertex layout).
    pub fn module_count(&self) -> usize {
        self.modules
            .lock()
            .expect("the module cache lock is not poisoned")
            .len()
    }
}

/// Effect pipelines of one device. [`Self::pipeline`] creates one on the calling thread; the
/// effect stage instead has them built by a worker and hands the results to [`Self::insert`].
pub struct EffectPipelines {
    layouts: EffectLayouts,
    compiler: Arc<PipelineCompiler>,
    pipelines: HashMap<EffectPipelineKey, wgpu::RenderPipeline>,
}

impl EffectPipelines {
    /// Pipelines over the translated shaders loaded at startup; `scene_layout` is the scene
    /// copy's bind group layout.
    pub fn new(device: &wgpu::Device, scene_layout: &wgpu::BindGroupLayout) -> Self {
        let translated = startup::translated()
            .expect("translated shaders are loaded at startup before any pipeline is built");
        Self::with_shaders(device, scene_layout, translated)
    }

    pub fn with_shaders(
        device: &wgpu::Device,
        scene_layout: &wgpu::BindGroupLayout,
        translated: &'static TranslatedShaders,
    ) -> Self {
        let layouts = EffectLayouts::new(device, scene_layout);
        Self {
            compiler: Arc::new(PipelineCompiler {
                device: device.clone(),
                translated,
                layout: layouts.pipeline.clone(),
                modules: Mutex::new(HashMap::new()),
            }),
            layouts,
            pipelines: HashMap::new(),
        }
    }

    pub fn layouts(&self) -> &EffectLayouts {
        &self.layouts
    }

    pub fn compiler(&self) -> Arc<PipelineCompiler> {
        Arc::clone(&self.compiler)
    }

    /// The pipeline of `key`, created on this thread if this is the first request for it.
    pub fn pipeline(
        &mut self,
        key: &EffectPipelineKey,
    ) -> Result<&wgpu::RenderPipeline, EffectShaderError> {
        match self.pipelines.entry(*key) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => Ok(entry.insert(self.compiler.build(key)?)),
        }
    }

    /// Stores a pipeline another thread built for `key`.
    pub fn insert(&mut self, key: EffectPipelineKey, pipeline: wgpu::RenderPipeline) {
        self.pipelines.insert(key, pipeline);
    }

    /// The pipeline of `key` when it was created before.
    pub fn existing(&self, key: &EffectPipelineKey) -> Option<&wgpu::RenderPipeline> {
        self.pipelines.get(key)
    }

    /// Pipelines created so far.
    pub fn pipeline_count(&self) -> usize {
        self.pipelines.len()
    }

    /// Shader modules composed so far (one per vertex/pixel pair and vertex layout).
    pub fn module_count(&self) -> usize {
        self.compiler.module_count()
    }
}

fn create_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    key: &EffectPipelineKey,
) -> wgpu::RenderPipeline {
    let vertex_layout = key.vertex.buffer_layout();
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("effect_pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(VERTEX_ENTRY),
            buffers: &[Some(vertex_layout)],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(FRAGMENT_ENTRY),
            targets: &[Some(wgpu::ColorTargetState {
                format: HDR_FORMAT,
                blend: key.blend.map(blend_state),
                write_mask: wgpu::ColorWrites::COLOR,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: (!key.double_sided).then_some(wgpu::Face::Back),
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: Texture::DEPTH_FORMAT,
            depth_write_enabled: Some(key.depth_write),
            depth_compare: Some(if key.depth_test {
                wgpu::CompareFunction::GreaterEqual
            } else {
                wgpu::CompareFunction::Always
            }),
            stencil: wgpu::StencilState::default(),
            bias: if key.descriptor.is_decal() {
                DECAL_DEPTH_BIAS
            } else {
                wgpu::DepthBiasState::default()
            },
        }),
        multisample: wgpu::MultisampleState {
            count: 1,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::rendering::{effects::shader::stubs, gpu_test::test_device, scene_copy::SceneCopy};

    fn material(blend: serde_json::Value) -> EffectMaterial {
        let mut document = json!({
            "v": 1, "vs": stubs::MESH_VS, "ps": stubs::MESH_PS,
            "flags1": 0, "flags2": 0, "blend": null,
            "z_test": true, "z_write": false, "double_sided": true,
            "clamp": "wrap_s_wrap_t", "textures": {},
            "base_color": [1.0, 1.0, 1.0, 1.0], "base_color_scale": 1.0,
            "falloff": { "start_angle": 1.0, "stop_angle": 1.0,
                         "start_opacity": 1.0, "stop_opacity": 1.0 },
            "soft_falloff_depth": 0.0, "uv_offset": [0.0, 0.0], "uv_scale": [1.0, 1.0],
            "lighting_influence": 0.0, "env_map_scale": 1.0,
            "emittance_color": [0.0, 0.0, 0.0], "refraction_power": 0.0, "alpha": 1.0
        });
        document["blend"] = blend;
        serde_json::from_value(document).unwrap()
    }

    fn key_of(blend: serde_json::Value) -> EffectPipelineKey {
        EffectPipelineKey::from_material(&material(blend), VertexLayoutKind::Mesh).unwrap()
    }

    #[test]
    fn the_key_takes_shaders_and_state_from_the_material() {
        let key = key_of(json!({ "mode": "add", "src": "src_alpha", "dst": "one" }));

        assert_eq!(key.descriptor, stubs::mesh());
        assert_eq!(
            key.blend,
            Some((wgpu::BlendFactor::SrcAlpha, wgpu::BlendFactor::One))
        );
        assert_eq!(
            (key.double_sided, key.depth_test, key.depth_write),
            (true, true, false)
        );
    }

    #[test]
    fn every_stored_factor_maps_to_its_wgpu_factor() {
        use MaterialFactor::*;
        let expected = [
            (One, wgpu::BlendFactor::One),
            (Zero, wgpu::BlendFactor::Zero),
            (SrcColor, wgpu::BlendFactor::Src),
            (InvSrcColor, wgpu::BlendFactor::OneMinusSrc),
            (DstColor, wgpu::BlendFactor::Dst),
            (InvDstColor, wgpu::BlendFactor::OneMinusDst),
            (SrcAlpha, wgpu::BlendFactor::SrcAlpha),
            (InvSrcAlpha, wgpu::BlendFactor::OneMinusSrcAlpha),
            (DstAlpha, wgpu::BlendFactor::DstAlpha),
            (InvDstAlpha, wgpu::BlendFactor::OneMinusDstAlpha),
            (SrcAlphaSaturate, wgpu::BlendFactor::SrcAlphaSaturated),
        ];
        for (stored, wanted) in expected {
            assert_eq!(wgpu_factor(stored), wanted, "{stored:?}");
        }
    }

    #[test]
    fn stored_factors_win_over_the_defaults_of_the_mode() {
        let key = key_of(json!({ "mode": "mult", "src": "zero", "dst": "src_color" }));
        assert_eq!(
            key.blend,
            Some((wgpu::BlendFactor::Zero, wgpu::BlendFactor::Src))
        );
    }

    #[test]
    fn a_mode_without_stored_factors_uses_its_defining_factors() {
        let blend = |mode: &str| key_of(json!({ "mode": mode })).blend;
        assert_eq!(
            blend("add"),
            Some((wgpu::BlendFactor::SrcAlpha, wgpu::BlendFactor::One))
        );
        assert_eq!(
            blend("alpha"),
            Some((
                wgpu::BlendFactor::SrcAlpha,
                wgpu::BlendFactor::OneMinusSrcAlpha
            ))
        );
        assert_eq!(
            blend("mult"),
            Some((wgpu::BlendFactor::Dst, wgpu::BlendFactor::Zero))
        );
        assert_eq!(blend("none"), None);
    }

    #[test]
    fn a_none_mode_ignores_stored_factors() {
        assert_eq!(
            key_of(json!({ "mode": "none", "src": "one", "dst": "one" })).blend,
            None
        );
    }

    #[test]
    fn the_blend_state_leaves_the_destination_alpha_alone() {
        let state = blend_state((wgpu::BlendFactor::SrcAlpha, wgpu::BlendFactor::One));
        assert_eq!(state.color.src_factor, wgpu::BlendFactor::SrcAlpha);
        assert_eq!(state.color.dst_factor, wgpu::BlendFactor::One);
        assert_eq!(state.alpha, KEEP_DESTINATION_ALPHA);
    }

    #[test]
    fn a_malformed_shader_name_in_the_material_is_a_fatal_error() {
        let mut broken = material(json!({ "mode": "none" }));
        broken.vs = "water_vs_basic".to_owned();
        let error = EffectPipelineKey::from_material(&broken, VertexLayoutKind::Mesh).unwrap_err();
        assert!(matches!(error, EffectShaderError::BadName { .. }));
    }

    #[test]
    fn keys_differing_in_any_state_are_distinct() {
        let base = key_of(json!({ "mode": "add" }));
        let variants = [
            EffectPipelineKey {
                double_sided: !base.double_sided,
                ..base
            },
            EffectPipelineKey {
                depth_test: !base.depth_test,
                ..base
            },
            EffectPipelineKey {
                depth_write: !base.depth_write,
                ..base
            },
            EffectPipelineKey {
                vertex: VertexLayoutKind::Particle,
                ..base
            },
            EffectPipelineKey {
                blend: None,
                ..base
            },
            EffectPipelineKey {
                descriptor: stubs::particle(),
                ..base
            },
        ];
        for variant in variants {
            assert_ne!(variant, base);
        }
        assert_eq!(base, key_of(json!({ "mode": "add" })));
    }

    fn leaked_stubs() -> &'static TranslatedShaders {
        Box::leak(Box::new(stubs::translated()))
    }

    #[test]
    fn stub_pipelines_are_created_once_and_validate_on_a_device() {
        let Some((device, _queue)) = test_device("effect pipeline creation") else {
            return;
        };
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let scene = SceneCopy::new(&device, 4, 4);
        let mut pipelines = EffectPipelines::with_shaders(&device, scene.layout(), leaked_stubs());
        let mesh = EffectPipelineKey {
            descriptor: stubs::mesh(),
            vertex: VertexLayoutKind::Mesh,
            blend: Some((wgpu::BlendFactor::SrcAlpha, wgpu::BlendFactor::One)),
            double_sided: true,
            depth_test: true,
            depth_write: false,
        };
        let particle = EffectPipelineKey {
            descriptor: stubs::particle(),
            vertex: VertexLayoutKind::Particle,
            blend: Some((
                wgpu::BlendFactor::SrcAlpha,
                wgpu::BlendFactor::OneMinusSrcAlpha,
            )),
            ..mesh
        };
        let opaque_mesh = EffectPipelineKey {
            blend: None,
            depth_write: true,
            ..mesh
        };

        for key in [mesh, particle, opaque_mesh, mesh] {
            pipelines.pipeline(&key).unwrap();
        }

        assert_eq!(pipelines.pipeline_count(), 3);
        assert_eq!(
            pipelines.module_count(),
            2,
            "the two mesh pipelines share the module of their pair"
        );
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{error:?}");
    }

    #[test]
    fn a_pair_missing_from_the_translated_shaders_is_a_fatal_error_not_a_fallback() {
        let Some((device, _queue)) = test_device("effect pipeline missing shader") else {
            return;
        };
        let scene = SceneCopy::new(&device, 4, 4);
        let empty: &'static TranslatedShaders = Box::leak(Box::default());
        let mut pipelines = EffectPipelines::with_shaders(&device, scene.layout(), empty);
        let key = key_of(json!({ "mode": "none" }));

        let error = pipelines
            .pipeline(&key)
            .expect_err("the pair is not installed");

        assert!(matches!(error, EffectShaderError::Load(_)), "{error}");
        assert_eq!(pipelines.pipeline_count(), 0);
    }

    #[test]
    fn a_pair_naga_rejects_is_a_fatal_error_naming_both_shaders() {
        let Some((device, _queue)) = test_device("effect pipeline invalid pair") else {
            return;
        };
        let scene = SceneCopy::new(&device, 4, 4);
        let broken: &'static TranslatedShaders = Box::leak(Box::new(
            stubs::translated_with_mesh_vertex_wgsl("fn broken( {"),
        ));
        let mut pipelines = EffectPipelines::with_shaders(&device, scene.layout(), broken);

        let error = pipelines
            .pipeline(&key_of(json!({ "mode": "none" })))
            .expect_err("the module does not parse");

        let message = error.to_string();
        assert!(
            matches!(error, EffectShaderError::Invalid { .. }),
            "{message}"
        );
        assert!(
            message.contains(stubs::MESH_VS) && message.contains(stubs::MESH_PS),
            "{message}"
        );
    }
}
