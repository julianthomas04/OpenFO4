//! The HUD render pipeline: a vertex-colored alpha-blended triangle pass with
//! NO depth attachment. Structurally a copy of `render_pipelines/debug_lines.rs`
//! adapted for triangles + a tint uniform. The absence of depth is load-bearing
//! (see the `depth_stencil: None` note below).

use wgpu::util::DeviceExt;

use super::quad::HudVertex;
use crate::rendering::shaders;
use crate::rendering::ui_colour::UiTint;

pub struct HudPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub tint_layout: wgpu::BindGroupLayout,
}

impl HudPipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = shaders::HUD.compile(device);

        let tint_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hud_tint_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("HUD Pipeline Layout"),
            bind_group_layouts: &[Some(&tint_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("HUD Pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(HudVertex::desc())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            // No depth attachment. The HiZ occlusion pyramid is generated from
            // the depth buffer BEFORE this pass runs (rendering/mod.rs), so a
            // depth write here would stale next frame's culling. The HUD is
            // depth-agnostic; matches the egui pass (debug_ui.rs).
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            tint_layout,
        }
    }

    /// The bind group holding `tint`, the colour and opacity every HUD vertex is multiplied by.
    pub fn create_tint_bind_group(&self, device: &wgpu::Device, tint: &UiTint) -> wgpu::BindGroup {
        let tint_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("hud_tint"),
            contents: bytemuck::bytes_of(tint),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hud_tint_bg"),
            layout: &self.tint_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: tint_buffer.as_entire_binding(),
            }],
        })
    }
}
