use wgpu::util::DeviceExt;

use crate::rendering::{
    device_loss::DeviceLossMonitor,
    gpu_profiler::{GpuPass, GpuProfiler, SpanPart},
    lighting::state::ExposureReadout,
    readback_ring::{ReadbackError, ReadbackRing},
    shaders,
};

const STATE_BYTES: u64 = std::mem::size_of::<ExposureReadout>() as u64;

/// Layout mirrored by `Params` in `shaders/auto_exposure.wgsl`.
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct AutoExposureParams {
    pub adaptation: f32,
    pub middle_gray: f32,
    pub min: f32,
    pub max: f32,
    pub ev_scale: f32,
    pub auto: u32,
}

/// Measures the log-average luminance of the HDR target on the GPU and adapts
/// the exposure the tonemap reads towards it. `ExposureReadout` doubles as the
/// layout of the state buffer, which starts zeroed: the first frame snaps.
pub struct AutoExposure {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    params: wgpu::Buffer,
    pub state: wgpu::Buffer,
    ring: ReadbackRing,
}

impl AutoExposure {
    pub fn new(device: &wgpu::Device, hdr_view: &wgpu::TextureView) -> Self {
        let shader = shaders::AUTO_EXPOSURE.compile(device);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("auto_exposure_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<
                            AutoExposureParams,
                        >() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(STATE_BYTES),
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("auto_exposure_pipeline_layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("auto_exposure_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("auto_exposure_params"),
            contents: bytemuck::bytes_of(&<AutoExposureParams as bytemuck::Zeroable>::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let state = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("auto_exposure_state"),
            size: STATE_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let bind_group = Self::create_bind_group(device, &layout, hdr_view, &params, &state);

        Self {
            pipeline,
            layout,
            bind_group,
            params,
            state,
            ring: ReadbackRing::new(device, "auto_exposure", STATE_BYTES),
        }
    }

    pub fn rebind_source(&mut self, device: &wgpu::Device, hdr_view: &wgpu::TextureView) {
        self.bind_group =
            Self::create_bind_group(device, &self.layout, hdr_view, &self.params, &self.state);
    }

    pub fn write_params(&self, queue: &wgpu::Queue, params: &AutoExposureParams) {
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(params));
    }

    pub fn encode(&mut self, encoder: &mut wgpu::CommandEncoder, profiler: &GpuProfiler) {
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("auto_exposure"),
                timestamp_writes: profiler.compute_writes(GpuPass::AutoExposure, SpanPart::Whole),
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        if let Some(slot) = self.ring.acquire() {
            encoder.copy_buffer_to_buffer(&self.state, 0, self.ring.buffer(slot), 0, STATE_BYTES);
        }
    }

    pub fn post_submit(&mut self) {
        self.ring.post_submit();
    }

    /// Non-blocking: the newest completed readback, if any arrived.
    pub fn poll(
        &mut self,
        device: &wgpu::Device,
        device_loss: &DeviceLossMonitor,
    ) -> Result<Option<ExposureReadout>, ReadbackError> {
        let mut latest = None;
        self.ring.poll(device, device_loss, |_slot, data| {
            latest = Some(*bytemuck::from_bytes::<ExposureReadout>(data));
        })?;
        Ok(latest)
    }

    fn create_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        hdr_view: &wgpu::TextureView,
        params: &wgpu::Buffer,
        state: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("auto_exposure_bind_group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(hdr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: state.as_entire_binding(),
                },
            ],
        })
    }
}
