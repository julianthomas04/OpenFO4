use glam::Vec4;

use super::bind_layout::SKINNED_GROUP;
use super::character::CharacterInstance;
use super::gpu_mesh::GpuSkinnedPart;
use crate::rendering::frustum::Aabb;

/// One draw of one mesh part of one character; `instance_index` addresses its
/// `GpuInstance` and palette base in the frame buffers.
#[derive(Clone, Copy)]
pub struct PartDraw {
    pub character: usize,
    pub mesh: usize,
    pub part: usize,
    pub instance_index: u32,
}

/// Which characters a draw considers: those the camera sees, or those inside
/// another view's frustum (a shadow cascade or lamp face).
#[derive(Clone, Copy)]
pub enum Visibility<'a> {
    Camera,
    Frustum(&'a [Vec4; 6]),
}

/// Read-only view of this frame's characters for the passes that draw them.
pub struct SkinnedScene<'a> {
    pub bind_group: &'a wgpu::BindGroup,
    pub characters: &'a [CharacterInstance],
    pub draws: &'a [PartDraw],
    pub visible: &'a [bool],
    pub world_bounds: &'a [Aabb],
}

impl SkinnedScene<'_> {
    pub fn is_empty(&self) -> bool {
        self.draws.is_empty()
    }

    fn admits(&self, character: usize, visibility: Visibility) -> bool {
        match visibility {
            Visibility::Camera => self.visible[character],
            Visibility::Frustum(planes) => !self.world_bounds[character].is_outside(planes),
        }
    }

    /// Draws every part of an admitted character that `select` yields a
    /// pipeline for. Binds the skinned group on first use and returns whether
    /// it did, so the caller can restore the group it displaced. The pipeline
    /// is only set when it changes between consecutive parts.
    pub fn draw<'p>(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        visibility: Visibility,
        select: impl Fn(&GpuSkinnedPart) -> Option<&'p wgpu::RenderPipeline>,
    ) -> bool {
        let mut bound = false;
        let mut current: Option<&wgpu::RenderPipeline> = None;
        for draw in self.draws {
            if !self.admits(draw.character, visibility) {
                continue;
            }
            let part = &self.characters[draw.character].rig().meshes[draw.mesh].parts[draw.part];
            let Some(pipeline) = select(part) else {
                continue;
            };
            if !bound {
                pass.set_bind_group(SKINNED_GROUP, self.bind_group, &[]);
                bound = true;
            }
            if current.is_none_or(|set| !std::ptr::eq(set, pipeline)) {
                pass.set_pipeline(pipeline);
                current = Some(pipeline);
            }
            pass.set_vertex_buffer(0, part.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, part.skin_buffer.slice(..));
            pass.set_index_buffer(part.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(
                0..part.index_count,
                0,
                draw.instance_index..draw.instance_index + 1,
            );
        }
        bound
    }
}
