//! The rain of the world: the weather's rain is worked out every frame and stirs the water
//! around the player, and later in the frame it is drawn.

use glam::Vec3;

use super::camera::Camera;
use super::gpu_profiler::GpuProfiler;
use super::lighting::frame_binding::FrameBinding;
use super::rain::{RainAssetStatus, RainGpu, RainInput, RainRenderer, RainView};
use super::water::WaterStage;
use super::weather::WeatherSync;
use super::weather::panel::RainControls;
use super::weather::rain::Ripples;

/// What the rain of a frame is worked out from.
pub struct RainFrame<'a> {
    pub weather: &'a WeatherSync,
    pub water: &'a WaterStage,
    pub controls: RainControls,
    /// The radiance the rain catches, lightning flash included.
    pub tint: Vec3,
    /// Whether the camera is in an interior, where no rain falls.
    pub indoors: bool,
    /// The world seconds of the frame, which stop with the Pip-Boy open.
    pub world_dt: f32,
    /// Where the water displacement takes its impacts from: the player, else the camera.
    pub origin: Vec3,
}

pub struct RainStage {
    renderer: RainRenderer,
    input: Option<RainInput>,
    /// World seconds the rain has seen.
    seconds: f64,
    ripples: Ripples,
}

impl RainStage {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        asset_root: &str,
        frame_layout: &wgpu::BindGroupLayout,
        seed: u64,
    ) -> Self {
        Self {
            renderer: RainRenderer::new(device, queue, asset_root, frame_layout),
            input: None,
            seconds: 0.0,
            ripples: Ripples::new(seed),
        }
    }

    /// Works out the rain of the frame: none in an interior or when it is switched off. Adds the
    /// water entries the rain makes to `water_entries`.
    pub fn sync(&mut self, frame: &RainFrame<'_>, water_entries: &mut Vec<Vec3>) {
        self.seconds += f64::from(frame.world_dt);
        self.input = (frame.controls.draw && !frame.indoors)
            .then(|| frame.weather.rain_input(frame.tint))
            .flatten();
        let Some(input) = &self.input else {
            return;
        };
        if frame.controls.ripples
            && frame.water.wants_water_entries()
            && !frame.water.camera_water().underwater
        {
            self.ripples.append_entries(
                water_entries,
                frame.world_dt,
                input.intensity,
                frame.origin,
                |point| frame.water.surface_height_at(point),
            );
        }
    }

    /// Records what the rain of the frame needs ahead of the passes; nothing without rain.
    pub fn prepare(
        &mut self,
        gpu: RainGpu<'_>,
        camera: &Camera,
        world_generation: u64,
        underwater: bool,
    ) {
        let Some(input) = &self.input else {
            return;
        };
        let view = RainView {
            eye: camera.eye,
            forward: camera.forward(),
            time: self.seconds,
            world_generation,
            underwater,
        };
        self.renderer.prepare(gpu, input, &view);
    }

    pub fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        frame_binding: &FrameBinding,
        profiler: &GpuProfiler,
    ) {
        self.renderer
            .encode(encoder, target, depth, frame_binding, profiler);
    }

    pub fn asset_status(&self) -> &RainAssetStatus {
        self.renderer.asset_status()
    }
}
