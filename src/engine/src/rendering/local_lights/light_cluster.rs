use glam::{Mat4, Vec4};

use super::buffers::ClusterBuffers;
use super::cluster_grid::{CLUSTER_TILES_X, CLUSTER_TILES_Y};
use crate::rendering::{
    device::STORAGE_BUFFERS_PER_STAGE,
    gpu_cull::{storage_entry, uniform_entry},
    gpu_profiler::{GpuPass, GpuProfiler, SpanPart},
    shaders,
};

/// Visible lights, counters, cluster counts, cluster indices and the
/// light-sphere draw arguments.
const STORAGE_BUFFERS: u32 = 5;
const _: () = assert!(STORAGE_BUFFERS <= STORAGE_BUFFERS_PER_STAGE);

/// Mirrors `LightClusterUniforms` in `shaders/light_cluster.wgsl`.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LightClusterUniforms {
    pub view: Mat4,
    /// x: projection x scale, y: projection y scale.
    pub projection_scale: Vec4,
    /// See `ClusterDepth::uniform`.
    pub cluster_z: Vec4,
}

const _: () = assert!(std::mem::size_of::<LightClusterUniforms>() == 96);

/// Byte offset of the instance count in the sphere draw arguments.
const SPHERE_INSTANCE_COUNT_OFFSET: u64 = std::mem::size_of::<u32>() as u64;

/// One workgroup per screen tile builds the sorted light list of each of the
/// tile's depth slices from `visible_lights`.
pub struct LightClusterPipeline {
    pipeline: wgpu::ComputePipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl LightClusterPipeline {
    pub fn new(device: &wgpu::Device, buffers: &ClusterBuffers) -> Self {
        let shader = shaders::LIGHT_CLUSTER.compile(device);
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("light_cluster_bgl"),
            entries: &[
                storage_entry(0, true),
                storage_entry(1, false),
                storage_entry(2, false),
                storage_entry(3, false),
                storage_entry(4, false),
                uniform_entry(STORAGE_BUFFERS),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("light_cluster_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("light_cluster_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("light_cluster_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("light_cluster_uniforms"),
            size: std::mem::size_of::<LightClusterUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let resources = [
            &buffers.visible_lights,
            &buffers.counters,
            &buffers.cluster_light_counts,
            &buffers.cluster_light_indices,
            &buffers.sphere_draw_args,
            &uniform_buffer,
        ];
        let entries: Vec<_> = resources
            .into_iter()
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect();
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("light_cluster_bg"),
            layout: &bind_group_layout,
            entries: &entries,
        });

        Self {
            pipeline,
            uniform_buffer,
            bind_group,
        }
    }

    /// Runs every frame local lights are on and at least one is placed.
    pub fn dispatch(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        uniforms: &LightClusterUniforms,
        profiler: &GpuProfiler,
    ) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(uniforms));
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("light_cluster"),
            timestamp_writes: profiler.compute_writes(GpuPass::LightCluster, SpanPart::Whole),
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.dispatch_workgroups(CLUSTER_TILES_X, CLUSTER_TILES_Y, 1);
    }

    /// Empty-scene replacement for `dispatch`: zeroes every cluster count and
    /// the sphere instance count so no list from an earlier frame survives.
    #[allow(
        clippy::unused_self,
        reason = "mirrors the signature of dispatch so callers treat both passes alike"
    )]
    pub fn clear(&self, encoder: &mut wgpu::CommandEncoder, buffers: &ClusterBuffers) {
        encoder.clear_buffer(&buffers.cluster_light_counts, 0, None);
        encoder.clear_buffer(
            &buffers.sphere_draw_args,
            SPHERE_INSTANCE_COUNT_OFFSET,
            Some(std::mem::size_of::<u32>() as u64),
        );
    }
}
