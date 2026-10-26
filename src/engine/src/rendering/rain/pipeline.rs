//! The pipelines of the rain pass and the bind group they share. The pass lays out its own
//! groups: the rain's at 0 and the frame's at 1. Both pipelines draw instanced quads of four
//! vertices, blended with the scene as their materials say and tested against its depth without
//! writing it.

use super::material::{SPLASH_BLEND, STREAK_BLEND};
use super::uniform::RainUniform;
use crate::rendering::post::hdr_target::HDR_FORMAT;
use crate::rendering::shaders::{self, ShaderSource};
use crate::rendering::texture::Texture;

/// The corners of one drop or splash.
pub(super) const QUAD_VERTICES: u32 = 4;

const UNIFORM_BINDING: u32 = 0;
const STREAK_TEXTURE_BINDING: u32 = 1;
const SAMPLER_BINDING: u32 = 2;
const SPLASH_TEXTURE_BINDING: u32 = 3;
const OCCLUSION_BINDING: u32 = 4;

pub(super) struct RainPipelines {
    pub layout: wgpu::BindGroupLayout,
    pub streaks: wgpu::RenderPipeline,
    pub splashes: wgpu::RenderPipeline,
}

impl RainPipelines {
    pub fn new(device: &wgpu::Device, frame_layout: &wgpu::BindGroupLayout) -> Self {
        let layout = bind_group_layout(device);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Rain Pipeline Layout"),
            bind_group_layouts: &[Some(&layout), Some(frame_layout)],
            immediate_size: 0,
        });
        let shader = &shaders::RAIN;
        Self {
            streaks: create_pipeline(
                device,
                "Rain Streak Pipeline",
                shader,
                &pipeline_layout,
                ("vs_streak", "fs_streak"),
                STREAK_BLEND,
            ),
            splashes: create_pipeline(
                device,
                "Rain Splash Pipeline",
                shader,
                &pipeline_layout,
                ("vs_splash", "fs_splash"),
                SPLASH_BLEND,
            ),
            layout,
        }
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

fn create_pipeline(
    device: &wgpu::Device,
    label: &str,
    shader: &ShaderSource,
    layout: &wgpu::PipelineLayout,
    (vertex_entry, fragment_entry): (&str, &str),
    blend: wgpu::BlendState,
) -> wgpu::RenderPipeline {
    let module = shader.compile(device);
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some(vertex_entry),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some(fragment_entry),
            targets: &[Some(wgpu::ColorTargetState {
                format: HDR_FORMAT,
                blend: Some(blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
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
