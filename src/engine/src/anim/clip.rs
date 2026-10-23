use std::collections::HashSet;

use glam::{Quat, Vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cycle {
    Loop,
    Clamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    Step,
    Linear,
}

#[derive(Debug, thiserror::Error)]
pub enum ClipError {
    #[error("keyframe track has {times} times but {values} values")]
    KeyCountMismatch { times: usize, values: usize },
    #[error("keyframe track is empty")]
    NoKeys,
    #[error("keyframe time {time} at index {index} is not finite")]
    NonFiniteKeyTime { index: usize, time: f32 },
    #[error("keyframe times decrease at index {index} ({previous} then {time})")]
    KeyTimesDecreasing {
        index: usize,
        previous: f32,
        time: f32,
    },
    #[error("clip {clip:?} has invalid duration {duration}")]
    InvalidDuration { clip: String, duration: f32 },
    #[error("clip {clip:?} has a marker {text:?} at non-finite time {time}")]
    NonFiniteMarkerTime {
        clip: String,
        text: String,
        time: f32,
    },
    #[error("clip {clip:?} has more than one track for node {node:?}")]
    DuplicateTrack { clip: String, node: String },
}

#[derive(Clone, Debug)]
pub struct Keys<T> {
    times: Vec<f32>,
    values: Vec<T>,
    interpolation: Interpolation,
    collapsed_duplicates: usize,
}

impl<T> Keys<T> {
    pub fn new(
        times: Vec<f32>,
        values: Vec<T>,
        interpolation: Interpolation,
    ) -> Result<Self, ClipError> {
        if times.len() != values.len() {
            return Err(ClipError::KeyCountMismatch {
                times: times.len(),
                values: values.len(),
            });
        }
        if times.is_empty() {
            return Err(ClipError::NoKeys);
        }
        if let Some((index, &time)) = times.iter().enumerate().find(|(_, t)| !t.is_finite()) {
            return Err(ClipError::NonFiniteKeyTime { index, time });
        }
        if let Some(index) = times.windows(2).position(|w| w[1] < w[0]) {
            return Err(ClipError::KeyTimesDecreasing {
                index: index + 1,
                previous: times[index],
                time: times[index + 1],
            });
        }
        let (times, values, collapsed_duplicates) = collapse_equal_times(times, values);
        Ok(Self {
            times,
            values,
            interpolation,
            collapsed_duplicates,
        })
    }

    /// How many keys were dropped because a later key had the same time.
    pub fn collapsed_duplicates(&self) -> usize {
        self.collapsed_duplicates
    }

    pub fn times(&self) -> &[f32] {
        &self.times
    }

    pub fn values(&self) -> &[T] {
        &self.values
    }

    pub fn interpolation(&self) -> Interpolation {
        self.interpolation
    }

    pub fn last_time(&self) -> f32 {
        self.times[self.times.len() - 1]
    }
}

/// Keeps the last value of each run of equal times (`times` is non-decreasing).
fn collapse_equal_times<T>(times: Vec<f32>, values: Vec<T>) -> (Vec<f32>, Vec<T>, usize) {
    let total = times.len();
    let mut kept_times: Vec<f32> = Vec::with_capacity(total);
    let mut kept_values: Vec<T> = Vec::with_capacity(total);
    for (time, value) in times.into_iter().zip(values) {
        if kept_times.last() == Some(&time) {
            *kept_values.last_mut().expect("values parallel times") = value;
        } else {
            kept_times.push(time);
            kept_values.push(value);
        }
    }
    let collapsed = total - kept_times.len();
    (kept_times, kept_values, collapsed)
}

#[derive(Clone, Debug)]
pub struct NodeTrack {
    pub node_name: String,
    pub translation: Option<Keys<Vec3>>,
    pub rotation: Option<Keys<Quat>>,
    pub scale: Option<Keys<Vec3>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub time: f32,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Clip {
    name: String,
    duration: f32,
    cycle: Cycle,
    tracks: Vec<NodeTrack>,
    markers: Vec<Marker>,
}

impl Clip {
    pub fn new(
        name: String,
        duration: f32,
        cycle: Cycle,
        tracks: Vec<NodeTrack>,
        mut markers: Vec<Marker>,
    ) -> Result<Self, ClipError> {
        if !duration.is_finite() || duration < 0.0 {
            return Err(ClipError::InvalidDuration {
                clip: name,
                duration,
            });
        }
        if let Some(marker) = markers.iter().find(|m| !m.time.is_finite()) {
            return Err(ClipError::NonFiniteMarkerTime {
                clip: name,
                text: marker.text.clone(),
                time: marker.time,
            });
        }
        let mut seen = HashSet::with_capacity(tracks.len());
        if let Some(track) = tracks.iter().find(|t| !seen.insert(t.node_name.as_str())) {
            return Err(ClipError::DuplicateTrack {
                clip: name,
                node: track.node_name.clone(),
            });
        }
        markers.sort_by(|a, b| a.time.total_cmp(&b.time));
        Ok(Self {
            name,
            duration,
            cycle,
            tracks,
            markers,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn duration(&self) -> f32 {
        self.duration
    }

    pub fn cycle(&self) -> Cycle {
        self.cycle
    }

    pub fn tracks(&self) -> &[NodeTrack] {
        &self.tracks
    }

    /// Sorted ascending by time.
    pub fn markers(&self) -> &[Marker] {
        &self.markers
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use super::*;

    fn linear<T>(times: Vec<f32>, values: Vec<T>) -> Result<Keys<T>, ClipError> {
        Keys::new(times, values, Interpolation::Linear)
    }

    fn track(node: &str) -> NodeTrack {
        NodeTrack {
            node_name: node.to_owned(),
            translation: None,
            rotation: None,
            scale: None,
        }
    }

    fn clip_with(
        duration: f32,
        tracks: Vec<NodeTrack>,
        markers: Vec<Marker>,
    ) -> Result<Clip, ClipError> {
        Clip::new("c".to_owned(), duration, Cycle::Clamp, tracks, markers)
    }

    #[test]
    fn keys_reject_length_mismatch() {
        let err = linear(vec![0.0, 1.0], vec![Vec3::ZERO]).unwrap_err();
        assert!(matches!(
            err,
            ClipError::KeyCountMismatch {
                times: 2,
                values: 1
            }
        ));
    }

    #[test]
    fn keys_reject_empty_track() {
        let err = linear::<Vec3>(vec![], vec![]).unwrap_err();
        assert!(matches!(err, ClipError::NoKeys));
    }

    #[test]
    fn keys_reject_non_finite_time_and_report_its_index() {
        let err = linear(vec![0.0, f32::NAN], vec![Vec3::ZERO; 2]).unwrap_err();
        assert!(matches!(err, ClipError::NonFiniteKeyTime { index: 1, .. }));
        let err = linear(vec![f32::INFINITY], vec![Vec3::ZERO]).unwrap_err();
        assert!(matches!(err, ClipError::NonFiniteKeyTime { index: 0, .. }));
    }

    #[test]
    fn keys_reject_decreasing_times_at_the_offending_index() {
        let err = linear(vec![0.0, 2.0, 1.0], vec![Vec3::ZERO; 3]).unwrap_err();
        assert!(matches!(
            err,
            ClipError::KeyTimesDecreasing { index: 2, previous, time } if previous == 2.0 && time == 1.0
        ));
    }

    #[test]
    fn keys_collapse_equal_times_keeping_the_last_value() {
        let keys = linear(
            vec![0.0, 1.0, 1.0, 1.0, 2.0],
            vec![1.0_f32, 2.0, 3.0, 4.0, 5.0],
        )
        .unwrap();
        assert_eq!(keys.times(), &[0.0, 1.0, 2.0]);
        assert_eq!(keys.values(), &[1.0, 4.0, 5.0]);
        assert_eq!(keys.collapsed_duplicates(), 2);
    }

    #[test]
    fn keys_accept_a_single_key() {
        let keys = linear(vec![3.0], vec![7.0_f32]).unwrap();
        assert_eq!(keys.last_time(), 3.0);
    }

    #[test]
    fn clip_rejects_invalid_durations() {
        for duration in [-0.001, f32::NAN, f32::INFINITY] {
            let err = clip_with(duration, vec![], vec![]).unwrap_err();
            assert!(
                matches!(err, ClipError::InvalidDuration { .. }),
                "{duration}"
            );
        }
    }

    #[test]
    fn clip_accepts_zero_duration_pose_clip() {
        assert_eq!(clip_with(0.0, vec![], vec![]).unwrap().duration(), 0.0);
    }

    #[test]
    fn clip_rejects_two_tracks_for_the_same_node() {
        let err = clip_with(1.0, vec![track("a"), track("b"), track("a")], vec![]).unwrap_err();
        assert!(matches!(err, ClipError::DuplicateTrack { node, .. } if node == "a"));
    }

    #[test]
    fn clip_rejects_non_finite_marker_time() {
        let marker = Marker {
            time: f32::NAN,
            text: "hit".to_owned(),
        };
        let err = clip_with(1.0, vec![], vec![marker]).unwrap_err();
        assert!(matches!(err, ClipError::NonFiniteMarkerTime { text, .. } if text == "hit"));
    }

    #[test]
    fn clip_sorts_markers_by_time() {
        let marker = |time, text: &str| Marker {
            time,
            text: text.to_owned(),
        };
        let clip = clip_with(
            1.0,
            vec![],
            vec![marker(0.9, "c"), marker(0.1, "a"), marker(0.5, "b")],
        )
        .unwrap();
        let texts: Vec<&str> = clip.markers().iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, ["a", "b", "c"]);
    }

    proptest::proptest! {
        #[test]
        fn collapsed_keys_are_strictly_increasing_and_lose_exactly_the_duplicates(
            steps in proptest::collection::vec(0_u8..4, 1..40),
        ) {
            let mut times = vec![0.0_f32];
            for step in &steps {
                times.push(times.last().unwrap() + f32::from(*step));
            }
            let values: Vec<f32> = (0..times.len()).map(|i| i as f32).collect();
            let total = times.len();
            let keys = Keys::new(times, values, Interpolation::Linear).unwrap();
            proptest::prop_assert!(keys.times().windows(2).all(|w| w[0] < w[1]));
            proptest::prop_assert_eq!(keys.times().len() + keys.collapsed_duplicates(), total);
            proptest::prop_assert_eq!(keys.values().len(), keys.times().len());
        }
    }
}
