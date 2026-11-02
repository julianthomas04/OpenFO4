//! The loading art's GPU state that does not depend on a model: the pipeline, the per-frame
//! uniform block and the depth buffer.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

use crate::rendering::shaders;
use crate::rendering::texture::Texture;
use crate::rendering::vertex::Vertex;

/// Inferred: a light above, left of and behind the viewer, with a flat ambient term.
const LIGHT_TOWARDS: Vec3 = Vec3::new(-0.4, 0.6, 0.7);
const AMBIENT: f32 = 0.35;
const DIFFUSE: f32 = 0.75;

/// The uniform block of `loading_art.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub(super) struct SceneUniform {
    view_proj: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    light_direction: [f32; 4],
    shading: [f32; 4],
}

impl SceneUniform {
    pub fn new(view_proj: Mat4, model: Mat4, encode_srgb: bool) -> Self {
        Self {
            view_proj: view_proj.to_cols_array_2d(),
            model: model.to_cols_array_2d(),
            light_direction: LIGHT_TOWARDS.normalize().extend(0.0).to_array(),
            shading: [AMBIENT, DIFFUSE, if encode_srgb { 1.0 } else { 0.0 }, 0.0],
        }
    }
}

pub(super) struct ArtPipeline {
    pub depth_prepass: wgpu::RenderPipeline,
    pub pipeline: wgpu::RenderPipeline,
    pub scene_layout: wgpu::BindGroupLayout,
    pub texture_layout: wgpu::BindGroupLayout,
}

impl ArtPipeline {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("loading_art_scene_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("loading_art_texture_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("loading_art_layout"),
            bind_group_layouts: &[Some(&scene_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let shader = shaders::LOADING_ART.compile(device);
        let build = |label: &str, stage: Stage| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[Some(Vertex::desc())],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(stage.entry_point),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend: stage.blend,
                        write_mask: stage.write_mask,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: Texture::DEPTH_FORMAT,
                    depth_write_enabled: Some(stage.depth_write),
                    depth_compare: Some(stage.depth_compare),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        // The nearest surface is found first, without colour, and cut-out texels write no depth;
        // the colour pass then draws only the fragments at exactly that depth, so overlapping
        // parts show their nearest surface whatever the order they are drawn in.
        let depth_prepass = build(
            "Loading Art Depth Pipeline",
            Stage {
                entry_point: "fs_depth",
                blend: None,
                write_mask: wgpu::ColorWrites::empty(),
                depth_write: true,
                depth_compare: wgpu::CompareFunction::Less,
            },
        );
        let pipeline = build(
            "Loading Art Pipeline",
            Stage {
                entry_point: "fs_main",
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
                depth_write: false,
                depth_compare: wgpu::CompareFunction::Equal,
            },
        );
        Self {
            depth_prepass,
            pipeline,
            scene_layout,
            texture_layout,
        }
    }
}

/// What differs between the depth prepass and the colour pass.
struct Stage {
    entry_point: &'static str,
    blend: Option<wgpu::BlendState>,
    write_mask: wgpu::ColorWrites,
    depth_write: bool,
    depth_compare: wgpu::CompareFunction,
}

/// The depth buffer of the loading art, sized like the target it draws into.
pub(super) struct DepthTarget {
    size: (u32, u32),
    view: wgpu::TextureView,
}

impl DepthTarget {
    fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("loading_art_depth"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Texture::DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        Self {
            size,
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
        }
    }

    /// The view of a depth buffer of `size`, replacing `slot`'s when the size changed.
    pub fn view_for<'a>(
        slot: &'a mut Option<Self>,
        device: &wgpu::Device,
        size: (u32, u32),
    ) -> &'a wgpu::TextureView {
        if slot.as_ref().is_some_and(|depth| depth.size != size) {
            *slot = None;
        }
        &slot.get_or_insert_with(|| Self::new(device, size)).view
    }
}
