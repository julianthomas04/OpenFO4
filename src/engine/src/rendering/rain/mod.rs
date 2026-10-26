//! Rain: the top-down occlusion map that tells which spots lie under a roof, and the passes that
//! draw the rain from it. The frame calls [`RainRenderer::prepare`] once ahead of its passes,
//! which draws the map when it is due and works the rain out from what the map then shows, and
//! [`RainRenderer::encode`] to draw that rain.
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
mod tuning;
mod uniform;

use std::sync::Arc;

use glam::{UVec2, Vec2, Vec3};
use wgpu::util::DeviceExt;

pub use assets::{RainAssetStatus, RainTextureState};
use motion::DropDrift;
pub use motion::wind_drift;
use occlusion::OcclusionMap;
use pipeline::{QUAD_VERTICES, RainPipelines, RainResources};
use uniform::{Draws, Fall, RainUniform};

use crate::rendering::gpu_profiler::{GpuPass, GpuProfiler, SpanPart};
use crate::rendering::lighting::frame_binding::FrameBinding;
use crate::rendering::shadows::{ShadowRenderer, ShadowScene};

/// The rain of the weather, as its shader particle geometry (SPGD) record and the weather
/// state give it.
#[derive(Debug, Clone, PartialEq)]
pub struct RainInput {
    /// 0 (dry) to 1 (downpour): how many drops and splashes there are. Interiors have no rain,
    /// but nothing here tells them from the open air: the caller skips [`RainRenderer::prepare`]
    /// there, or passes 0.
    pub intensity: f32,
    /// Whether roofs shelter from the rain (the weather's rain occlusion flag).
    pub occludable: bool,
    /// Game units per second the drops fall at (the SPGD's gravity velocity).
    pub fall_speed: f32,
    /// The SPGD particle size, in metres before the streak scales of the tuning: x along the
    /// fall, y across it.
    pub size: Vec2,
    /// The columns and rows of streak variants the texture holds; 0 counts as 1.
    pub subtextures: UVec2,
    /// The SPGD particle density: how many drops fall at full intensity, by the tuning's drops
    /// per unit of density.
    pub density: f32,
    /// The edge in game units of the box around the camera the rain falls through.
    pub box_size: f32,
    /// The streak texture, a path below the textures folder (`Effects\FXRaindrops_D.DDS`);
    /// `None` for a weather without one.
    pub texture: Option<Arc<str>>,
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
    /// The streamer's loaded generation: the static world the occlusion map shows. The map is
    /// drawn again once it moves on.
    pub world_generation: u64,
    /// Whether the camera is under water, where no rain falls and no splash is seen.
    pub underwater: bool,
}

/// What the rain of a frame records its GPU work with: the encoder the frame goes into, and
/// what the casters of the occlusion map are drawn from.
pub struct RainGpu<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub shadows: &'a ShadowRenderer,
    pub scene: &'a ShadowScene<'a>,
    pub profiler: &'a GpuProfiler,
}

/// Draws the rain streaks and the splashes where they land, in a pass of its own after the
/// translucent pass.
pub struct RainRenderer {
    pipelines: RainPipelines,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    map: OcclusionMap,
    /// Stands in for a texture that is not loaded; nothing is drawn with it.
    blank: wgpu::TextureView,
    textures: assets::RainTextures,
    bind_group: wgpu::BindGroup,
    drift: DropDrift,
    draws: Draws,
}

impl RainRenderer {
    /// `frame_layout` is the layout of the frame binding the pass binds at group 1.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        asset_root: &str,
        frame_layout: &wgpu::BindGroupLayout,
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
        let map = OcclusionMap::new(device);
        let bind_group = bind_group(
            device,
            &pipelines,
            &uniform,
            &sampler,
            &textures,
            &blank,
            map.view(),
        );
        Self {
            pipelines,
            sampler,
            uniform,
            map,
            blank,
            textures,
            bind_group,
            drift: DropDrift::default(),
            draws: Draws::NONE,
        }
    }

    /// Works out this frame's rain ahead of the passes that draw it. The occlusion map is drawn
    /// into the frame's encoder first when it is due, and the rain is worked out from the window
    /// that map shows. Nothing is drawn, loaded or uploaded while it does not rain, the camera
    /// is under water or the camera is no point in space. Interiors, where no rain falls, are the
    /// caller's to tell: it skips this call there, or passes an intensity of 0.
    pub fn prepare(&mut self, mut gpu: RainGpu<'_>, input: &RainInput, view: &RainView) {
        let fall = Fall::of(input);
        self.drift.advance(fall.velocity, view.time, fall.edge);
        let wanted = Draws::wanted(input, view);
        self.draws = Draws::NONE;
        if wanted.is_none() {
            return;
        }
        let window = self.map.refresh(&mut gpu, view.eye, view.world_generation);
        if self
            .textures
            .request_streaks(gpu.device, gpu.queue, input.texture.as_deref())
        {
            self.bind_group = bind_group(
                gpu.device,
                &self.pipelines,
                &self.uniform,
                &self.sampler,
                &self.textures,
                &self.blank,
                self.map.view(),
            );
        }
        let uniform = RainUniform::new(input, view, window, &fall, &self.drift);
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
        self.draws = Draws {
            drops: self.textures.streaks().map_or(0, |_| wanted.drops),
            splashes: self.textures.splashes().map_or(0, |_| wanted.splashes),
        };
    }

    /// Draws the rain over `target` where the scene's `depth` does not hide it: the rain the
    /// last [`Self::prepare`] worked out, once. A frame that does not prepare draws none.
    /// Encodes nothing while there is nothing to draw. Timed as [`GpuPass::Rain`].
    pub fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        frame_binding: &FrameBinding,
        profiler: &GpuProfiler,
    ) {
        let draws = std::mem::replace(&mut self.draws, Draws::NONE);
        if draws.is_none() {
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
        if draws.drops > 0 {
            pass.set_pipeline(&self.pipelines.streaks);
            pass.draw(0..QUAD_VERTICES, 0..draws.drops);
        }
        if draws.splashes > 0 {
            pass.set_pipeline(&self.pipelines.splashes);
            pass.draw(0..QUAD_VERTICES, 0..draws.splashes);
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
