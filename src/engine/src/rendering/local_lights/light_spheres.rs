use super::buffers::ClusterBuffers;
use crate::rendering::{shaders, texture::Texture};

/// Three great circles of `SPHERE_CIRCLE_SEGMENTS` line segments; mirrored in
/// `shaders/light_spheres.wgsl`.
const SPHERE_CIRCLES: u32 = 3;
const SPHERE_CIRCLE_SEGMENTS: u32 = 32;
pub const SPHERE_VERTEX_COUNT: u32 = SPHERE_CIRCLES * SPHERE_CIRCLE_SEGMENTS * 2;

/// Debug overlay of every visible light's range, drawn inside the world pass
/// with its shared layout. The instance count comes from the cluster build.
pub struct LightSpherePipeline {
    pipeline: wgpu::RenderPipeline,
}

impl LightSpherePipeline {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        world_pass_layout: &wgpu::PipelineLayout,
    ) -> Self {
        let shader = shaders::LIGHT_SPHERES.compile(device);
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Light Sphere Pipeline"),
            layout: Some(world_pass_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
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
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: Texture::DEPTH_FORMAT,
                // Never written: the HiZ pyramid is built from this depth buffer.
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self { pipeline }
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, buffers: &ClusterBuffers) {
        pass.set_pipeline(&self.pipeline);
        pass.draw_indirect(&buffers.sphere_draw_args, 0);
    }
}
