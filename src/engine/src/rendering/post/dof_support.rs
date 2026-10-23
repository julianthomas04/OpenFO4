//! What the passes of the depth of field effect share: the bind group layout
//! entries, the fullscreen pipelines and the draw of one pass.

use super::depth_of_field::DofRegisters;
use crate::rendering::shaders::composed::dof::DEPTH_OF_FIELD;

pub const PARAMS_BINDING: u32 = 0;
pub const SHARP_BINDING: u32 = 1;
pub const BLURRED_BINDING: u32 = 2;
/// The merged reverse-Z depth the circle of confusion and resolve passes read.
pub const DEPTH_BINDING: u32 = 3;
pub const SURFACE_DEPTH_BINDING: u32 = 4;
pub const BLUR_SOURCE_BINDING: u32 = 5;
pub const BLUR_SAMPLER_BINDING: u32 = 6;
/// The live scene depth, read by the depth merge pass only.
pub const SCENE_DEPTH_BINDING: u32 = 7;

/// What the first and last pass read besides their own input.
pub struct SceneInputs<'a> {
    pub params: &'a wgpu::Buffer,
    /// The scene as the effect starts.
    pub sharp: &'a wgpu::TextureView,
    /// The nearer of the scene depth and the underwater surface depth.
    pub merged_depth: &'a wgpu::TextureView,
}

pub fn extent(width: u32, height: u32) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: width.max(1),
        height: height.max(1),
        depth_or_array_layers: 1,
    }
}

pub fn params_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: PARAMS_BINDING,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<DofRegisters>() as u64),
        },
        count: None,
    }
}

pub fn texture_entry(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

pub fn unfilterable_float(binding: u32) -> wgpu::BindGroupLayoutEntry {
    texture_entry(
        binding,
        wgpu::TextureSampleType::Float { filterable: false },
    )
}

pub fn depth_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    texture_entry(binding, wgpu::TextureSampleType::Depth)
}

pub fn view_entry(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

/// The depth of field shader module, composed from the wrapper and the translated passes.
pub fn compile_module(device: &wgpu::Device) -> wgpu::ShaderModule {
    DEPTH_OF_FIELD.compile_runtime(device)
}

pub fn fullscreen_pipeline(
    device: &wgpu::Device,
    label: &str,
    module: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    fragment_entry: &str,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment_entry),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// One fullscreen triangle into `target`.
pub fn draw_fullscreen(
    encoder: &mut wgpu::CommandEncoder,
    label: &str,
    target: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    bind_group: &wgpu::BindGroup,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind_group, &[]);
    pass.draw(0..3, 0..1);
}
