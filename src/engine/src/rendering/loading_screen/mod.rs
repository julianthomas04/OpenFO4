//! The 3D scene of a large loading screen, drawn between the black backdrop and the logo with
//! pipelines of its own: the stage, the model in front of it and the colour pass over both. The
//! world pass is not reused: its buffers churn while a load tears the old scene down.
//!
//! A model is loaded in two steps, [`CpuArt::load`] and [`LoadedArt::upload`], neither of which
//! needs the frame thread. [`LoadingArtRenderer::set_art`] only builds bind groups.

mod art;
mod camera;
mod controls;
mod grade;
mod grade_pass;
mod model;
mod motion;
mod pan_window;
mod pipeline;
#[cfg(test)]
mod renderer_tests;
mod scene;
mod scene_lights;
#[cfg(test)]
mod scene_tests;
mod spot_shadow;
mod stage;
pub(crate) mod stage_framing;
mod stage_renderer;
#[cfg(test)]
pub(crate) mod test_support;
mod tuning;

pub use art::{LoadingArt, ModelTransform};
pub use camera::ArtCamera;
pub use controls::ModelControls;
pub use grade::LoadScreenLut;
pub use model::{CpuArt, LoadedArt, LoadingArtError};
pub use motion::PanKey;
pub use scene::{GradeSource, LoadingScene, PICTURE_FORMAT};
pub use scene_lights::SceneLights;
pub use stage::StageError;

use bytemuck::bytes_of;
use wgpu::util::DeviceExt;

use crate::rendering::shadows::lamps::LampShadowConfig;
use pipeline::{ArtPipeline, DepthTarget, MaterialUniform, SceneUniform};
use scene_lights::{GpuSceneLight, LIGHT_COUNT};
use spot_shadow::{Casters, LightShadowFrame, SpotShadows};

struct Active {
    art: LoadedArt,
    texture_groups: Vec<wgpu::BindGroup>,
}

pub struct LoadingArtRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    lights: [GpuSceneLight; LIGHT_COUNT],
    lamp_shadows: LampShadowConfig,
    pipeline: ArtPipeline,
    shadows: SpotShadows,
    sampler: wgpu::Sampler,
    scene_buffer: wgpu::Buffer,
    scene_group: wgpu::BindGroup,
    depth: Option<DepthTarget>,
    active: Option<Active>,
    model_controls: ModelControls,
}

impl LoadingArtRenderer {
    /// A renderer for views of `target_format`, lit by `lights`, whose shadows are filtered and
    /// biased as the world's lamp shadows of `lamp_shadows`. It writes linear light, so the
    /// target is the scene's float picture.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        lights: &SceneLights,
        lamp_shadows: &LampShadowConfig,
    ) -> Self {
        let pipeline = ArtPipeline::new(device, target_format);
        let shadows = SpotShadows::new(
            device,
            &pipeline.shadow_layout,
            &pipeline.scene_layout,
            &pipeline.texture_layout,
        );
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
            lights: lights.to_gpu(),
            lamp_shadows: *lamp_shadows,
            pipeline,
            shadows,
            sampler,
            scene_buffer,
            scene_group,
            depth: None,
            active: None,
            model_controls: ModelControls::VANILLA,
        }
    }

    /// Shows `art` from the next render on, with a camera back at its placement as stored;
    /// `None` shows nothing.
    pub fn set_art(&mut self, art: Option<LoadedArt>) {
        self.active = art.map(|mut art| {
            art.camera
                .set_spin_resume_delay(self.model_controls.spin_resume_delay_seconds);
            let texture_groups = art
                .draws
                .iter()
                .map(|draw| {
                    let material =
                        self.device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("loading_art_material"),
                                contents: bytes_of(&MaterialUniform::new(
                                    draw.alpha_cutoff,
                                    draw.smoothness,
                                    draw.specular_scale,
                                )),
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
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::TextureView(&draw.normal),
                            },
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: wgpu::BindingResource::TextureView(&draw.specular),
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

    /// The viewer let go of the model.
    pub fn drag_ended(&mut self) {
        if let Some(active) = &mut self.active {
            active.art.camera.drag_ended();
        }
    }

    /// How the settings shape the control of the models shown from now on.
    pub fn set_model_controls(&mut self, controls: ModelControls) {
        self.model_controls = controls;
    }

    /// Puts the model's camera back at its start and stops the spin.
    pub fn pin_camera(&mut self) {
        if let Some(active) = &mut self.active {
            active.art.camera.pin();
        }
    }

    /// A pan key went down or up.
    pub fn set_pan_key(&mut self, key: PanKey, held: bool) {
        if let Some(active) = &mut self.active {
            active.art.camera.set_pan_key(key, held);
        }
    }

    /// Lets go of every held pan key.
    pub fn release_pan_keys(&mut self) {
        if let Some(active) = &mut self.active {
            active.art.camera.release_pan_keys();
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
    /// its own, seen through a vertical field of `vertical_fov_radians`. Nothing is encoded
    /// without a model or for an empty target.
    pub fn render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        size: (u32, u32),
        vertical_fov_radians: f32,
    ) {
        let Some(active) = &self.active else { return };
        if size.0 == 0 || size.1 == 0 {
            return;
        }
        let aspect = size.0 as f32 / size.1 as f32;
        let aim = active.art.camera.spot_aim();
        let model = active.art.camera.model_matrix();
        let scene = SceneUniform::new(
            active.art.camera.view_proj(aspect, vertical_fov_radians),
            model,
            self.lights,
            aim,
        );
        self.queue
            .write_buffer(&self.scene_buffer, 0, bytes_of(&scene));
        self.shadows.render(
            &self.queue,
            encoder,
            &LightShadowFrame::of(&self.lights, aim, &self.lamp_shadows),
            &Casters {
                scene_group: &self.scene_group,
                vertices: &active.art.vertices,
                indices: &active.art.indices,
                draws: &active.art.draws,
                texture_groups: &active.texture_groups,
            },
        );
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
        pass.set_bind_group(2, &self.shadows.receiver_group, &[]);
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
