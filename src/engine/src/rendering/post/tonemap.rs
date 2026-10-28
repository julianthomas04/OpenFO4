pub mod lut_library;
pub mod lut_table;

use lut_table::{LUT_SIZE, LutTable, blended_texels};

use crate::rendering::{
    gpu_profiler::{GpuPass, GpuProfiler, SpanPart},
    lighting::FrameBinding,
    shaders,
};

const LUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

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
    colour_lut: ColourLut,
}

/// The one table the pass grades the display picture with, filtered trilinearly between its
/// grid points; the frame uniform says how much of it to show.
struct ColourLut {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
}

impl ColourLut {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tonemap_colour_lut"),
            size: wgpu::Extent3d {
                width: LUT_SIZE,
                height: LUT_SIZE,
                depth_or_array_layers: LUT_SIZE,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: LUT_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("tonemap_colour_lut_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let lut = Self {
            texture,
            view,
            sampler,
        };
        lut.write(
            queue,
            &blended_texels(&[(Some(&LutTable::identity()), 1.0)]),
        );
        lut
    }

    fn write(&self, queue: &wgpu::Queue, texels: &[[u8; 4]]) {
        queue.write_texture(
            self.texture.as_image_copy(),
            bytemuck::cast_slice(texels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(LUT_SIZE * 4),
                rows_per_image: Some(LUT_SIZE),
            },
            self.texture.size(),
        );
    }
}

impl TonemapPass {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D3,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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

        let colour_lut = ColourLut::new(device, queue);
        let source_bind_group = Self::create_source_bind_group(
            device,
            &source_layout,
            hdr_view,
            exposure_state,
            &colour_lut,
        );

        Self {
            pipeline,
            source_layout,
            source_bind_group,
            target_format,
            colour_lut,
        }
    }

    /// The table the picture is graded with from now on: `texels` as [`blended_texels`] lays
    /// them out.
    pub fn write_colour_lut(&self, queue: &wgpu::Queue, texels: &[[u8; 4]]) {
        self.colour_lut.write(queue, texels);
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
        self.source_bind_group = Self::create_source_bind_group(
            device,
            &self.source_layout,
            hdr_view,
            exposure_state,
            &self.colour_lut,
        );
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
        colour_lut: &ColourLut,
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
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&colour_lut.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&colour_lut.sampler),
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
