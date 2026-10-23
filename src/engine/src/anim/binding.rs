use super::clip::Clip;
use super::skeleton::{JointIndex, Skeleton};

const REPORTED_NAMES: usize = 5;

#[derive(Debug, thiserror::Error)]
#[error("clip {clip:?} binds none of its {track_count} tracks to the skeleton")]
pub struct BindError {
    pub clip: String,
    pub track_count: usize,
}

/// Maps a clip's tracks onto skeleton joints. Tracks for nodes the skeleton
/// lacks (e.g. weapon and camera bones that live on attached skeletons) stay
/// unbound and are skipped when sampling.
#[derive(Clone, Debug)]
pub struct ClipBinding {
    joints: Vec<Option<JointIndex>>,
    unbound: Vec<String>,
    joint_count: usize,
}

impl ClipBinding {
    pub fn resolve(clip: &Clip, skeleton: &Skeleton) -> Result<Self, BindError> {
        let mut joints = Vec::with_capacity(clip.tracks().len());
        let mut unbound = Vec::new();
        for track in clip.tracks() {
            let joint = skeleton.index_of(&track.node_name);
            if joint.is_none() {
                unbound.push(track.node_name.clone());
            }
            joints.push(joint);
        }
        if !joints.is_empty() && unbound.len() == joints.len() {
            return Err(BindError {
                clip: clip.name().to_owned(),
                track_count: joints.len(),
            });
        }
        Ok(Self {
            joints,
            unbound,
            joint_count: skeleton.len(),
        })
    }

    /// Joint index for each of the clip's tracks, in track order; `None` for unbound tracks.
    pub fn joints(&self) -> &[Option<JointIndex>] {
        &self.joints
    }

    pub fn bound_joints(&self) -> impl Iterator<Item = JointIndex> + '_ {
        self.joints.iter().flatten().copied()
    }

    /// Names of the animated nodes the skeleton does not contain.
    pub fn unbound(&self) -> &[String] {
        &self.unbound
    }

    pub fn joint_count(&self) -> usize {
        self.joint_count
    }

    /// Logs one warning with the count and the first few unbound nodes; silent when everything bound.
    pub fn warn_unbound(&self, owner: &str, clip: &str) {
        if self.unbound.is_empty() {
            return;
        }
        let shown = self
            .unbound
            .iter()
            .take(REPORTED_NAMES)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let more = if self.unbound.len() > REPORTED_NAMES {
            ", ..."
        } else {
            ""
        };
        tracing::warn!(
            "anim: {owner}: clip {clip:?} skips {} of {} tracks for nodes missing from the skeleton: {shown}{more}",
            self.unbound.len(),
            self.joints.len()
        );
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use super::*;
    use crate::anim::clip::{Cycle, NodeTrack};
    use crate::anim::skeleton::Trs;

    fn skeleton(names: &[&str]) -> Skeleton {
        let parents = (0..names.len())
            .map(|i| i.checked_sub(1).map(|p| p as JointIndex))
            .collect();
        Skeleton::new(
            names.iter().map(|s| (*s).to_owned()).collect(),
            parents,
            vec![Trs::IDENTITY; names.len()],
        )
        .unwrap()
    }

    fn clip(nodes: &[&str]) -> Clip {
        let tracks = nodes
            .iter()
            .map(|n| NodeTrack {
                node_name: (*n).to_owned(),
                translation: None,
                rotation: None,
                scale: None,
            })
            .collect();
        Clip::new("walk".to_owned(), 1.0, Cycle::Loop, tracks, vec![]).unwrap()
    }

    #[test]
    fn resolve_maps_tracks_to_joints_in_track_order() {
        let binding = ClipBinding::resolve(&clip(&["b", "a"]), &skeleton(&["a", "b"])).unwrap();
        assert_eq!(binding.joints(), &[Some(1), Some(0)]);
        assert_eq!(binding.joint_count(), 2);
        assert_eq!(binding.unbound(), [] as [String; 0]);
    }

    #[test]
    fn resolve_keeps_partially_bound_clips_and_lists_the_missing_nodes() {
        let binding = ClipBinding::resolve(&clip(&["a", "weapon"]), &skeleton(&["a"])).unwrap();
        assert_eq!(binding.joints(), &[Some(0), None]);
        assert_eq!(binding.unbound(), ["weapon"]);
        assert_eq!(binding.bound_joints().collect::<Vec<_>>(), [0]);
    }

    #[test]
    fn resolve_fails_when_no_track_binds() {
        let err = ClipBinding::resolve(&clip(&["x", "y"]), &skeleton(&["a"])).unwrap_err();
        assert_eq!(err.track_count, 2);
        assert_eq!(err.clip, "walk");
    }

    #[test]
    fn resolve_accepts_a_clip_without_tracks() {
        let binding = ClipBinding::resolve(&clip(&[]), &skeleton(&["a"])).unwrap();
        assert_eq!(binding.joints(), []);
    }
}
