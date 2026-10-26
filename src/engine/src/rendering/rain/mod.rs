//! Rain: the top-down occlusion map that tells which spots lie under a roof, and the passes that
//! draw the rain from it. Everything here takes plain data; the frame decides when it runs.
//!
//! The drops and splashes are worked out on the GPU from their instance index: the CPU hands
//! over one uniform per frame and the pass draws instanced quads, so the cost does not grow
//! with the number of drops.

mod assets;
pub mod impacts;
mod motion;
pub mod occlusion;
mod pipeline;
#[cfg(test)]
mod tests;
mod uniform;

use std::sync::Arc;

use glam::{UVec2, Vec2, Vec3};
use wgpu::util::DeviceExt;

pub use assets::{RainAssetStatus, RainTextureState};
use motion::DropDrift;
use occlusion::OcclusionWindow;
use pipeline::{QUAD_VERTICES, RainPipelines, RainResources};
use uniform::{Fall, RainPlan, RainUniform};

use crate::bethesda_space::UNITS_TO_ENGINE;
use crate::rendering::gpu_profiler::{GpuPass, GpuProfiler, SpanPart};
use crate::rendering::lighting::frame_binding::FrameBinding;

/// Bethesda game units to the metre.
const GAME_UNITS_PER_METRE: f32 = 70.0;

/// Engine units to the metre: the scale the rain constants are written in.
pub const ENGINE_UNITS_PER_METRE: f32 = GAME_UNITS_PER_METRE * UNITS_TO_ENGINE;

/// The rain of the weather, as its shader particle geometry (SPGD) record and the weather
/// state give it.
#[derive(Debug, Clone, PartialEq)]
pub struct RainInput {
    /// 0 (dry) to 1 (downpour): how many drops and splashes there are.
    pub intensity: f32,
    /// Whether roofs shelter from the rain (the weather's rain occlusion flag).
    pub occludable: bool,
    /// Game units per second the drops fall at (the SPGD's gravity velocity).
    pub fall_speed: f32,
    /// The SPGD particle size in metres: x the width of a drop's card, y its length along the
    /// fall.
    pub size: Vec2,
    /// The columns and rows of streak variants the texture holds; 0 counts as 1.
    pub subtextures: UVec2,
    /// The SPGD particle density: a thousand drops per unit at full intensity.
    pub density: f32,
    /// The edge in game units of the box around the camera the rain falls through.
    pub box_size: f32,
    /// The streak texture, a path below the textures folder (`Effects\FXRaindrops_D.DDS`);
    /// empty for a weather without one.
    pub texture: Arc<str>,
    /// The horizontal drift of a drop on the engine's x and z axes, in game units per second.
    pub wind: Vec2,
    /// Linear colour of the rain: the light it catches, with any lightning flash.
    pub tint: Vec3,
}

/// The camera and the clock of the frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RainView {
    pub eye: Vec3,
    pub forward: Vec3,
    /// Seconds on any clock that runs on with the game, in double precision.
    pub time: f64,
    /// What the occlusion map shows, `None` until it was first drawn: without it the rain
    /// falls through every roof and no splash is drawn.
    pub window: Option<OcclusionWindow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Draws {
    drops: u32,
    splashes: u32,
}

impl Draws {
    const NONE: Self = Self {
        drops: 0,
        splashes: 0,
    };

    fn is_none(self) -> bool {
        self == Self::NONE
    }
}

/// Draws the rain streaks and the splashes where they land, in a pass of its own after the
/// translucent pass.
pub struct RainRenderer {
    pipelines: RainPipelines,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    occlusion: wgpu::TextureView,
    /// Stands in for a texture that is not loaded; nothing is drawn with it.
    blank: wgpu::TextureView,
    textures: assets::RainTextures,
    bind_group: wgpu::BindGroup,
    drift: DropDrift,
    draws: Draws,
}

impl RainRenderer {
    /// `frame_layout` is the layout of the frame binding the pass binds at group 1, and
    /// `occlusion` the depth view of the [`occlusion::OcclusionMap`].
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        asset_root: &str,
        frame_layout: &wgpu::BindGroupLayout,
        occlusion: &wgpu::TextureView,
    ) -> Self {
        let pipelines = RainPipelines::new(device, frame_layout);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rain_uniform"),
            size: std::mem::size_of::<RainUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = pipeline::sampler(device);
        let blank = blank_texture(device, queue);
        let textures = assets::RainTextures::new(device, queue, asset_root);
        let bind_group = bind_group(
            device, &pipelines, &uniform, &sampler, &textures, &blank, occlusion,
        );
        Self {
            pipelines,
            sampler,
            uniform,
            occlusion: occlusion.clone(),
            blank,
            textures,
            bind_group,
            drift: DropDrift::default(),
            draws: Draws::NONE,
        }
    }

    /// Works out this frame's rain. Nothing is loaded or uploaded while it does not rain.
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        input: &RainInput,
        view: &RainView,
    ) {
        let fall = Fall::of(input);
        self.drift.advance(fall.velocity, view.time, fall.edge);
        let plan = RainPlan::new(input, view, &fall, &self.drift);
        self.draws = Draws::NONE;
        if plan.drops == 0 && plan.splashes == 0 {
            return;
        }
        if self.textures.request_streaks(device, queue, &input.texture) {
            self.bind_group = bind_group(
                device,
                &self.pipelines,
                &self.uniform,
                &self.sampler,
                &self.textures,
                &self.blank,
                &self.occlusion,
            );
        }
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&plan.uniform));
        self.draws = Draws {
            drops: self.textures.streaks().map_or(0, |_| plan.drops),
            splashes: self.textures.splashes().map_or(0, |_| plan.splashes),
        };
    }

    /// Draws the rain over `target` where the scene's `depth` does not hide it. Encodes nothing
    /// while there is nothing to draw. Timed as [`GpuPass::Rain`].
    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        frame_binding: &FrameBinding,
        profiler: &GpuProfiler,
    ) {
        if self.draws.is_none() {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Rain Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: None,
                stencil_ops: None,
            }),
            timestamp_writes: profiler.render_writes(GpuPass::Rain, SpanPart::Whole),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &self.bind_group, &[]);
        frame_binding.bind(&mut pass);
        if self.draws.drops > 0 {
            pass.set_pipeline(&self.pipelines.streaks);
            pass.draw(0..QUAD_VERTICES, 0..self.draws.drops);
        }
        if self.draws.splashes > 0 {
            pass.set_pipeline(&self.pipelines.splashes);
            pass.draw(0..QUAD_VERTICES, 0..self.draws.splashes);
        }
    }

    pub fn asset_status(&self) -> &RainAssetStatus {
        self.textures.status()
    }
}

fn bind_group(
    device: &wgpu::Device,
    pipelines: &RainPipelines,
    uniform: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
    textures: &assets::RainTextures,
    blank: &wgpu::TextureView,
    occlusion: &wgpu::TextureView,
) -> wgpu::BindGroup {
    pipeline::bind_group(
        device,
        &pipelines.layout,
        &RainResources {
            uniform,
            streaks: textures.streaks().unwrap_or(blank),
            sampler,
            splashes: textures.splashes().unwrap_or(blank),
            occlusion,
        },
    )
}

fn blank_texture(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("rain_blank_texture"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &[0; 4],
        )
        .create_view(&wgpu::TextureViewDescriptor::default())
}
