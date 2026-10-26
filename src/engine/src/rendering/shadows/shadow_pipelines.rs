use glam::Mat4;

use super::shadow_cull::{CASTER_KINDS, CasterKind};
use super::shadow_maps::SHADOW_MAP_FORMAT;
use super::views::{MAX_SHADOW_VIEWS, ShadowView};
use crate::rendering::skinning::{bind_layout::SkinnedBindLayout, skin_vertex::SkinVertex};
use crate::rendering::{
    render_pipelines::world_pass_layout::WorldPassLayout, shaders, vertex::Vertex,
};

pub const MATERIAL_GROUP: u32 = 0;
const VIEW_GROUP: u32 = 1;
pub const INSTANCE_GROUP: u32 = 2;

const VIEW_UNIFORM_SIZE: u64 = std::mem::size_of::<Mat4>() as u64;

/// The view slot after those of the cascades and lamp views, which `CullSlots` keeps below
/// `MAX_SHADOW_VIEWS`, for the one extra orthographic map. It has a slot of its own because every
/// view is written through the queue, ahead of all passes of the submission.
pub const ORTHO_MAP_VIEW_SLOT: usize = MAX_SHADOW_VIEWS;
const VIEW_SLOTS: usize = ORTHO_MAP_VIEW_SLOT + 1;

/// The caster pipelines of every shadow pass. They share one layout,
/// `[material, view, instance]`, so the pass binds each group once; the view
/// group selects a view slot by dynamic offset.
pub struct ShadowPipelines {
    pipelines: [wgpu::RenderPipeline; CASTER_KINDS],
    /// Skinned casters, opaque then alpha-tested; they read the skinned group
    /// where the static casters read the instance group.
    skinned_pipelines: [wgpu::RenderPipeline; 2],
    view_buffer: wgpu::Buffer,
    view_bind_group: wgpu::BindGroup,
    view_stride: u64,
}

impl ShadowPipelines {
    pub fn new(
        device: &wgpu::Device,
        world_pass_layout: &WorldPassLayout,
        skinned_layout: &SkinnedBindLayout,
    ) -> Self {
        let view_stride = VIEW_UNIFORM_SIZE.next_multiple_of(u64::from(
            device.limits().min_uniform_buffer_offset_alignment,
        ));

        let view_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow_view_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(VIEW_UNIFORM_SIZE),
                },
                count: None,
            }],
        });

        let view_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow_view_uniforms"),
            size: view_stride * VIEW_SLOTS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let view_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow_view_bind_group"),
            layout: &view_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &view_buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(VIEW_UNIFORM_SIZE),
                }),
            }],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Shadow Pass Pipeline Layout"),
            bind_group_layouts: &[
                Some(&world_pass_layout.material_bind_group_layout),
                Some(&view_layout),
                Some(&world_pass_layout.instance_bind_group_layout),
            ],
            immediate_size: 0,
        });

        let shader = shaders::SHADOW.compile(device);
        let pipelines = CasterKind::ALL.map(|kind| {
            let (vertex_entry, fragment_entry) = match kind {
                CasterKind::Opaque => ("vs_shadow_opaque", None),
                CasterKind::AlphaTest => ("vs_shadow_alpha_test", Some("fs_shadow_alpha_test")),
                CasterKind::Terrain => ("vs_shadow_terrain", None),
            };
            let label = format!("Shadow Pipeline {kind:?}");
            Self::create_pipeline(
                device,
                &layout,
                &shader,
                &label,
                vertex_entry,
                fragment_entry,
                &[Some(Vertex::desc())],
            )
        });

        let skinned_pass_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Skinned Shadow Pass Pipeline Layout"),
            bind_group_layouts: &[
                Some(&world_pass_layout.material_bind_group_layout),
                Some(&view_layout),
                Some(skinned_layout.layout()),
            ],
            immediate_size: 0,
        });
        let skinned_shader = shaders::SKINNED_SHADOW.compile(device);
        let skinned_pipelines = [
            (
                "Shadow Pipeline Skinned Opaque",
                "vs_shadow_skinned_opaque",
                None,
            ),
            (
                "Shadow Pipeline Skinned AlphaTest",
                "vs_shadow_skinned_alpha_test",
                Some("fs_shadow_alpha_test"),
            ),
        ]
        .map(|(label, vertex_entry, fragment_entry)| {
            Self::create_pipeline(
                device,
                &skinned_pass_layout,
                &skinned_shader,
                label,
                vertex_entry,
                fragment_entry,
                &[Some(Vertex::desc()), Some(SkinVertex::desc())],
            )
        });

        Self {
            pipelines,
            skinned_pipelines,
            view_buffer,
            view_bind_group,
            view_stride,
        }
    }

    /// Depth-only, both faces: thin and single-sided geometry still casts.
    fn create_pipeline(
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
        label: &str,
        vertex_entry: &str,
        fragment_entry: Option<&str>,
        vertex_buffers: &[Option<wgpu::VertexBufferLayout>],
    ) -> wgpu::RenderPipeline {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(vertex_entry),
                buffers: vertex_buffers,
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: fragment_entry.map(|entry_point| wgpu::FragmentState {
                module: shader,
                entry_point: Some(entry_point),
                targets: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_MAP_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }

    pub fn write_view(&self, queue: &wgpu::Queue, slot: usize, view: &ShadowView) {
        queue.write_buffer(
            &self.view_buffer,
            slot as u64 * self.view_stride,
            bytemuck::bytes_of(&view.view_proj),
        );
    }

    pub fn bind_view(&self, pass: &mut wgpu::RenderPass<'_>, slot: usize) {
        let offset = (slot as u64 * self.view_stride) as u32;
        pass.set_bind_group(VIEW_GROUP, &self.view_bind_group, &[offset]);
    }

    pub fn pipeline(&self, kind: CasterKind) -> &wgpu::RenderPipeline {
        &self.pipelines[kind.index()]
    }

    pub fn skinned_pipeline(&self, alpha_test: bool) -> &wgpu::RenderPipeline {
        &self.skinned_pipelines[usize::from(alpha_test)]
    }
}
