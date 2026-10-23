//! Everything the ragdoll renderer needs from the sim's ragdoll interface,
//! behind one seam: when the sim's interface changes, only this file does.

use crate::sim::{EntityId, RagdollPoses, SimWorld};

/// A ragdoll the sim stopped simulating.
pub struct Release {
    pub entity: EntityId,
    /// The sim remembers the last pose and republishes it when the ragdoll
    /// returns, so the renderer keeps showing it. Without a remembered pose the
    /// ragdoll is back at rest and its static rows must return.
    pub pose_cached: bool,
}

pub fn releases(poses: &RagdollPoses) -> impl Iterator<Item = Release> + '_ {
    poses.released.iter().map(|&entity| Release {
        entity,
        pose_cached: poses.released_with_cached_pose.contains(&entity),
    })
}

/// Reverts every placement of the model to its legacy static collision; called
/// when the model cannot be skinned, so no moving body is left with a static mesh.
/// `model_path` is the path the renderer names the model by, which is the
/// `ObjectColliderGroup::path` the sim matches (it normalises separators and case).
pub fn disable_model(sim: &mut SimWorld, model_path: &str) {
    sim.disable_ragdoll_model(model_path);
}

/// Asks the sim to publish the entity's pose again; the sim publishes a pose once,
/// so this is for a placement the renderer learned of after that publish. An
/// entity at rest publishes nothing.
pub fn republish(sim: &mut SimWorld, entity: EntityId) {
    sim.republish_ragdoll(entity);
}

/// The sim publishes a pose for the entity instead of leaving it at rest: the
/// REFR's authored pose, or a remembered one it respawns at.
pub fn has_pose(sim: &SimWorld, entity: EntityId) -> bool {
    sim.ragdoll_has_pose(entity)
}
