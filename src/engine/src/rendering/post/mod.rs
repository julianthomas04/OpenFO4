pub mod auto_exposure;
pub mod bokeh_blur;
pub mod bokeh_dof;
pub mod depth_of_field;
mod dof_depth_merge;
pub mod dof_resolve;
mod dof_support;
#[cfg(test)]
mod dof_test_support;
pub mod hdr_target;
pub mod night_sky;
pub mod sky;
pub mod tonemap;

use crate::rendering::{
    device_loss::DeviceLossMonitor,
    gpu_profiler::GpuProfiler,
    lighting::{FrameBinding, state::ExposureReadout},
    readback_ring::ReadbackError,
};
use auto_exposure::{AutoExposure, AutoExposureParams};
use bokeh_dof::BokehDepthOfField;
use depth_of_field::DofFrame;
use glam::Vec2;
use hdr_target::HdrTarget;
use sky::SkyPipeline;
use tonemap::TonemapPass;

/// Everything between the world pass and the swapchain: the HDR colour target,
/// the sky drawn into it, the auto exposure measuring it and the tonemap
/// resolving it to the display.
pub struct PostChain {
    hdr_target: HdrTarget,
    sky: SkyPipeline,
    auto_exposure: AutoExposure,
    depth_of_field: BokehDepthOfField,
    tonemap: TonemapPass,
}

impl PostChain {
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        frame_layout: &wgpu::BindGroupLayout,
        world_pass_layout: &wgpu::PipelineLayout,
    ) -> Self {
        let hdr_target = HdrTarget::new(device, width, height);
        let sky = SkyPipeline::new(device, world_pass_layout);
        let auto_exposure = AutoExposure::new(device, &hdr_target.view);
        let depth_of_field = BokehDepthOfField::new(device, width, height);
        let tonemap = TonemapPass::new(
            device,
            surface_format,
            frame_layout,
            &hdr_target.view,
            &auto_exposure.state,
        );

        Self {
            hdr_target,
            sky,
            auto_exposure,
            depth_of_field,
            tonemap,
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.hdr_target = HdrTarget::new(device, width, height);
        self.auto_exposure
            .rebind_source(device, &self.hdr_target.view);
        self.depth_of_field.resize(device, width, height);
        self.tonemap
            .rebind_source(device, &self.hdr_target.view, &self.auto_exposure.state);
    }

    pub fn write_exposure_params(&self, queue: &wgpu::Queue, params: &AutoExposureParams) {
        self.auto_exposure.write_params(queue, params);
    }

    pub fn encode_auto_exposure(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        profiler: &GpuProfiler,
    ) {
        self.auto_exposure.encode(encoder, profiler);
    }

    pub fn post_submit(&mut self) {
        self.auto_exposure.post_submit();
    }

    pub fn poll_exposure(
        &mut self,
        device: &wgpu::Device,
        device_loss: &DeviceLossMonitor,
    ) -> Result<Option<ExposureReadout>, ReadbackError> {
        self.auto_exposure.poll(device, device_loss)
    }

    pub fn hdr_texture(&self) -> &wgpu::Texture {
        self.hdr_target.texture()
    }

    pub fn hdr_view(&self) -> &wgpu::TextureView {
        &self.hdr_target.view
    }

    /// The scene target in pixels.
    pub fn target_size(&self) -> Vec2 {
        let size = self.hdr_target.texture().size();
        Vec2::new(size.width as f32, size.height as f32)
    }

    /// The bokeh depth of field over the finished scene. `depth` is the live
    /// scene depth, `surface_depth` the underwater surface depth, if any.
    pub fn encode_depth_of_field(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        surface_depth: Option<&wgpu::TextureView>,
        frame: &DofFrame,
    ) {
        self.depth_of_field.encode(
            device,
            queue,
            encoder,
            self.hdr_target.texture(),
            &self.hdr_target.view,
            depth,
            surface_depth,
            &frame.registers,
            frame.resolve,
        );
    }

    pub fn draw_sky(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.sky.draw(pass);
    }

    pub fn tonemap_target_format(&self) -> wgpu::TextureFormat {
        self.tonemap.target_format()
    }

    pub fn encode_tonemap(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        frame: &FrameBinding,
        profiler: &GpuProfiler,
    ) {
        self.tonemap.encode(encoder, target, frame, profiler);
    }
}
