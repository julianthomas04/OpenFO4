//! The 3D model of a large loading screen, drawn between the black backdrop and the logo with a
//! pipeline of its own. The world pass is not reused: its buffers churn while a load tears the
//! old scene down.
//!
//! A model is loaded in two steps, [`CpuArt::load`] and [`LoadedArt::upload`], neither of which
//! needs the frame thread. [`LoadingArtRenderer::set_art`] only builds bind groups.

mod art;
mod background;
mod camera;
mod model;
mod motion;
mod pan_window;
mod pipeline;
#[cfg(test)]
mod renderer_tests;
#[cfg(test)]
pub(crate) mod test_support;
mod tuning;

pub use art::{LoadingArt, ModelTransform};
pub use background::LoadingBackground;
pub use camera::ArtCamera;
pub use model::{CpuArt, LoadedArt, LoadingArtError};
pub use motion::PanKey;

use bytemuck::bytes_of;
use wgpu::util::DeviceExt;

use pipeline::{ArtPipeline, DepthTarget, MaterialUniform, SceneUniform};

struct Active {
    art: LoadedArt,
    texture_groups: Vec<wgpu::BindGroup>,
}

pub struct LoadingArtRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    encode_srgb: bool,
    pipeline: ArtPipeline,
    sampler: wgpu::Sampler,
    scene_buffer: wgpu::Buffer,
    scene_group: wgpu::BindGroup,
    depth: Option<DepthTarget>,
    active: Option<Active>,
}

impl LoadingArtRenderer {
    /// A renderer for views of `target_format`. The menu draws through the swapchain's non-sRGB
    /// view, so a format that does not encode sRGB itself gets its colour encoded by the shader,
    /// and alpha blends on the encoded values like the rest of the menu.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let pipeline = ArtPipeline::new(device, target_format);
        let scene_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("loading_art_scene"),
            size: std::mem::size_of::<SceneUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let scene_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("loading_art_scene_group"),
            layout: &pipeline.scene_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: scene_buffer.as_entire_binding(),
            }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("loading_art_sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        Self {
            device: device.clone(),
            queue: queue.clone(),
            encode_srgb: !target_format.is_srgb(),
            pipeline,
            sampler,
            scene_buffer,
            scene_group,
            depth: None,
            active: None,
        }
    }

    /// Shows `art` from the next render on, with a camera back at its placement as stored;
    /// `None` shows nothing.
    pub fn set_art(&mut self, art: Option<LoadedArt>) {
        self.active = art.map(|art| {
            let texture_groups = art
                .draws
                .iter()
                .map(|draw| {
                    let material =
                        self.device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("loading_art_material"),
                                contents: bytes_of(&MaterialUniform::new(draw.alpha_cutoff)),
                                usage: wgpu::BufferUsages::UNIFORM,
                            });
                    self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("loading_art_texture_group"),
                        layout: &self.pipeline.texture_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&draw.diffuse),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&self.sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: material.as_entire_binding(),
                            },
                        ],
                    })
                })
                .collect();
            Active {
                art,
                texture_groups,
            }
        });
    }

    pub fn has_art(&self) -> bool {
        self.active.is_some()
    }

    /// A drag of `dx_px` pixels to the right and `dy_px` down.
    pub fn drag(&mut self, dx_px: f32, dy_px: f32) {
        if let Some(active) = &mut self.active {
            active.art.camera.drag(dx_px, dy_px);
        }
    }

    /// A pan key went down or up.
    pub fn set_pan_key(&mut self, key: PanKey, held: bool) {
        if let Some(active) = &mut self.active {
            active.art.camera.set_pan_key(key, held);
        }
    }

    /// Lets `seconds` pass for the model's spin and pan.
    pub fn tick(&mut self, seconds: f32) {
        if let Some(active) = &mut self.active {
            active.art.camera.tick(seconds);
        }
    }

    /// The model's camera, for the screen's input handling to read.
    pub fn camera(&self) -> Option<&ArtCamera> {
        self.active.as_ref().map(|active| &active.art.camera)
    }

    /// Draws the model over what `view`, of `size` pixels, holds, opaque, in a pass of
    /// its own. Nothing is encoded without a model or for an empty target.
    pub fn render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        size: (u32, u32),
    ) {
        let Some(active) = &self.active else { return };
        if size.0 == 0 || size.1 == 0 {
            return;
        }
        let aspect = size.0 as f32 / size.1 as f32;
        let scene = SceneUniform::new(
            active.art.camera.view_proj(aspect),
            active.art.camera.model_matrix(),
            self.encode_srgb,
        );
        self.queue
            .write_buffer(&self.scene_buffer, 0, bytes_of(&scene));
        let depth = DepthTarget::view_for(&mut self.depth, &self.device, size);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Loading Art Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &self.scene_group, &[]);
        pass.set_vertex_buffer(0, active.art.vertices.slice(..));
        pass.set_index_buffer(active.art.indices.slice(..), wgpu::IndexFormat::Uint32);
        for pipeline in [&self.pipeline.depth_prepass, &self.pipeline.pipeline] {
            pass.set_pipeline(pipeline);
            for (draw, group) in active.art.draws.iter().zip(&active.texture_groups) {
                pass.set_bind_group(1, group, &[]);
                pass.draw_indexed(draw.indices.clone(), draw.base_vertex, 0..1);
            }
        }
    }
}
