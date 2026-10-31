//! The pipelines of the rain pass and the bind group they share. The pass lays out its own
//! groups: the rain's at 0 and the frame's at 1. The pipelines draw instanced quads of four
//! vertices, blended with the scene as their materials say and tested against its depth without
//! writing it. The streaks have one pipeline per blend their materials ask for, created the
//! first time a weather asks for it.

use std::collections::HashMap;

use super::material::{SPLASH_BLEND, StreakMaterial, adds_light};
use super::uniform::RainUniform;
use crate::rendering::effects::pipeline::{BlendFactors, blend_state};
use crate::rendering::post::hdr_target::HDR_FORMAT;
use crate::rendering::shaders;
use crate::rendering::texture::Texture;

/// The corners of one drop or splash.
pub(super) const QUAD_VERTICES: u32 = 4;

const UNIFORM_BINDING: u32 = 0;
const STREAK_TEXTURE_BINDING: u32 = 1;
const SAMPLER_BINDING: u32 = 2;
const SPLASH_TEXTURE_BINDING: u32 = 3;
const OCCLUSION_BINDING: u32 = 4;
/// The `rain.wgsl` override that tells the streaks whether their blend adds light.
const STREAK_ADDS_LIGHT_OVERRIDE: &str = "STREAK_ADDS_LIGHT";

pub(super) struct RainPipelines {
    pub layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    module: wgpu::ShaderModule,
    /// Keyed by the blend of the streak material; `None` draws unblended.
    streaks: HashMap<Option<BlendFactors>, wgpu::RenderPipeline>,
    pub splashes: wgpu::RenderPipeline,
}

impl RainPipelines {
    /// Creates the streak pipeline of [`StreakMaterial::FALLBACK`] up front.
    pub fn new(device: &wgpu::Device, frame_layout: &wgpu::BindGroupLayout) -> Self {
        let layout = bind_group_layout(device);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Rain Pipeline Layout"),
            bind_group_layouts: &[Some(&layout), Some(frame_layout)],
            immediate_size: 0,
        });
        let module = shaders::RAIN.compile(device);
        let splashes = create_pipeline(
            device,
            "Rain Splash Pipeline",
            &module,
            &pipeline_layout,
            ("vs_splash", "fs_splash"),
            Some(SPLASH_BLEND),
            &[],
        );
        let mut pipelines = Self {
            layout,
            pipeline_layout,
            module,
            streaks: HashMap::new(),
            splashes,
        };
        pipelines.streaks(device, StreakMaterial::FALLBACK.blend);
        pipelines
    }

    /// The streak pipeline of `blend`, created on first use. The blend also decides how the fog
    /// covers the streaks, so the one key serves both.
    pub fn streaks(
        &mut self,
        device: &wgpu::Device,
        blend: Option<BlendFactors>,
    ) -> &wgpu::RenderPipeline {
        self.streaks.entry(blend).or_insert_with(|| {
            create_pipeline(
                device,
                "Rain Streak Pipeline",
                &self.module,
                &self.pipeline_layout,
                ("vs_streak", "fs_streak"),
                blend,
                &[(
                    STREAK_ADDS_LIGHT_OVERRIDE,
                    f64::from(u8::from(adds_light(blend))),
                )],
            )
        })
    }

    #[cfg(test)]
    pub fn streak_pipeline_count(&self) -> usize {
        self.streaks.len()
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("rain_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: UNIFORM_BINDING,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<RainUniform>() as u64
                    ),
                },
                count: None,
            },
            texture_entry(STREAK_TEXTURE_BINDING),
            wgpu::BindGroupLayoutEntry {
                binding: SAMPLER_BINDING,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            texture_entry(SPLASH_TEXTURE_BINDING),
            wgpu::BindGroupLayoutEntry {
                binding: OCCLUSION_BINDING,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    })
}

pub(super) fn sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("rain_sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    })
}

pub(super) struct RainResources<'a> {
    pub uniform: &'a wgpu::Buffer,
    pub streaks: &'a wgpu::TextureView,
    pub sampler: &'a wgpu::Sampler,
    pub splashes: &'a wgpu::TextureView,
    pub occlusion: &'a wgpu::TextureView,
}

pub(super) fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    resources: &RainResources<'_>,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("rain_bind_group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: UNIFORM_BINDING,
                resource: resources.uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: STREAK_TEXTURE_BINDING,
                resource: wgpu::BindingResource::TextureView(resources.streaks),
            },
            wgpu::BindGroupEntry {
                binding: SAMPLER_BINDING,
                resource: wgpu::BindingResource::Sampler(resources.sampler),
            },
            wgpu::BindGroupEntry {
                binding: SPLASH_TEXTURE_BINDING,
                resource: wgpu::BindingResource::TextureView(resources.splashes),
            },
            wgpu::BindGroupEntry {
                binding: OCCLUSION_BINDING,
                resource: wgpu::BindingResource::TextureView(resources.occlusion),
            },
        ],
    })
}

/// The target's alpha stays as it is.
fn create_pipeline(
    device: &wgpu::Device,
    label: &str,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    (vertex_entry, fragment_entry): (&str, &str),
    blend: Option<BlendFactors>,
    constants: &[(&str, f64)],
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(vertex_entry),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment_entry),
            targets: &[Some(wgpu::ColorTargetState {
                format: HDR_FORMAT,
                blend: blend.map(blend_state),
                write_mask: wgpu::ColorWrites::COLOR,
            })],
            compilation_options: wgpu::PipelineCompilationOptions {
                constants,
                ..Default::default()
            },
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: Texture::DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}
