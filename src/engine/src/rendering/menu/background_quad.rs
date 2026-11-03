//! The large loading screen's background: the interface image and the additive grid overlay over
//! the whole target, drawn by `loading_background.wgsl`.

use bytemuck::{Pod, Zeroable};

use crate::rendering::loading_screen::{
    BACKGROUND_U_MAX, BackgroundImages, GRADE_LIFT, GRADE_TINT, OVERLAY_ALPHA_FACTOR,
};
use crate::rendering::shaders;

/// The uniform block of `loading_background.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
struct BackgroundParams {
    /// The right end of the u range, the factor of the overlay's alpha, the fade, unused.
    look: [f32; 4],
    tint: [f32; 4],
    lift: [f32; 4],
}

impl BackgroundParams {
    fn new(opacity: f32) -> Self {
        Self {
            look: [
                BACKGROUND_U_MAX,
                OVERLAY_ALPHA_FACTOR,
                opacity.clamp(0.0, 1.0),
                0.0,
            ],
            tint: [GRADE_TINT[0], GRADE_TINT[1], GRADE_TINT[2], 0.0],
            lift: [GRADE_LIFT[0], GRADE_LIFT[1], GRADE_LIFT[2], 0.0],
        }
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// A one texel texture that adds nothing, standing in for a missing overlay.
fn blank_overlay(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    let size = wgpu::Extent3d {
        width: 1,
        height: 1,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("loading_background_blank_overlay"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        &[0; 4],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        size,
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

pub(super) struct BackgroundQuad {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    blank_overlay: wgpu::TextureView,
    params: wgpu::Buffer,
    bind_group: Option<wgpu::BindGroup>,
}

impl BackgroundQuad {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("loading_background_bgl"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("loading_background_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let shader = shaders::LOADING_BACKGROUND.compile(device);
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Loading Background Pipeline"),
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
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
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
            label: Some("loading_background_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("loading_background_params"),
            size: std::mem::size_of::<BackgroundParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            bind_group_layout,
            sampler,
            blank_overlay: blank_overlay(device, queue),
            params,
            bind_group: None,
        }
    }

    /// Readies `images` to fill the target at `opacity`; a missing overlay adds nothing.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        images: BackgroundImages<'_>,
        opacity: f32,
    ) {
        queue.write_buffer(
            &self.params,
            0,
            bytemuck::bytes_of(&BackgroundParams::new(opacity)),
        );
        let overlay = images.overlay.unwrap_or(&self.blank_overlay);
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("loading_background_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(images.background),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(overlay),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        }));
    }

    /// Takes the readied images away: the next draw shows nothing until [`Self::prepare`].
    pub fn clear(&mut self) {
        self.bind_group = None;
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some(bind_group) = &self.bind_group {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..4, 0..1);
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the clamp returns exactly the values it is given"
)]
mod tests {
    use super::*;

    #[test]
    fn the_opacity_stays_between_clear_and_opaque() {
        let opacity = |value| BackgroundParams::new(value).look[2];
        assert_eq!(opacity(-1.0), 0.0);
        assert_eq!(opacity(0.25), 0.25);
        assert_eq!(opacity(3.0), 1.0);
    }
}
