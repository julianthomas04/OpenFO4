use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use glam::Mat4;

use super::binding::{BindError, ClipBinding};
use super::clip_roles::ClipRoles;
use super::gltf_import::AnimationAsset;
use super::pose::{PoseError, local_to_model};
use super::skeleton::{JointIndex, Trs};

#[derive(Debug, thiserror::Error)]
pub enum RigError {
    #[error(transparent)]
    Bind(#[from] BindError),
    #[error(transparent)]
    Pose(#[from] PoseError),
}

/// An animation asset prepared for playback: every clip bound to the skeleton,
/// the rest pose in model space, and which joints any clip can move.
#[derive(Debug)]
pub struct AnimationRig {
    pub asset: AnimationAsset,
    bindings: Vec<ClipBinding>,
    rest_model: Vec<Mat4>,
    rest_model_inverse: Vec<Mat4>,
    moving: Vec<bool>,
    joint_reach: Vec<f32>,
    roles: ClipRoles,
}

impl AnimationRig {
    pub fn new(asset: AnimationAsset) -> Result<Self, RigError> {
        let bindings = asset
            .clips
            .iter()
            .map(|clip| {
                let binding = ClipBinding::resolve(clip, &asset.skeleton)?;
                binding.warn_unbound("rig", clip.name());
                Ok(binding)
            })
            .collect::<Result<Vec<_>, BindError>>()?;

        let mut rest_model = vec![Mat4::IDENTITY; asset.skeleton.len()];
        local_to_model(&asset.skeleton, asset.skeleton.rest_pose(), &mut rest_model)?;

        let mut moving = vec![false; asset.skeleton.len()];
        for joint in asset.anim_joints.iter().copied().chain(
            bindings
                .iter()
                .flat_map(super::binding::ClipBinding::bound_joints),
        ) {
            moving[joint as usize] = true;
        }
        for (joint, parent) in asset.skeleton.parents().iter().enumerate() {
            if parent.is_some_and(|p| moving[p as usize]) {
                moving[joint] = true;
            }
        }
        let rest_model_inverse = rest_model.iter().map(Mat4::inverse).collect();
        let joint_reach = vec![0.0; asset.skeleton.len()];
        let roles = ClipRoles::resolve(&asset.clips);
        Ok(Self {
            asset,
            bindings,
            rest_model,
            rest_model_inverse,
            moving,
            joint_reach,
            roles,
        })
    }

    /// Per joint, the largest distance from its origin to the geometry attached to
    /// it, in its own space; 0 for a joint with no geometry.
    pub fn joint_reach(&self) -> &[f32] {
        &self.joint_reach
    }

    pub fn set_joint_reach(&mut self, joint_reach: Vec<f32>) {
        debug_assert_eq!(joint_reach.len(), self.asset.skeleton.len());
        self.joint_reach = joint_reach;
    }

    pub fn roles(&self) -> &ClipRoles {
        &self.roles
    }

    pub fn clip_index(&self, name: &str) -> Option<usize> {
        self.asset
            .clips
            .iter()
            .position(|clip| clip.name().eq_ignore_ascii_case(name))
    }

    pub fn binding(&self, clip: usize) -> &ClipBinding {
        &self.bindings[clip]
    }

    pub fn joint_by_name(&self, name: &str) -> Option<JointIndex> {
        self.asset.skeleton.index_of(name)
    }

    pub fn rest_local(&self) -> &[Trs] {
        self.asset.skeleton.rest_pose()
    }

    pub fn rest_model(&self) -> &[Mat4] {
        &self.rest_model
    }

    pub fn rest_model_inverse(&self) -> &[Mat4] {
        &self.rest_model_inverse
    }

    /// Identifies which joints (by index and name) any clip moves; equal for
    /// rigs that bind shapes to the same joints, `None` when nothing moves.
    pub fn moving_joint_fingerprint(&self) -> Option<u64> {
        let mut hasher = DefaultHasher::new();
        let mut any = false;
        for (joint, name) in self.asset.skeleton.names().iter().enumerate() {
            if self.moving[joint] {
                (joint, name).hash(&mut hasher);
                any = true;
            }
        }
        any.then(|| hasher.finish())
    }

    /// True when a clip can move the joint, directly or through an ancestor.
    pub fn is_moving(&self, joint: JointIndex) -> bool {
        self.moving[joint as usize]
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::anim::clip::{Clip, Cycle, Interpolation, Keys, NodeTrack};
    use crate::anim::skeleton::Skeleton;

    /// root -> arm -> hand, plus an unrelated root `other`.
    fn asset(animated_nodes: &[&str], anim_joints: Vec<JointIndex>) -> AnimationAsset {
        let names = ["root", "arm", "hand", "other"];
        let skeleton = Skeleton::new(
            names.iter().map(|s| (*s).to_owned()).collect(),
            vec![None, Some(0), Some(1), None],
            vec![
                Trs {
                    translation: Vec3::X,
                    ..Trs::IDENTITY
                };
                4
            ],
        )
        .unwrap();
        let tracks = animated_nodes
            .iter()
            .map(|node| NodeTrack {
                node_name: (*node).to_owned(),
                translation: Some(
                    Keys::new(vec![0.0], vec![Vec3::ZERO], Interpolation::Linear).unwrap(),
                ),
                rotation: None,
                scale: None,
            })
            .collect::<Vec<_>>();
        let clips = if tracks.is_empty() {
            vec![]
        } else {
            vec![Clip::new("Open".to_owned(), 1.0, Cycle::Clamp, tracks, vec![]).unwrap()]
        };
        AnimationAsset {
            skeleton,
            anim_joints,
            clips,
            skins: vec![],
            warnings: vec![],
        }
    }

    #[test]
    fn animating_a_joint_marks_it_and_all_descendants_as_moving() {
        let rig = AnimationRig::new(asset(&["arm"], vec![])).unwrap();
        let moving: Vec<bool> = (0..4).map(|j| rig.is_moving(j)).collect();
        assert_eq!(moving, [false, true, true, false]);
    }

    #[test]
    fn anim_flagged_joints_move_even_without_a_track() {
        let rig = AnimationRig::new(asset(&[], vec![3])).unwrap();
        assert!(rig.is_moving(3));
        assert!(!rig.is_moving(0));
    }

    #[test]
    fn rest_model_composes_the_hierarchy_and_inverse_undoes_it() {
        let rig = AnimationRig::new(asset(&[], vec![])).unwrap();
        // Three chained unit translations along X put the hand 3 units out.
        assert_eq!(
            rig.rest_model()[2].w_axis.truncate(),
            Vec3::new(3.0, 0.0, 0.0)
        );
        let roundtrip = rig.rest_model()[2] * rig.rest_model_inverse()[2];
        assert!(roundtrip.abs_diff_eq(Mat4::IDENTITY, 1e-6));
    }

    #[test]
    fn fingerprint_is_none_when_nothing_moves_and_equal_for_equal_moving_sets() {
        let still = AnimationRig::new(asset(&[], vec![])).unwrap();
        assert_eq!(still.moving_joint_fingerprint(), None);
        let a = AnimationRig::new(asset(&["arm"], vec![])).unwrap();
        let b = AnimationRig::new(asset(&["hand"], vec![1])).unwrap();
        let c = AnimationRig::new(asset(&["root"], vec![])).unwrap();
        assert_eq!(a.moving_joint_fingerprint(), b.moving_joint_fingerprint());
        assert_ne!(a.moving_joint_fingerprint(), c.moving_joint_fingerprint());
    }

    #[test]
    fn a_clip_binding_nothing_fails_rig_construction() {
        let err = AnimationRig::new(asset(&["ghost"], vec![])).unwrap_err();
        assert!(matches!(err, RigError::Bind(_)));
    }

    #[test]
    fn clip_lookup_ignores_case() {
        let rig = AnimationRig::new(asset(&["arm"], vec![])).unwrap();
        assert_eq!(rig.clip_index("OPEN"), Some(0));
        assert_eq!(rig.clip_index("close"), None);
        assert_eq!(rig.roles().open, Some(0));
    }
}
