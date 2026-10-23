//! The main menu's background video: the latest decoded frame as three plane textures, converted to
//! RGB in the fragment shader and drawn into the fitted rectangle of the target.
//!
//! The conversion runs on the GPU because the decoder already delivers I420: uploading it takes
//! 1.5 bytes per pixel instead of the 4 of CPU-converted RGBA and costs no CPU conversion time,
//! while the shader's three dot products are free next to a full-screen draw. The output is the
//! video's gamma-encoded R'G'B', so the layer draws through the non-sRGB view the menu uses.

use bytemuck::{Pod, Zeroable};

use crate::rendering::hud::quad::PxRect;
use crate::rendering::shaders;
use crate::video::{ChromaSiting, PlanarFrame, YuvToRgb, fit_rect};

const PLANE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// The uniform block of `menu_video.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
struct VideoParams {
    rect: [f32; 4],
    bias: [f32; 4],
    rows: [[f32; 4]; 3],
    // Added to the texture coordinates of the chroma planes; the first two are used.
    chroma_shift: [f32; 4],
}

impl VideoParams {
    /// `rect` in pixels of a target of `target` pixels, as clip-space edges.
    fn new(
        rect: PxRect,
        target: (u32, u32),
        video: (u32, u32),
        conversion: &YuvToRgb,
        siting: ChromaSiting,
    ) -> Self {
        let [shift_x, shift_y] = siting.sampling_shift();
        let [bias_y, bias_cb, bias_cr] = conversion.bias;
        Self {
            rect: rect.clip_edges(target),
            bias: [bias_y, bias_cb, bias_cr, 0.0],
            rows: conversion.rows.map(|[y, cb, cr]| [y, cb, cr, 0.0]),
            chroma_shift: [shift_x / video.0 as f32, shift_y / video.1 as f32, 0.0, 0.0],
        }
    }
}

struct Planes {
    textures: [wgpu::Texture; 3],
    size: (u32, u32),
    bind_group: wgpu::BindGroup,
    conversion: YuvToRgb,
    siting: ChromaSiting,
}

pub struct VideoLayer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
    planes: Option<Planes>,
}

impl VideoLayer {
    /// A layer drawing into targets of `format`.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = shaders::MENU_VIDEO.compile(device);
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("menu_video_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_entry(1),
                texture_entry(2),
                texture_entry(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("menu_video_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Menu Video Pipeline"),
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
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("menu_video_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("menu_video_params"),
            size: std::mem::size_of::<VideoParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            bind_group_layout,
            sampler,
            params,
            planes: None,
        }
    }

    #[cfg(test)]
    pub fn is_showing(&self) -> bool {
        self.planes.is_some()
    }

    /// Copies `frame` into the plane textures, recreating them when the video size changed.
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &PlanarFrame<'_>,
        conversion: YuvToRgb,
        siting: ChromaSiting,
    ) {
        let size = (frame.width, frame.height);
        if self
            .planes
            .as_ref()
            .is_none_or(|planes| planes.size != size)
        {
            self.planes = Some(self.create_planes(device, frame, conversion, siting));
        }
        let Some(planes) = &mut self.planes else {
            return;
        };
        planes.conversion = conversion;
        planes.siting = siting;
        for (index, (texture, plane)) in planes.textures.iter().zip(frame.planes).enumerate() {
            let (width, height) = frame.plane_size(index);
            queue.write_texture(
                texture.as_image_copy(),
                plane.data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(plane.stride as u32),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    /// Frees the textures; nothing is drawn until the next upload.
    pub fn clear(&mut self) {
        self.planes = None;
    }

    /// Writes the fitted rectangle for a target of `target` pixels; false when there is nothing
    /// to draw.
    pub fn prepare(&self, queue: &wgpu::Queue, target: (u32, u32)) -> bool {
        let Some(planes) = &self.planes else {
            return false;
        };
        let Some(rect) = fit_rect(planes.size, target) else {
            return false;
        };
        let params = VideoParams::new(rect, target, planes.size, &planes.conversion, planes.siting);
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        true
    }

    /// Draws the frame; call only after [`Self::prepare`] returned true.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(planes) = &self.planes else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &planes.bind_group, &[]);
        pass.draw(0..4, 0..1);
    }

    fn create_planes(
        &self,
        device: &wgpu::Device,
        frame: &PlanarFrame<'_>,
        conversion: YuvToRgb,
        siting: ChromaSiting,
    ) -> Planes {
        let textures: [wgpu::Texture; 3] = std::array::from_fn(|index| {
            let (width, height) = frame.plane_size(index);
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(["menu_video_y", "menu_video_cb", "menu_video_cr"][index]),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: PLANE_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        });
        let views = textures
            .each_ref()
            .map(|texture| texture.create_view(&wgpu::TextureViewDescriptor::default()));
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("menu_video_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&views[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&views[2]),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        Planes {
            textures,
            size: (frame.width, frame.height),
            bind_group,
            conversion,
            siting,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::{ColourMatrix, ColourRange};

    #[test]
    fn a_full_target_rectangle_spans_clip_space() {
        let conversion = YuvToRgb::new(ColourMatrix::Bt601, ColourRange::Limited);
        let rect = PxRect {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        };
        let params = VideoParams::new(
            rect,
            (1920, 1080),
            (1920, 1080),
            &conversion,
            ChromaSiting::MPEG2,
        );
        assert_eq!(
            params.rect.map(f32::to_bits),
            [-1.0, 1.0, 1.0, -1.0].map(f32::to_bits)
        );
    }

    #[test]
    fn letterbox_bars_become_symmetric_clip_space_margins() {
        let conversion = YuvToRgb::new(ColourMatrix::Bt601, ColourRange::Limited);
        let rect = PxRect {
            x: 0.0,
            y: 96.0,
            w: 1024.0,
            h: 576.0,
        };
        let params = VideoParams::new(
            rect,
            (1024, 768),
            (1920, 1080),
            &conversion,
            ChromaSiting::MPEG2,
        );
        assert_eq!(
            params.rect.map(f32::to_bits),
            [-1.0, 0.75, 1.0, -0.75].map(f32::to_bits)
        );
    }

    #[test]
    fn the_conversion_is_packed_row_by_row() {
        let conversion = YuvToRgb::new(ColourMatrix::Bt601, ColourRange::Limited);
        let rect = PxRect {
            x: 0.0,
            y: 0.0,
            w: 2.0,
            h: 2.0,
        };
        let params = VideoParams::new(rect, (2, 2), (2, 2), &conversion, ChromaSiting::MPEG2);
        assert_eq!(params.bias[..3], conversion.bias);
        for (packed, row) in params.rows.iter().zip(conversion.rows) {
            assert_eq!(packed[..3], row);
        }
    }
}
