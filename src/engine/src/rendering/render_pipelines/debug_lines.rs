use crate::rendering::{shaders, texture::Texture};

/// One end of a debug line: world-space position and an RGBA8 colour.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LineVertex {
    pub position: [f32; 3],
    pub color: [u8; 4],
}

impl LineVertex {
    pub const STRIDE: u64 = std::mem::size_of::<Self>() as u64;

    pub fn new(position: glam::Vec3, color: [u8; 4]) -> Self {
        Self {
            position: position.to_array(),
            color,
        }
    }
}

/// Line-list pipeline for the debug wireframe overlays (collision, navmesh). Owns nothing but
/// the pipeline: vertices are [`LineVertex`] values supplied per draw; of the world pass
/// groups it reads only the frame uniform.
pub struct DebugLinePipeline {
    pub pipeline: wgpu::RenderPipeline,
}

impl DebugLinePipeline {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        layout: &wgpu::PipelineLayout,
    ) -> Self {
        let shader = shaders::DEBUG_LINES.compile(device);

        let vertex_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Unorm8x4];

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Debug Line Pipeline"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: LineVertex::STRIDE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &vertex_attrs,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: Texture::DEPTH_FORMAT,
                // Never write: the HiZ occlusion pyramid is generated from
                // this depth buffer after the pass — wireframe lines must
                // not occlusion-cull real geometry next frame.
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        });

        Self { pipeline }
    }
}
