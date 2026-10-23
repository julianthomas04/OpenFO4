use crate::rendering::{
    gpu_driven::{DRAW_BUCKET_COUNT, IndirectDrawBuffers, TerrainIndirectBuffer},
    gpu_profiler::{GpuPass, GpuProfiler, SpanPart},
    shaders,
};

fn div_ceil(a: u32, b: u32) -> u32 {
    a.div_ceil(b)
}

// ---------- Cull Uniforms (shared layout for object + terrain) ----------
// The terrain cull shader declares only the prefix up to the pads; the
// show_markers, show_fx and HiZ fields are object-cull-only and harmlessly ignored
// there.

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
#[allow(
    clippy::pub_underscore_fields,
    reason = "explicit padding must stay a public field so the Pod struct matches the WGSL layout"
)]
pub struct CullUniforms {
    pub frustum_planes: [[f32; 4]; 5],
    pub total_instances: u32,
    /// 1 = render marker-flagged instances; 0 = cull them (debug toggle).
    pub show_markers: u32,
    /// 1 = route decal-flagged geometry into the depth-biased decal buckets;
    /// 0 = treat decals as ordinary geometry (debug A/B for the z-fighting fix).
    pub decal_bias_enabled: u32,
    /// 1 = render hidden-FX-flagged instances; 0 = cull them (debug toggle).
    pub show_fx: u32,
    pub view_proj: [[f32; 4]; 4],
    pub screen_size: [f32; 2],
    pub hiz_mip_count: u32,
    /// 0 until a frame has rendered into the depth pyramid (first frame,
    /// resize): the occlusion test is skipped while 0.
    pub hiz_valid: u32,
}

// ---------- Object Cull Pipeline ----------

pub struct ObjectCullPipeline {
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
}

pub fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

pub fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

impl ObjectCullPipeline {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = shaders::OBJECT_CULL.compile(device);

        // 0: instances (read), 1..=DRAW_BUCKET_COUNT: per-bucket draws (rw),
        // DRAW_BUCKET_COUNT+1: counts (rw), +2: uniforms, +3: HiZ pyramid (draws
        // 11/12 are the multiply buckets, counts 13, uniforms 14, HiZ 15). The
        // instances + per-bucket draws + counts are storage buffers (device.rs
        // requests DRAW_BUCKET_COUNT + 2 and verifies adapter support); the pyramid
        // is a texture binding and doesn't count.
        let mut entries = vec![storage_entry(0, true)];
        for binding in 1..=(DRAW_BUCKET_COUNT as u32) {
            entries.push(storage_entry(binding, false));
        }
        entries.push(storage_entry(DRAW_BUCKET_COUNT as u32 + 1, false));
        entries.push(uniform_entry(DRAW_BUCKET_COUNT as u32 + 2));
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: DRAW_BUCKET_COUNT as u32 + 3,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("object_cull_bgl"),
            entries: &entries,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("object_cull_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("object_cull_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cull_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cull_uniforms"),
            size: std::mem::size_of::<CullUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            bind_group_layout,
            uniform_buffer,
        }
    }

    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        instance_buffer: &wgpu::Buffer,
        indirect: &IndirectDrawBuffers,
        hiz_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: instance_buffer.as_entire_binding(),
        }];
        for (i, buffer) in indirect.draw_buffers.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: i as u32 + 1,
                resource: buffer.as_entire_binding(),
            });
        }
        entries.push(wgpu::BindGroupEntry {
            binding: DRAW_BUCKET_COUNT as u32 + 1,
            resource: indirect.count_buffer.as_entire_binding(),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: DRAW_BUCKET_COUNT as u32 + 2,
            resource: self.uniform_buffer.as_entire_binding(),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: DRAW_BUCKET_COUNT as u32 + 3,
            resource: wgpu::BindingResource::TextureView(hiz_view),
        });

        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("object_cull_bg"),
            layout: &self.bind_group_layout,
            entries: &entries,
        })
    }

    pub fn dispatch(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        bind_group: &wgpu::BindGroup,
        uniforms: &CullUniforms,
        queue: &wgpu::Queue,
        profiler: &GpuProfiler,
    ) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(uniforms));

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("object_cull"),
            timestamp_writes: profiler.compute_writes(GpuPass::ObjectCull, SpanPart::Whole),
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.dispatch_workgroups(div_ceil(uniforms.total_instances, 256), 1, 1);
    }
}

