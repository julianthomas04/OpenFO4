use glam::{Mat4, UVec4, Vec4};

use super::LocalLightFrame;
use super::buffers::ClusterBuffers;
use crate::rendering::shadows::lamps::table::MAX_SHADOW_SLOTS;
use crate::rendering::{
    gpu_cull::{storage_entry, uniform_entry},
    gpu_profiler::{GpuPass, GpuProfiler, SpanPart},
    shaders,
};

const WORKGROUP_SIZE: u32 = 64;
const HIZ_BINDING: u32 = 4;

/// Mirrors `LightCullUniforms` in `shaders/light_cull.wgsl`.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LightCullUniforms {
    pub view_proj: Mat4,
    pub frustum_planes: [Vec4; 5],
    pub camera_pos: Vec4,
    pub screen_size: [f32; 2],
    pub hiz_mip_count: u32,
    /// 0 until the depth pyramid holds a rendered frame.
    pub hiz_valid: u32,
    /// Placed-light slab slots to scan, holes included.
    pub slot_count: u32,
    pub fade_start: f32,
    pub fade_range: f32,
    pub intensity: f32,
    pub shadow_lamp_count: u32,
    _padding: [u32; 3],
    /// x: placed-light slab slot, y: lamp-shadow table slot.
    pub shadow_lamps: [UVec4; MAX_SHADOW_SLOTS],
}

const _: () = assert!(std::mem::size_of::<LightCullUniforms>() == 208 + 16 * MAX_SHADOW_SLOTS);

impl LightCullUniforms {
    pub fn new(frame: &LocalLightFrame) -> Self {
        let fade = frame.settings.fade;
        let mut shadow_lamps = [UVec4::ZERO; MAX_SHADOW_SLOTS];
        for (entry, pair) in shadow_lamps.iter_mut().zip(frame.shadow_lamps) {
            *entry = pair.extend(0).extend(0);
        }
        Self {
            view_proj: frame.view_proj,
            frustum_planes: frame.frustum_planes.map(Vec4::from_array),
            camera_pos: frame.camera.eye.extend(1.0),
            screen_size: frame.screen_size,
            hiz_mip_count: frame.hiz_mip_count,
            hiz_valid: u32::from(frame.hiz_valid),
            slot_count: frame.slot_count,
            fade_start: fade.start,
            fade_range: fade.range,
            intensity: frame.settings.intensity,
            shadow_lamp_count: frame.shadow_lamps.len() as u32,
            _padding: [0; 3],
            shadow_lamps,
        }
    }
}

/// One thread per placed-light slab slot: skips holes, off-by-default and
/// ambient-only lights, applies the distance fade, frustum and `HiZ` tests and
/// appends the survivors to `visible_lights` with their lamp-shadow slot.
pub struct LightCullPipeline {
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
}

impl LightCullPipeline {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = shaders::LIGHT_CULL.compile(device);
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("light_cull_bgl"),
            entries: &[
                storage_entry(0, true),
                storage_entry(1, false),
                storage_entry(2, false),
                uniform_entry(3),
                wgpu::BindGroupLayoutEntry {
                    binding: HIZ_BINDING,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("light_cull_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("light_cull_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("light_cull_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("light_cull_uniforms"),
            size: std::mem::size_of::<LightCullUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            bind_group_layout,
            uniform_buffer,
        }
    }

    /// Rebuild whenever the placed-light slab or the depth pyramid is
    /// reallocated.
    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        placed_lights: &wgpu::Buffer,
        buffers: &ClusterBuffers,
        hiz_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("light_cull_bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: placed_lights.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffers.visible_lights.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffers.counters.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: HIZ_BINDING,
                    resource: wgpu::BindingResource::TextureView(hiz_view),
                },
            ],
        })
    }

    /// The pass (and its timestamp) is skipped entirely while the slab is empty.
    pub fn dispatch(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        bind_group: &wgpu::BindGroup,
        uniforms: &LightCullUniforms,
        profiler: &GpuProfiler,
    ) {
        if uniforms.slot_count == 0 {
            return;
        }
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(uniforms));
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("light_cull"),
            timestamp_writes: profiler.compute_writes(GpuPass::LightCull, SpanPart::Whole),
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.dispatch_workgroups(uniforms.slot_count.div_ceil(WORKGROUP_SIZE), 1, 1);
    }
}
