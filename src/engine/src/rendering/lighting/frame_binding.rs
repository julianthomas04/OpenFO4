use wgpu::util::DeviceExt;

use super::frame_uniform::FrameUniform;
use crate::rendering::local_lights::ClusterBuffers;
use crate::rendering::shadows::lamps::table::GpuShadowLampTable;
use crate::rendering::shadows::shadow_maps::ShadowMaps;

const FRAME_GROUP: u32 = 1;
const UNIFORM_BINDING: u32 = 0;
const SHADOW_MAP_BINDING: u32 = 1;
const SHADOW_SAMPLER_BINDING: u32 = 2;
const VISIBLE_LIGHTS_BINDING: u32 = 3;
const CLUSTER_LIGHT_COUNTS_BINDING: u32 = 4;
const CLUSTER_LIGHT_INDICES_BINDING: u32 = 5;
const LAMP_SHADOW_ATLAS_BINDING: u32 = 6;
const LAMP_SHADOW_TABLE_BINDING: u32 = 7;
/// Storage buffers the frame group adds to the fragment stage.
pub const FRAME_STORAGE_BUFFERS: u32 = 3;

fn read_only_storage(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform_entry<T>(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<T>() as u64),
        },
        count: None,
    }
}

fn depth_texture_entry(
    binding: u32,
    view_dimension: wgpu::TextureViewDimension,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension,
            multisampled: false,
        },
        count: None,
    }
}

pub struct FrameBinding {
    pub layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    buffer: wgpu::Buffer,
    lamp_shadow_buffer: wgpu::Buffer,
}

impl FrameBinding {
    pub fn new(
        device: &wgpu::Device,
        initial: &FrameUniform,
        shadow_maps: &ShadowMaps,
        lamp_shadow_atlas: &wgpu::TextureView,
        clusters: &ClusterBuffers,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame_bgl"),
            entries: &[
                uniform_entry::<FrameUniform>(UNIFORM_BINDING, wgpu::ShaderStages::VERTEX_FRAGMENT),
                depth_texture_entry(SHADOW_MAP_BINDING, wgpu::TextureViewDimension::D2Array),
                wgpu::BindGroupLayoutEntry {
                    binding: SHADOW_SAMPLER_BINDING,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                // The light-sphere debug lines read the visible lights per vertex.
                read_only_storage(VISIBLE_LIGHTS_BINDING, wgpu::ShaderStages::VERTEX_FRAGMENT),
                read_only_storage(CLUSTER_LIGHT_COUNTS_BINDING, wgpu::ShaderStages::FRAGMENT),
                read_only_storage(CLUSTER_LIGHT_INDICES_BINDING, wgpu::ShaderStages::FRAGMENT),
                depth_texture_entry(LAMP_SHADOW_ATLAS_BINDING, wgpu::TextureViewDimension::D2),
                uniform_entry::<GpuShadowLampTable>(
                    LAMP_SHADOW_TABLE_BINDING,
                    wgpu::ShaderStages::FRAGMENT,
                ),
            ],
        });

        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("frame_uniform_buffer"),
            contents: bytemuck::bytes_of(initial),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let lamp_shadow_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lamp_shadow_table"),
            size: std::mem::size_of::<GpuShadowLampTable>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame_bind_group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: UNIFORM_BINDING,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: SHADOW_MAP_BINDING,
                    resource: wgpu::BindingResource::TextureView(shadow_maps.array_view()),
                },
                wgpu::BindGroupEntry {
                    binding: SHADOW_SAMPLER_BINDING,
                    resource: wgpu::BindingResource::Sampler(shadow_maps.comparison_sampler()),
                },
                wgpu::BindGroupEntry {
                    binding: VISIBLE_LIGHTS_BINDING,
                    resource: clusters.visible_lights.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: CLUSTER_LIGHT_COUNTS_BINDING,
                    resource: clusters.cluster_light_counts.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: CLUSTER_LIGHT_INDICES_BINDING,
                    resource: clusters.cluster_light_indices.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: LAMP_SHADOW_ATLAS_BINDING,
                    resource: wgpu::BindingResource::TextureView(lamp_shadow_atlas),
                },
                wgpu::BindGroupEntry {
                    binding: LAMP_SHADOW_TABLE_BINDING,
                    resource: lamp_shadow_buffer.as_entire_binding(),
                },
            ],
        });

        Self {
            layout,
            bind_group,
            buffer,
            lamp_shadow_buffer,
        }
    }

    pub fn bind(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_bind_group(FRAME_GROUP, &self.bind_group, &[]);
    }

    pub fn write(&self, queue: &wgpu::Queue, uniform: &FrameUniform) {
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(uniform));
    }

    pub fn write_lamp_shadows(&self, queue: &wgpu::Queue, table: &GpuShadowLampTable) {
        queue.write_buffer(&self.lamp_shadow_buffer, 0, bytemuck::bytes_of(table));
    }
}
