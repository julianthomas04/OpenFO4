use crate::rendering::{
    gpu_profiler::{GpuPass, GpuProfiler, SpanPart},
    lighting::FrameBinding,
    shaders,
};

/// The tonemap writes linear values, so it must target an sRGB view of the swapchain.
pub fn target_format(surface_format: wgpu::TextureFormat) -> wgpu::TextureFormat {
    surface_format.add_srgb_suffix()
}

/// The view format the 2D UI draws through: the swapchain without its sRGB encoding, so that
/// alpha blending happens on the encoded values as Flash composites them.
pub fn ui_gamma_format(surface_format: wgpu::TextureFormat) -> wgpu::TextureFormat {
    surface_format.remove_srgb_suffix()
}

/// Extra view formats the surface must be configured with for `target_format` and
/// `ui_gamma_format` to be viewable.
pub fn surface_view_formats(surface_format: wgpu::TextureFormat) -> Vec<wgpu::TextureFormat> {
    let mut formats = Vec::new();
    for format in [
        target_format(surface_format),
        ui_gamma_format(surface_format),
    ] {
        if format != surface_format && !formats.contains(&format) {
            formats.push(format);
        }
    }
    formats
}

pub struct TonemapPass {
    pipeline: wgpu::RenderPipeline,
    source_layout: wgpu::BindGroupLayout,
    source_bind_group: wgpu::BindGroup,
    target_format: wgpu::TextureFormat,
}

impl TonemapPass {
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        frame_layout: &wgpu::BindGroupLayout,
        hdr_view: &wgpu::TextureView,
        exposure_state: &wgpu::Buffer,
    ) -> Self {
        let target_format = target_format(surface_format);
        let shader = shaders::TONEMAP.compile(device);

        let source_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tonemap_source_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Tonemap Pipeline Layout"),
            bind_group_layouts: &[Some(&source_layout), Some(frame_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Tonemap Pipeline"),
            layout: Some(&layout),
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
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let source_bind_group =
            Self::create_source_bind_group(device, &source_layout, hdr_view, exposure_state);

        Self {
            pipeline,
            source_layout,
            source_bind_group,
            target_format,
        }
    }

    pub fn target_format(&self) -> wgpu::TextureFormat {
        self.target_format
    }

    pub fn rebind_source(
        &mut self,
        device: &wgpu::Device,
        hdr_view: &wgpu::TextureView,
        exposure_state: &wgpu::Buffer,
    ) {
        self.source_bind_group =
            Self::create_source_bind_group(device, &self.source_layout, hdr_view, exposure_state);
    }

    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        frame: &FrameBinding,
        profiler: &GpuProfiler,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Tonemap Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: profiler.render_writes(GpuPass::Tonemap, SpanPart::Whole),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.source_bind_group, &[]);
        frame.bind(&mut pass);
        pass.draw(0..3, 0..1);
    }

    fn create_source_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        hdr_view: &wgpu::TextureView,
        exposure_state: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tonemap_source_bind_group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(hdr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: exposure_state.as_entire_binding(),
                },
            ],
        })
    }
}

#[cfg(test)]
mod gpu_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::TextureFormat;

    #[test]
    fn a_linear_surface_is_viewed_through_its_srgb_variant() {
        assert_eq!(
            target_format(TextureFormat::Bgra8Unorm),
            TextureFormat::Bgra8UnormSrgb
        );
        assert_eq!(
            surface_view_formats(TextureFormat::Bgra8Unorm),
            vec![TextureFormat::Bgra8UnormSrgb]
        );
    }

    #[test]
    fn an_srgb_surface_is_viewed_as_itself_and_as_its_gamma_variant() {
        assert_eq!(
            target_format(TextureFormat::Rgba8UnormSrgb),
            TextureFormat::Rgba8UnormSrgb
        );
        assert_eq!(
            ui_gamma_format(TextureFormat::Rgba8UnormSrgb),
            TextureFormat::Rgba8Unorm
        );
        assert_eq!(
            surface_view_formats(TextureFormat::Rgba8UnormSrgb),
            vec![TextureFormat::Rgba8Unorm]
        );
    }

    #[test]
    fn a_linear_surface_draws_the_ui_through_itself() {
        assert_eq!(
            ui_gamma_format(TextureFormat::Bgra8Unorm),
            TextureFormat::Bgra8Unorm
        );
    }
}
