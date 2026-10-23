use crate::rendering::{
    gpu_profiler::{GpuPass, GpuProfiler, SpanPart},
    shaders,
};

const WORKGROUP_SIZE: u32 = 8;

/// Hierarchical-Z depth pyramid. Generated from the depth buffer after each
/// frame's render pass; the object cull shader samples it the *next* frame
/// (current matrices against one-frame-old depth — standard single-phase
/// GPU occlusion).
pub struct HizPipeline {
    _hiz_texture: wgpu::Texture,
    /// Per-mip views used as reduce pass attachments.
    _hiz_views: Vec<wgpu::TextureView>,
    /// Full-pyramid view for sampling in the cull shader.
    pyramid_view: wgpu::TextureView,
    mip_count: u32,
    width: u32,
    height: u32,

    depth_copy_pipeline: wgpu::ComputePipeline,
    _depth_copy_layout: wgpu::BindGroupLayout,
    depth_copy_bind_group: wgpu::BindGroup,

    reduce_pipeline: wgpu::ComputePipeline,
    _reduce_layout: wgpu::BindGroupLayout,
    reduce_bind_groups: Vec<wgpu::BindGroup>,
}

fn mip_size(base: u32, level: u32) -> u32 {
    (base >> level).max(1)
}

fn div_ceil(a: u32, b: u32) -> u32 {
    a.div_ceil(b)
}

impl HizPipeline {
    pub fn new(
        device: &wgpu::Device,
        depth_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Self {
        let mip_count = (width.max(height) as f32).log2().floor() as u32 + 1;

        let hiz_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hiz_pyramid"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: mip_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });

        let hiz_views: Vec<wgpu::TextureView> = (0..mip_count)
            .map(|level| {
                hiz_texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some(&format!("hiz_mip_{level}")),
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();

        let pyramid_view = hiz_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("hiz_pyramid_view"),
            ..Default::default()
        });

        let depth_copy_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hiz_depth_copy_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::R32Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });

        let depth_copy_shader = shaders::HIZ_DEPTH_COPY.compile(device);

        let depth_copy_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("hiz_depth_copy_pipeline"),
                layout: Some(
                    &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: None,
                        bind_group_layouts: &[Some(&depth_copy_layout)],
                        immediate_size: 0,
                    }),
                ),
                module: &depth_copy_shader,
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });

        let depth_copy_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hiz_depth_copy_bg"),
            layout: &depth_copy_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&hiz_views[0]),
                },
            ],
        });

        let reduce_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hiz_reduce_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::R32Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });

        let reduce_shader = shaders::HIZ_REDUCE.compile(device);

        let reduce_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("hiz_reduce_pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[Some(&reduce_layout)],
                    immediate_size: 0,
                }),
            ),
            module: &reduce_shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let reduce_bind_groups: Vec<wgpu::BindGroup> = (1..mip_count)
            .map(|level| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(&format!("hiz_reduce_bg_{level}")),
                    layout: &reduce_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(
                                &hiz_views[level as usize - 1],
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(
                                &hiz_views[level as usize],
                            ),
                        },
                    ],
                })
            })
            .collect();

        Self {
            _hiz_texture: hiz_texture,
            _hiz_views: hiz_views,
            pyramid_view,
            mip_count,
            width,
            height,
            depth_copy_pipeline,
            _depth_copy_layout: depth_copy_layout,
            depth_copy_bind_group,
            reduce_pipeline,
            _reduce_layout: reduce_layout,
            reduce_bind_groups,
        }
    }

    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        depth_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        *self = Self::new(device, depth_view, width, height);
    }

    /// Full-pyramid view for binding into the cull shader.
    pub fn pyramid_view(&self) -> &wgpu::TextureView {
        &self.pyramid_view
    }

    pub fn mip_count(&self) -> u32 {
        self.mip_count
    }

    pub fn generate(&self, encoder: &mut wgpu::CommandEncoder, profiler: &GpuProfiler) {
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("hiz_depth_copy"),
                timestamp_writes: profiler
                    .compute_writes(GpuPass::HizBuild, SpanPart::of(0, self.mip_count as usize)),
            });
            pass.set_pipeline(&self.depth_copy_pipeline);
            pass.set_bind_group(0, &self.depth_copy_bind_group, &[]);
            pass.dispatch_workgroups(
                div_ceil(self.width, WORKGROUP_SIZE),
                div_ceil(self.height, WORKGROUP_SIZE),
                1,
            );
        }

        for level in 1..self.mip_count {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("hiz_reduce"),
                timestamp_writes: profiler.compute_writes(
                    GpuPass::HizBuild,
                    SpanPart::of(level as usize, self.mip_count as usize),
                ),
            });
            pass.set_pipeline(&self.reduce_pipeline);
            pass.set_bind_group(0, &self.reduce_bind_groups[level as usize - 1], &[]);

            let w = mip_size(self.width, level);
            let h = mip_size(self.height, level);
            pass.dispatch_workgroups(div_ceil(w, WORKGROUP_SIZE), div_ceil(h, WORKGROUP_SIZE), 1);
        }
    }
}
