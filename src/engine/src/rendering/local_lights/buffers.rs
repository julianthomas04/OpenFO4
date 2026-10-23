use wgpu::util::DeviceExt;

use super::cluster_grid::{CLUSTER_COUNT, MAX_LIGHTS_PER_CLUSTER, MAX_VISIBLE_LIGHTS};
use super::light_spheres::SPHERE_VERTEX_COUNT;

/// Mirrors `VisibleLight` in `shaders/light_data.wgsl`.
const VISIBLE_LIGHT_BYTES: u64 = 80;
const U32_BYTES: u64 = std::mem::size_of::<u32>() as u64;
/// Mirrors `LightCounters` in `shaders/light_data.wgsl`: the visible-light
/// append counter (counts past the capacity too) and the overflowing clusters.
pub const COUNTER_BYTES: u64 = 2 * U32_BYTES;

/// The fixed-size outputs of the light cull and the cluster build. Nothing
/// here is ever reallocated, so the frame bind group referencing them is
/// built once.
pub struct ClusterBuffers {
    pub visible_lights: wgpu::Buffer,
    pub counters: wgpu::Buffer,
    /// One light count per cluster.
    pub cluster_light_counts: wgpu::Buffer,
    /// `MAX_LIGHTS_PER_CLUSTER` ascending visible-light indices per cluster.
    pub cluster_light_indices: wgpu::Buffer,
    /// `DrawIndirectArgs` of the light-sphere debug lines; the cluster build
    /// writes the instance count.
    pub sphere_draw_args: wgpu::Buffer,
}

impl ClusterBuffers {
    pub fn new(device: &wgpu::Device) -> Self {
        let storage = |label: &str, size: u64, extra: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE | extra,
                mapped_at_creation: false,
            })
        };
        let cluster_count = u64::from(CLUSTER_COUNT);

        Self {
            visible_lights: storage(
                "visible_lights",
                u64::from(MAX_VISIBLE_LIGHTS) * VISIBLE_LIGHT_BYTES,
                wgpu::BufferUsages::empty(),
            ),
            counters: storage(
                "light_counters",
                COUNTER_BYTES,
                wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            ),
            cluster_light_counts: storage(
                "cluster_light_counts",
                cluster_count * U32_BYTES,
                wgpu::BufferUsages::COPY_DST,
            ),
            cluster_light_indices: storage(
                "cluster_light_indices",
                cluster_count * u64::from(MAX_LIGHTS_PER_CLUSTER) * U32_BYTES,
                wgpu::BufferUsages::empty(),
            ),
            sphere_draw_args: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("light_sphere_draw_args"),
                contents: bytemuck::cast_slice(&[SPHERE_VERTEX_COUNT, 0, 0, 0]),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::INDIRECT
                    | wgpu::BufferUsages::COPY_DST,
            }),
        }
    }
}
