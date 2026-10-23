use super::model::{blade_layout, vertex_layout};
use super::uniforms::{GrassBatchUniform, GrassFrameUniform};
use crate::rendering::{
    render_pipelines::world_pass_layout::WorldPassLayout, shaders::composed, texture::Texture,
};

/// The bind group the grass shader reads on top of the world pass groups.
pub const GRASS_GROUP: u32 = 3;

pub struct GrassPipelines {
    pub batch_layout: wgpu::BindGroupLayout,
    culled: wgpu::RenderPipeline,
    two_sided: wgpu::RenderPipeline,
}

impl GrassPipelines {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        world: &WorldPassLayout,
        frame_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let uniform = |binding, visibility, size| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(size),
            },
            count: None,
        };
        let batch_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grass_batch_bgl"),
            entries: &[
                uniform(
                    0,
                    wgpu::ShaderStages::VERTEX,
                    std::mem::size_of::<GrassFrameUniform>() as u64,
                ),
                uniform(
                    1,
                    wgpu::ShaderStages::VERTEX_FRAGMENT,
                    std::mem::size_of::<GrassBatchUniform>() as u64,
                ),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Grass Pipeline Layout"),
            bind_group_layouts: &[
                Some(&world.material_bind_group_layout),
                Some(frame_layout),
                Some(&world.instance_bind_group_layout),
                Some(&batch_layout),
            ],
            immediate_size: 0,
        });
        let shader = composed::grass::GRASS.compile_runtime(device);
        let create =
            |label, cull_mode| create_pipeline(device, &shader, &layout, format, label, cull_mode);
        Self {
            culled: create("Grass Pipeline Culled", Some(wgpu::Face::Back)),
            two_sided: create("Grass Pipeline Two-Sided", None),
            batch_layout,
        }
    }

    pub fn pipeline(&self, two_sided: bool) -> &wgpu::RenderPipeline {
        if two_sided {
            &self.two_sided
        } else {
            &self.culled
        }
    }
}

fn create_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    label: &'static str,
    cull_mode: Option<wgpu::Face>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_grass"),
            buffers: &[Some(vertex_layout()), Some(blade_layout())],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_grass"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        // Reverse-Z like the object pipelines; grass writes depth so the water
        // passes and next frame's HiZ see it.
        depth_stencil: Some(wgpu::DepthStencilState {
            format: Texture::DEPTH_FORMAT,
            depth_write_enabled: Some(true),
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
    })
}
