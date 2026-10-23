//! How the per-frame pose writers describe where an entity's instance rows go,
//! independent of whether a rigid body or an animation drives them.

use glam::{Mat4, Quat, Vec3};

use crate::sim::{DynamicPose, EntityId};

pub(crate) enum RowPose<'a> {
    /// One rigid pose for every row; each row composes its own placement scale.
    Rigid { translation: Vec3, rotation: Quat },
    /// Each row follows its joint: `refr_world * model_pose[joint]`.
    Skeletal {
        refr_world: Mat4,
        model_pose: &'a [Mat4],
    },
    /// Scales every row to zero at `at`: its triangles degenerate and draw
    /// nothing, and its cull bounds collapse to the point `at` (the REFR origin).
    /// The GPU cull still tests that point, so a hidden row costs a draw of
    /// degenerate triangles while `at` is in view.
    Hidden { at: Vec3 },
}

pub(crate) trait PoseSource {
    fn len(&self) -> usize;
    fn entity(&self, index: usize) -> EntityId;
    /// `None` when the source cannot produce the pose it announced.
    fn pose(&self, index: usize) -> Option<RowPose<'_>>;
}

impl PoseSource for [DynamicPose] {
    fn len(&self) -> usize {
        <[DynamicPose]>::len(self)
    }

    fn entity(&self, index: usize) -> EntityId {
        self[index].entity
    }

    fn pose(&self, index: usize) -> Option<RowPose<'_>> {
        let pose = self.get(index)?;
        Some(RowPose::Rigid {
            translation: pose.translation,
            rotation: pose.rotation,
        })
    }
}
