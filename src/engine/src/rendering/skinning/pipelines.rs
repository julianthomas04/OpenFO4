use super::bind_layout::SkinnedBindLayout;
use super::materials::SurfaceKind;
use super::skin_vertex::SkinVertex;
use crate::rendering::{
    render_pipelines::{
        object::{ObjectPipelineSpec, SurfaceVariant, create_object_pipeline},
        world_pass_layout::WorldPassLayout,
    },
    shaders,
    vertex::Vertex,
};

/// World-pass pipelines of skinned characters: the object fragment shaders
/// behind a skinning vertex stage. The layout swaps the instance group for the
/// skinned group and keeps material, frame and feedback groups.
pub struct SkinnedPipelines {
    pipelines: [wgpu::RenderPipeline; 8],
}

impl SkinnedPipelines {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        world_pass_layout: &WorldPassLayout,
        frame_bind_group_layout: &wgpu::BindGroupLayout,
        skinned_layout: &SkinnedBindLayout,
        texture_feedback: bool,
    ) -> Self {
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Skinned Pass Pipeline Layout"),
            bind_group_layouts: &[
                Some(&world_pass_layout.material_bind_group_layout),
                Some(frame_bind_group_layout),
                Some(skinned_layout.layout()),
                Some(&world_pass_layout.feedback_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let shader = shaders::SKINNED_OBJECT.compile(device);
        let spec = ObjectPipelineSpec {
            shader: &shader,
            layout: &layout,
            vertex_entry: "vs_skinned",
            vertex_buffers: &[Some(Vertex::desc()), Some(SkinVertex::desc())],
            format,
            texture_feedback,
        };
        let create = |variant| create_object_pipeline(device, &spec, variant);
        Self {
            pipelines: [
                create(SurfaceVariant::opaque(false)),
                create(SurfaceVariant::opaque(true)),
                create(SurfaceVariant::alphatest(false)),
                create(SurfaceVariant::alphatest(true)),
                create(SurfaceVariant::blend(false)),
                create(SurfaceVariant::blend(true)),
                create(SurfaceVariant::multiply(false)),
                create(SurfaceVariant::multiply(true)),
            ],
        }
    }

    pub fn pipeline(&self, kind: SurfaceKind, two_sided: bool) -> &wgpu::RenderPipeline {
        let kind_index = match kind {
            SurfaceKind::Opaque => 0,
            SurfaceKind::AlphaTest => 1,
            SurfaceKind::Blend => 2,
            SurfaceKind::Multiply => 3,
        };
        &self.pipelines[kind_index * 2 + usize::from(two_sided)]
    }
}