// ---------- Terrain Cull Pipeline ----------

pub struct TerrainCullPipeline {
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
}

impl TerrainCullPipeline {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = shaders::TERRAIN_CULL.compile(device);

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("terrain_cull_bgl"),
            entries: &[
                // 0: terrain instances (read)
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // 1: draw_commands (read_write)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // 2: draw_count (read_write, atomic)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // 3: cull uniforms
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain_cull_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("terrain_cull_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("terrain_cull_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain_cull_uniforms"),
            size: std::mem::size_of::<CullUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            bind_group_layout,
            uniform_buffer,
        }
    }

    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        instance_buffer: &wgpu::Buffer,
        indirect: &TerrainIndirectBuffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain_cull_bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: instance_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: indirect.draw_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: indirect.count_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
            ],
        })
    }

    pub fn dispatch(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        bind_group: &wgpu::BindGroup,
        uniforms: &CullUniforms,
        queue: &wgpu::Queue,
        profiler: &GpuProfiler,
    ) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(uniforms));

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("terrain_cull"),
            timestamp_writes: profiler.compute_writes(GpuPass::TerrainCull, SpanPart::Whole),
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.dispatch_workgroups(div_ceil(uniforms.total_instances, 64), 1, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::{
        gpu_driven::{
            BUCKET_BLEND_CULLED, BUCKET_MULTIPLY_CULLED, BUCKET_MULTIPLY_TWOSIDED, GpuInstance,
            INSTANCE_FLAGS_BLEND, INSTANCE_FLAGS_DECAL, INSTANCE_FLAGS_EFFECT,
            INSTANCE_FLAGS_HIDDEN_FX, INSTANCE_FLAGS_MULTIPLY, INSTANCE_FLAGS_TWO_SIDED,
        },
        gpu_test::test_device,
    };
    use bytemuck::Zeroable;

    /// Draw count per bucket after culling one instance with these flags.
    fn cull_one_instance(flags: u32, decal_bias_enabled: bool) -> [u32; DRAW_BUCKET_COUNT] {
        cull_one_instance_with_fx(flags, decal_bias_enabled, false)
    }

    fn cull_one_instance_with_fx(
        flags: u32,
        decal_bias_enabled: bool,
        show_fx: bool,
    ) -> [u32; DRAW_BUCKET_COUNT] {
        let (device, queue) = test_device("object cull routing").expect("a GPU adapter");
        let cull = ObjectCullPipeline::new(&device);
        let indirect = IndirectDrawBuffers::new(&device, "cull_test");
        let profiler = GpuProfiler::new(&device, &queue);

        let instance = GpuInstance {
            index_count: 3,
            flags,
            ..GpuInstance::zeroed()
        };
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cull_test_instances"),
            size: std::mem::size_of::<GpuInstance>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&instance_buffer, 0, bytemuck::bytes_of(&instance));

        let hiz = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cull_test_hiz"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let bind_group = cull.create_bind_group(
            &device,
            &instance_buffer,
            &indirect,
            &hiz.create_view(&wgpu::TextureViewDescriptor::default()),
        );

        let accept_everything = [[0.0, 0.0, 0.0, 1.0]; 5];
        let uniforms = CullUniforms {
            frustum_planes: accept_everything,
            total_instances: 1,
            show_markers: 0,
            decal_bias_enabled: u32::from(decal_bias_enabled),
            show_fx: u32::from(show_fx),
            view_proj: [[0.0; 4]; 4],
            screen_size: [1.0, 1.0],
            hiz_mip_count: 1,
            hiz_valid: 0,
        };

        let counts_size = (DRAW_BUCKET_COUNT * std::mem::size_of::<u32>()) as u64;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cull_test_counts_staging"),
            size: counts_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        indirect.clear_counts(&mut encoder);
        cull.dispatch(&mut encoder, &bind_group, &uniforms, &queue, &profiler);
        encoder.copy_buffer_to_buffer(&indirect.count_buffer, 0, &staging, 0, counts_size);
        queue.submit(std::iter::once(encoder.finish()));

        let (sender, outcome) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                sender
                    .send(result)
                    .expect("readback outlives its map callback");
            });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("cull readback poll failed");
        outcome
            .recv()
            .expect("wait_indefinitely runs the map callback")
            .expect("cull readback map failed");
        let data = staging
            .slice(..)
            .get_mapped_range()
            .expect("mapped staging buffer exposes its full range");
        let counts: Vec<u32> = bytemuck::cast_slice(&data).to_vec();
        counts.try_into().expect("one count per bucket")
    }

    fn only_bucket(counts: [u32; DRAW_BUCKET_COUNT]) -> Option<usize> {
        let mut drawn = counts.iter().enumerate().filter(|(_, count)| **count > 0);
        let first = drawn.next().map(|(bucket, _)| bucket);
        assert!(
            drawn.next().is_none(),
            "instance drawn into several buckets: {counts:?}"
        );
        first
    }

    #[test]
    #[ignore = "needs a GPU adapter"]
    fn cull_routes_multiply_instances_to_the_multiply_buckets() {
        let multiply = INSTANCE_FLAGS_BLEND | INSTANCE_FLAGS_MULTIPLY;
        for decal_bias in [false, true] {
            assert_eq!(
                only_bucket(cull_one_instance(
                    multiply | INSTANCE_FLAGS_TWO_SIDED,
                    decal_bias
                )),
                Some(BUCKET_MULTIPLY_TWOSIDED)
            );
            assert_eq!(
                only_bucket(cull_one_instance(multiply, decal_bias)),
                Some(BUCKET_MULTIPLY_CULLED)
            );
            assert_eq!(
                only_bucket(cull_one_instance(
                    multiply | INSTANCE_FLAGS_DECAL,
                    decal_bias
                )),
                Some(BUCKET_MULTIPLY_CULLED)
            );
        }
    }

    #[test]
    #[ignore = "needs a GPU adapter"]
    fn cull_drops_hidden_fx_while_the_toggle_is_off() {
        assert_eq!(
            only_bucket(cull_one_instance_with_fx(
                INSTANCE_FLAGS_HIDDEN_FX,
                true,
                false
            )),
            None
        );
    }

    #[test]
    #[ignore = "needs a GPU adapter"]
    fn cull_draws_hidden_fx_once_the_toggle_is_on() {
        assert_eq!(
            only_bucket(cull_one_instance_with_fx(
                INSTANCE_FLAGS_HIDDEN_FX | INSTANCE_FLAGS_BLEND,
                true,
                true
            )),
            Some(BUCKET_BLEND_CULLED)
        );
    }

    #[test]
    #[ignore = "needs a GPU adapter"]
    fn cull_always_draws_plain_effect_instances() {
        assert_eq!(
            only_bucket(cull_one_instance_with_fx(
                INSTANCE_FLAGS_EFFECT | INSTANCE_FLAGS_BLEND,
                true,
                false
            )),
            Some(BUCKET_BLEND_CULLED)
        );
    }

    #[test]
    #[ignore = "needs a GPU adapter"]
    fn cull_still_routes_plain_blend_to_the_blend_bucket() {
        assert_eq!(
            only_bucket(cull_one_instance(INSTANCE_FLAGS_BLEND, true)),
            Some(BUCKET_BLEND_CULLED)
        );
    }
}
