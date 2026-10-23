use std::collections::HashMap;

use glam::{Mat4, Quat, Vec3};

pub type JointIndex = u16;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trs {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Trs {
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    pub fn to_matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

impl Default for Trs {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SkeletonError {
    #[error(
        "skeleton arrays disagree in length: {names} names, {parents} parents, {rest} rest transforms"
    )]
    LengthMismatch {
        names: usize,
        parents: usize,
        rest: usize,
    },
    #[error("skeleton has {0} joints, more than a joint index can address")]
    TooManyJoints(usize),
    #[error("duplicate joint name {0:?}")]
    DuplicateName(String),
    #[error("joint {joint} ({name:?}) names parent {parent}, which does not come before it")]
    ParentNotBeforeChild {
        joint: usize,
        name: String,
        parent: JointIndex,
    },
}

#[derive(Clone, Debug)]
pub struct Skeleton {
    names: Vec<String>,
    index_by_name: HashMap<String, JointIndex>,
    /// Fallback for sources that disagree on case (the Gamebryo NIF skeleton
    /// says "HEAD" where the Havok one says "Head"); the first joint wins.
    index_by_lowercase_name: HashMap<String, JointIndex>,
    parents: Vec<Option<JointIndex>>,
    rest: Vec<Trs>,
}

impl Skeleton {
    pub fn new(
        names: Vec<String>,
        parents: Vec<Option<JointIndex>>,
        rest: Vec<Trs>,
    ) -> Result<Self, SkeletonError> {
        if names.len() != parents.len() || names.len() != rest.len() {
            return Err(SkeletonError::LengthMismatch {
                names: names.len(),
                parents: parents.len(),
                rest: rest.len(),
            });
        }
        if names.len() > JointIndex::MAX as usize + 1 {
            return Err(SkeletonError::TooManyJoints(names.len()));
        }

        let mut index_by_name = HashMap::with_capacity(names.len());
        let mut index_by_lowercase_name = HashMap::with_capacity(names.len());
        for (joint, name) in names.iter().enumerate() {
            if index_by_name
                .insert(name.clone(), joint as JointIndex)
                .is_some()
            {
                return Err(SkeletonError::DuplicateName(name.clone()));
            }
            index_by_lowercase_name
                .entry(name.to_lowercase())
                .or_insert(joint as JointIndex);
        }
        for (joint, parent) in parents.iter().enumerate() {
            if let Some(parent) = *parent
                && parent as usize >= joint
            {
                return Err(SkeletonError::ParentNotBeforeChild {
                    joint,
                    name: names[joint].clone(),
                    parent,
                });
            }
        }

        Ok(Self {
            names,
            index_by_name,
            index_by_lowercase_name,
            parents,
            rest,
        })
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn parents(&self) -> &[Option<JointIndex>] {
        &self.parents
    }

    pub fn rest_pose(&self) -> &[Trs] {
        &self.rest
    }

    /// An exact name match wins; otherwise the name is matched ignoring case.
    pub fn index_of(&self, name: &str) -> Option<JointIndex> {
        self.index_by_name
            .get(name)
            .or_else(|| self.index_by_lowercase_name.get(&name.to_lowercase()))
            .copied()
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn skeleton(
        list: &[&str],
        parents: Vec<Option<JointIndex>>,
    ) -> Result<Skeleton, SkeletonError> {
        let rest = vec![Trs::IDENTITY; list.len()];
        Skeleton::new(names(list), parents, rest)
    }

    #[test]
    fn new_rejects_arrays_of_different_length() {
        let err =
            Skeleton::new(names(&["a", "b"]), vec![None], vec![Trs::IDENTITY; 2]).unwrap_err();
        assert!(matches!(
            err,
            SkeletonError::LengthMismatch {
                names: 2,
                parents: 1,
                rest: 2
            }
        ));
    }

    #[test]
    fn new_rejects_duplicate_names() {
        let err = skeleton(&["a", "a"], vec![None, Some(0)]).unwrap_err();
        assert!(matches!(err, SkeletonError::DuplicateName(n) if n == "a"));
    }

    #[test]
    fn new_rejects_a_parent_that_comes_after_its_child() {
        let err = skeleton(&["child", "parent"], vec![Some(1), None]).unwrap_err();
        assert!(matches!(
            err,
            SkeletonError::ParentNotBeforeChild {
                joint: 0,
                parent: 1,
                ..
            }
        ));
    }

    #[test]
    fn new_rejects_a_joint_that_is_its_own_parent() {
        let err = skeleton(&["a"], vec![Some(0)]).unwrap_err();
        assert!(matches!(
            err,
            SkeletonError::ParentNotBeforeChild { joint: 0, .. }
        ));
    }

    #[test]
    fn new_rejects_more_joints_than_an_index_can_address() {
        let count = JointIndex::MAX as usize + 2;
        let names: Vec<String> = (0..count).map(|i| i.to_string()).collect();
        let err = Skeleton::new(names, vec![None; count], vec![Trs::IDENTITY; count]).unwrap_err();
        assert!(matches!(err, SkeletonError::TooManyJoints(n) if n == count));
    }

    #[test]
    fn index_of_finds_exact_names() {
        let s = skeleton(&["root", "Head"], vec![None, Some(0)]).unwrap();
        assert_eq!(s.index_of("Head"), Some(1));
        assert_eq!(s.index_of("missing"), None);
    }

    #[test]
    fn index_of_falls_back_to_case_insensitive_match() {
        let s = skeleton(&["root", "Head"], vec![None, Some(0)]).unwrap();
        assert_eq!(s.index_of("HEAD"), Some(1));
    }

    #[test]
    fn index_of_prefers_exact_match_over_case_insensitive_one() {
        let s = skeleton(&["HEAD", "Head"], vec![None, None]).unwrap();
        assert_eq!(s.index_of("Head"), Some(1));
        assert_eq!(s.index_of("head"), Some(0), "first joint wins the fallback");
    }
}
