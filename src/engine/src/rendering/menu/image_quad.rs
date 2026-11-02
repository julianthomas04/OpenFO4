//! A textured rectangle of the menu's gamma-space pass: the loading logo and the large loading
//! screen's background image share this pipeline. The texture is premultiplied, multiplied by a
//! colour and an opacity, and written as is, because the menu draws through the non-sRGB view of
//! the swapchain.

use bytemuck::{Pod, Zeroable};

use crate::rendering::hud::quad::PxRect;
use crate::rendering::shaders;

/// The uniform block of `menu_icon.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub(super) struct ImageQuadParams {
    /// Left, top, right and bottom in clip space.
    pub rect: [f32; 4],
    /// The red, green and blue the texture is multiplied by, and the opacity of the whole quad.
    pub colour: [f32; 4],
}

impl ImageQuadParams {
    /// A quad over `rect` of a target of `target` pixels, drawn as stored at `opacity`.
    pub fn plain(rect: PxRect, target: (u32, u32), opacity: f32) -> Self {
        Self {
            rect: rect.clip_edges(target),
            colour: [1.0, 1.0, 1.0, opacity.clamp(0.0, 1.0)],
        }
    }
}

/// A uniform buffer for one quad's [`ImageQuadParams`].
pub(super) fn params_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("menu_icon_params"),
        size: std::mem::size_of::<ImageQuadParams>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

pub(super) struct ImageQuadPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl ImageQuadPipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("menu_icon_bgl"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("menu_icon_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let shader = shaders::MENU_ICON.compile(device);
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Menu Icon Pipeline"),
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
            label: Some("menu_icon_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            bind_group_layout,
            sampler,
        }
    }

    /// The bind group that draws `view` with the parameters in `params`.
    pub fn bind_group(
        &self,
        device: &wgpu::Device,
        params: &wgpu::Buffer,
        view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("menu_icon_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, bind_group: &wgpu::BindGroup) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..4, 0..1);
    }
}

/// The rectangle of an image of `image` pixels that fills a target of `target` pixels with its
/// aspect kept: scaled until both sides are covered, centred, the overflow cropped by the target.
pub(super) fn cover_rect(image: (u32, u32), target: (u32, u32)) -> PxRect {
    let (target_w, target_h) = (target.0 as f32, target.1 as f32);
    let scale = (target_w / image.0 as f32).max(target_h / image.1 as f32);
    let (w, h) = (image.0 as f32 * scale, image.1 as f32 * scale);
    PxRect {
        x: (target_w - w) / 2.0,
        y: (target_h - h) / 2.0,
        w,
        h,
    }
}

/// The large loading screen's background: one texture covering the whole target.
pub(super) struct BackgroundQuad {
    params: wgpu::Buffer,
    bind_group: Option<wgpu::BindGroup>,
}

impl BackgroundQuad {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            params: params_buffer(device),
            bind_group: None,
        }
    }

    /// Readies `view` to cover a target of `target` pixels at `opacity`, its aspect kept.
    pub fn prepare(
        &mut self,
        pipeline: &ImageQuadPipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        target: (u32, u32),
        opacity: f32,
    ) {
        let size = view.texture().size();
        let cover = cover_rect((size.width, size.height), target);
        let params = ImageQuadParams::plain(cover, target, opacity);
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        self.bind_group = Some(pipeline.bind_group(device, &self.params, view));
    }

    /// Takes the readied image away: the next draw shows nothing until [`Self::prepare`].
    pub fn clear(&mut self) {
        self.bind_group = None;
    }

    pub fn draw(&self, pipeline: &ImageQuadPipeline, pass: &mut wgpu::RenderPass<'_>) {
        if let Some(bind_group) = &self.bind_group {
            pipeline.draw(pass, bind_group);
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the arithmetic below stays on exactly representable values"
)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> PxRect {
        PxRect { x, y, w, h }
    }

    #[test]
    fn an_image_of_the_targets_aspect_fills_it_exactly() {
        assert_eq!(
            cover_rect((640, 360), (1280, 720)),
            rect(0.0, 0.0, 1280.0, 720.0)
        );
    }

    #[test]
    fn a_square_image_on_a_wide_target_is_scaled_to_its_width_and_cropped_top_and_bottom() {
        assert_eq!(
            cover_rect((100, 100), (400, 200)),
            rect(0.0, -100.0, 400.0, 400.0)
        );
    }

    #[test]
    fn a_square_image_on_a_tall_target_is_scaled_to_its_height_and_cropped_left_and_right() {
        assert_eq!(
            cover_rect((100, 100), (200, 400)),
            rect(-100.0, 0.0, 400.0, 400.0)
        );
    }

    #[test]
    fn the_opacity_stays_between_clear_and_opaque() {
        let whole = rect(0.0, 0.0, 10.0, 10.0);
        let opacity = |value| ImageQuadParams::plain(whole, (10, 10), value).colour[3];
        assert_eq!(opacity(-1.0), 0.0);
        assert_eq!(opacity(0.25), 0.25);
        assert_eq!(opacity(3.0), 1.0);
    }
}
