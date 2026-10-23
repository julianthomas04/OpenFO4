pub mod buffers;
pub mod cluster_grid;
pub mod debug_panel;
mod light_cluster;
mod light_cull;
pub mod light_spheres;
pub mod settings;
pub mod stats;

pub use buffers::ClusterBuffers;
pub use settings::{LightFade, LocalLightSettings};

use glam::{Mat4, UVec2, Vec4};

use crate::rendering::{
    camera::Camera, device_loss::DeviceLossMonitor, gpu_profiler::GpuProfiler,
    readback_ring::ReadbackError,
};
use cluster_grid::ClusterDepth;
use light_cluster::{LightClusterPipeline, LightClusterUniforms};
use light_cull::{LightCullPipeline, LightCullUniforms};
use light_spheres::LightSpherePipeline;
use stats::{LocalLightCounters, LocalLightStatsReadback};

/// Per-frame inputs of the light cull and cluster build.
pub struct LocalLightFrame<'a> {
    pub camera: &'a Camera,
    pub view_proj: Mat4,
    pub frustum_planes: [[f32; 4]; 5],
    pub screen_size: [f32; 2],
    pub hiz_mip_count: u32,
    pub hiz_valid: bool,
    /// Placed-light slab dispatch bound.
    pub slot_count: u32,
    /// Live placed lights; zero lets the cluster build be skipped.
    pub lights_loaded: u32,
    pub settings: &'a LocalLightSettings,
    /// (placed-light slab slot, lamp-shadow table slot) of every sampled lamp.
    pub shadow_lamps: &'a [UVec2],
}

/// Clustered forward shading of placed lights: the light cull fills
/// `visible_lights`, the cluster build fills the per-cluster lists, and the
/// world pass reads both through the frame group.
pub struct LocalLightRenderer {
    buffers: ClusterBuffers,
    cull: LightCullPipeline,
    cull_bind_group: wgpu::BindGroup,
    cluster: LightClusterPipeline,
    spheres: LightSpherePipeline,
    stats: LocalLightStatsReadback,
}

impl LocalLightRenderer {
    pub fn new(
        device: &wgpu::Device,
        buffers: ClusterBuffers,
        placed_lights: &wgpu::Buffer,
        hiz_view: &wgpu::TextureView,
        color_format: wgpu::TextureFormat,
        world_pass_layout: &wgpu::PipelineLayout,
    ) -> Self {
        let cull = LightCullPipeline::new(device);
        let cull_bind_group = cull.create_bind_group(device, placed_lights, &buffers, hiz_view);
        let cluster = LightClusterPipeline::new(device, &buffers);

        Self {
            cull,
            cull_bind_group,
            cluster,
            spheres: LightSpherePipeline::new(device, color_format, world_pass_layout),
            stats: LocalLightStatsReadback::new(device),
            buffers,
        }
    }

    /// After the placed-light slab grew or the depth pyramid was recreated.
    pub fn rebind_cull(
        &mut self,
        device: &wgpu::Device,
        placed_lights: &wgpu::Buffer,
        hiz_view: &wgpu::TextureView,
    ) {
        self.cull_bind_group =
            self.cull
                .create_bind_group(device, placed_lights, &self.buffers, hiz_view);
    }

    /// Nothing runs while local lights are off; the frame uniform then tells
    /// the shaders not to read the cluster lists.
    pub fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        frame: &LocalLightFrame,
        profiler: &GpuProfiler,
    ) {
        if !frame.settings.enabled {
            return;
        }
        encoder.clear_buffer(&self.buffers.counters, 0, None);
        self.cull.dispatch(
            encoder,
            queue,
            &self.cull_bind_group,
            &LightCullUniforms::new(frame),
            profiler,
        );
        if frame.lights_loaded == 0 {
            self.cluster.clear(encoder, &self.buffers);
        } else {
            self.cluster
                .dispatch(encoder, queue, &cluster_uniforms(frame), profiler);
        }
        self.stats.encode_copy(encoder, &self.buffers.counters);
    }

    pub fn draw_spheres(&self, pass: &mut wgpu::RenderPass<'_>, settings: &LocalLightSettings) {
        if settings.enabled && settings.show_spheres {
            self.spheres.draw(pass, &self.buffers);
        }
    }

    pub fn post_submit(&mut self) {
        self.stats.post_submit();
    }

    pub fn poll(
        &mut self,
        device: &wgpu::Device,
        device_loss: &DeviceLossMonitor,
    ) -> Result<(), ReadbackError> {
        self.stats.poll(device, device_loss)
    }

    pub fn counters(&self) -> &LocalLightCounters {
        &self.stats.latest
    }
}

fn cluster_uniforms(frame: &LocalLightFrame) -> LightClusterUniforms {
    let projection = frame.camera.projection_matrix();
    LightClusterUniforms {
        view: frame.camera.view_matrix(),
        projection_scale: Vec4::new(projection.x_axis.x, projection.y_axis.y, 0.0, 0.0),
        cluster_z: ClusterDepth::for_settings(frame.settings).uniform(),
    }
}
