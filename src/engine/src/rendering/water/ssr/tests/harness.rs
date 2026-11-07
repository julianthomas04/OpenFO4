//! GPU test plumbing for the SSR and depth of field tests: a depth buffer filled from explicit
//! values. Reading textures back and converting the values is in `gpu_test`.

use glam::UVec2;
use wgpu::util::DeviceExt;

use crate::rendering::{gpu_test::extent, texture::Texture};

const DEPTH_FILL: &str = "
@group(0) @binding(0) var source: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

struct Output {
    @builtin(frag_depth) depth: f32,
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> Output {
    return Output(textureLoad(source, vec2<i32>(position.xy), 0).x);
}
";

/// A `Depth32Float` texture holding `raw_depth` (row-major, one value per
/// texel). The depth format cannot be uploaded to, so a pass writes it.
pub fn depth_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    size: UVec2,
    raw_depth: &[f32],
) -> wgpu::Texture {
    let source = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("ssr_test_depth_source"),
            size: extent(size),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        bytemuck::cast_slice(raw_depth),
    );
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ssr_test_depth"),
        size: extent(size),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: Texture::DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ssr_test_depth_fill"),
        source: wgpu::ShaderSource::Wgsl(DEPTH_FILL.into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ssr_test_depth_fill_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }],
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("ssr_test_depth_fill"),
        layout: Some(
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            }),
        ),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            targets: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: Texture::DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ssr_test_depth_fill_group"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(
                &source.create_view(&wgpu::TextureViewDescriptor::default()),
            ),
        }],
    });
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ssr_test_depth_fill_pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
    queue.submit(std::iter::once(encoder.finish()));
    depth
}
