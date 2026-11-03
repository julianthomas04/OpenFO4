//! The large loading screen's colour pass on the GPU: grades the linear picture of its stage and
//! model with `loading_grade.wgsl` onto the menu's target, faded by the overlay.

use super::grade::{GradeLook, GradeParams};
use crate::rendering::lighting::tonemap_settings::ImageSpaceCinematic;
use crate::rendering::post::tonemap::colour_lut::ColourLut;
use crate::rendering::post::tonemap::lut_table::{LutTable, blended_texels};
use crate::rendering::shaders;

pub struct GradePass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    lut: ColourLut,
    look: GradeLook,
    target_decodes_srgb: bool,
    /// The bind group of the picture drawn last, with that picture's view.
    group: Option<(wgpu::TextureView, wgpu::BindGroup)>,
}

impl GradePass {
    /// A pass onto targets of `target_format`, grading with no LUT, a neutral cinematic stage
    /// until told otherwise.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("loading_grade_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("loading_grade_layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = shaders::LOADING_GRADE.compile(device);
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Loading Grade Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("loading_grade_params"),
            size: std::mem::size_of::<GradeParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            params,
            lut: ColourLut::new(device, queue, "loading_grade_lut"),
            look: GradeLook {
                cinematic: ImageSpaceCinematic::NEUTRAL,
            },
            target_decodes_srgb: target_format.is_srgb(),
            group: None,
        }
    }

    /// Grades with `table` from now on.
    pub fn set_lut(&self, queue: &wgpu::Queue, table: &LutTable) {
        self.lut
            .write(queue, &blended_texels(&[(Some(table), 1.0)]));
    }

    pub fn set_look(&mut self, look: GradeLook) {
        self.look = look;
    }

    /// Grades `picture`, a linear image the size of `target`, over what `target` holds at
    /// `fade`, in a pass of its own.
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        picture: &wgpu::TextureView,
        target: &wgpu::TextureView,
        fade: f32,
    ) {
        let params = GradeParams::new(&self.look, fade, self.target_decodes_srgb);
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        self.ensure_group(device, picture);
        let (_, group) = self.group.as_ref().expect("the group was ensured above");
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Loading Grade Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Makes the bind group of `picture` again only when it is another view than last time.
    fn ensure_group(&mut self, device: &wgpu::Device, picture: &wgpu::TextureView) {
        if self.group.as_ref().is_none_or(|(view, _)| view != picture) {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("loading_grade_group"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(picture),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&self.lut.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.lut.sampler),
                    },
                ],
            });
            self.group = Some((picture.clone(), group));
        }
    }
}
