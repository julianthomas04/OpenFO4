use glam::{Quat, Vec3};

use super::binding::ClipBinding;
use super::clip::{Clip, Interpolation, Keys};
use super::skeleton::Trs;

#[derive(Debug, thiserror::Error)]
pub enum SampleError {
    #[error("pose has {pose} joints but the clip was bound to a skeleton of {bound}")]
    PoseSizeMismatch { pose: usize, bound: usize },
    #[error("binding has {binding} tracks but the clip has {clip}")]
    BindingMismatch { binding: usize, clip: usize },
    #[error("cursors cover {cursors} tracks but the clip has {clip}")]
    CursorMismatch { cursors: usize, clip: usize },
}

/// Remembers the last keyframe segment so monotonic playback finds the next one in O(1).
#[derive(Clone, Copy, Debug, Default)]
pub struct SampleCursor(usize);

#[derive(Clone, Debug)]
pub struct ClipCursors {
    per_track: Vec<[SampleCursor; 3]>,
}

impl ClipCursors {
    pub fn new(clip: &Clip) -> Self {
        Self {
            per_track: vec![[SampleCursor::default(); 3]; clip.tracks().len()],
        }
    }
}

trait Interpolate: Copy {
    fn interpolate(a: Self, b: Self, f: f32) -> Self;
}

impl Interpolate for Vec3 {
    fn interpolate(a: Self, b: Self, f: f32) -> Self {
        a.lerp(b, f)
    }
}

impl Interpolate for Quat {
    fn interpolate(a: Self, b: Self, f: f32) -> Self {
        a.slerp(b, f).normalize()
    }
}

/// Index of the segment `[times[i], times[i + 1])` containing `t`; requires
/// `times[0] < t < times[last]`.
fn locate(times: &[f32], t: f32, cursor: &mut SampleCursor) -> usize {
    let last_segment = times.len() - 2;
    let contains = |i: usize| times[i] <= t && t < times[i + 1];
    let hint = cursor.0.min(last_segment);
    let index = if contains(hint) {
        hint
    } else if hint < last_segment && contains(hint + 1) {
        hint + 1
    } else {
        (times.partition_point(|&k| k <= t) - 1).min(last_segment)
    };
    cursor.0 = index;
    index
}

fn sample_keys<T: Interpolate>(keys: &Keys<T>, t: f32, cursor: &mut SampleCursor) -> T {
    let times = keys.times();
    let values = keys.values();
    let last = times.len() - 1;
    if t <= times[0] {
        return values[0];
    }
    if t >= times[last] {
        return values[last];
    }
    let i = locate(times, t, cursor);
    match keys.interpolation() {
        Interpolation::Step => values[i],
        Interpolation::Linear => {
            let f = (t - times[i]) / (times[i + 1] - times[i]);
            T::interpolate(values[i], values[i + 1], f)
        }
    }
}

/// Writes only the channels the clip animates; callers initialise `pose` from the rest pose.
pub fn sample(
    clip: &Clip,
    binding: &ClipBinding,
    cursors: &mut ClipCursors,
    time: f32,
    pose: &mut [Trs],
) -> Result<(), SampleError> {
    if pose.len() != binding.joint_count() {
        return Err(SampleError::PoseSizeMismatch {
            pose: pose.len(),
            bound: binding.joint_count(),
        });
    }
    if binding.joints().len() != clip.tracks().len() {
        return Err(SampleError::BindingMismatch {
            binding: binding.joints().len(),
            clip: clip.tracks().len(),
        });
    }
    if cursors.per_track.len() != clip.tracks().len() {
        return Err(SampleError::CursorMismatch {
            cursors: cursors.per_track.len(),
            clip: clip.tracks().len(),
        });
    }

    for ((track, &joint), cursor) in clip
        .tracks()
        .iter()
        .zip(binding.joints())
        .zip(&mut cursors.per_track)
    {
        let Some(joint) = joint else { continue };
        let local = &mut pose[joint as usize];
        if let Some(keys) = &track.translation {
            local.translation = sample_keys(keys, time, &mut cursor[0]);
        }
        if let Some(keys) = &track.rotation {
            local.rotation = sample_keys(keys, time, &mut cursor[1]);
        }
        if let Some(keys) = &track.scale {
            local.scale = sample_keys(keys, time, &mut cursor[2]);
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use std::f32::consts::{FRAC_PI_2, PI};

    use proptest::prelude::*;

    use super::*;
    use crate::anim::clip::{Cycle, NodeTrack};
    use crate::anim::skeleton::Skeleton;

    fn skeleton(names: &[&str]) -> Skeleton {
        Skeleton::new(
            names.iter().map(|s| (*s).to_owned()).collect(),
            vec![None; names.len()],
            vec![Trs::IDENTITY; names.len()],
        )
        .unwrap()
    }

    fn translation_track(node: &str, times: &[f32], x: &[f32], mode: Interpolation) -> NodeTrack {
        NodeTrack {
            node_name: node.to_owned(),
            translation: Some(
                Keys::new(
                    times.to_vec(),
                    x.iter().map(|&x| Vec3::new(x, 0.0, 0.0)).collect(),
                    mode,
                )
                .unwrap(),
            ),
            rotation: None,
            scale: None,
        }
    }

    fn rotation_track(node: &str, times: &[f32], rotations: &[Quat]) -> NodeTrack {
        NodeTrack {
            node_name: node.to_owned(),
            translation: None,
            rotation: Some(
                Keys::new(times.to_vec(), rotations.to_vec(), Interpolation::Linear).unwrap(),
            ),
            scale: None,
        }
    }

    struct Fixture {
        clip: Clip,
        binding: ClipBinding,
        cursors: ClipCursors,
        pose: Vec<Trs>,
    }

    impl Fixture {
        fn new(joints: &[&str], tracks: Vec<NodeTrack>) -> Self {
            let clip = Clip::new("c".to_owned(), 10.0, Cycle::Clamp, tracks, vec![]).unwrap();
            let binding = ClipBinding::resolve(&clip, &skeleton(joints)).unwrap();
            let cursors = ClipCursors::new(&clip);
            Self {
                pose: vec![Trs::IDENTITY; joints.len()],
                clip,
                binding,
                cursors,
            }
        }

        fn sample_at(&mut self, time: f32) -> &[Trs] {
            sample(
                &self.clip,
                &self.binding,
                &mut self.cursors,
                time,
                &mut self.pose,
            )
            .unwrap();
            &self.pose
        }
    }

    fn linear_x(times: &[f32], x: &[f32]) -> Fixture {
        Fixture::new(
            &["a"],
            vec![translation_track("a", times, x, Interpolation::Linear)],
        )
    }

    #[test]
    fn linear_interpolates_inside_a_segment() {
        let mut f = linear_x(&[0.0, 2.0], &[10.0, 20.0]);
        assert_eq!(f.sample_at(0.5)[0].translation.x, 12.5);
    }

    #[test]
    fn time_before_the_first_key_holds_the_first_value() {
        let mut f = linear_x(&[1.0, 2.0], &[10.0, 20.0]);
        assert_eq!(f.sample_at(-5.0)[0].translation.x, 10.0);
        assert_eq!(f.sample_at(0.0)[0].translation.x, 10.0);
    }

    #[test]
    fn time_after_the_last_key_holds_the_last_value() {
        let mut f = linear_x(&[1.0, 2.0], &[10.0, 20.0]);
        assert_eq!(f.sample_at(2.0)[0].translation.x, 20.0);
        assert_eq!(f.sample_at(99.0)[0].translation.x, 20.0);
    }

    #[test]
    fn exact_key_times_return_the_key_value() {
        let mut f = linear_x(&[0.0, 1.0, 3.0], &[0.0, 10.0, 50.0]);
        assert_eq!(f.sample_at(1.0)[0].translation.x, 10.0);
    }

    #[test]
    fn single_key_track_is_constant() {
        let mut f = linear_x(&[4.0], &[7.0]);
        assert_eq!(f.sample_at(0.0)[0].translation.x, 7.0);
        assert_eq!(f.sample_at(9.0)[0].translation.x, 7.0);
    }

    #[test]
    fn step_interpolation_holds_the_previous_key_until_the_next() {
        let mut f = Fixture::new(
            &["a"],
            vec![translation_track(
                "a",
                &[0.0, 1.0, 2.0],
                &[1.0, 2.0, 3.0],
                Interpolation::Step,
            )],
        );
        assert_eq!(f.sample_at(0.99)[0].translation.x, 1.0);
        assert_eq!(f.sample_at(1.0)[0].translation.x, 2.0);
        assert_eq!(f.sample_at(1.5)[0].translation.x, 2.0);
    }

    #[test]
    fn segment_lookup_survives_backwards_and_jumping_time() {
        let mut f = linear_x(&[0.0, 1.0, 2.0, 3.0, 4.0], &[0.0, 10.0, 20.0, 30.0, 40.0]);
        for time in [3.5, 0.5, 2.5, 3.5, 1.5, 0.25] {
            assert_eq!(f.sample_at(time)[0].translation.x, time * 10.0, "t={time}");
        }
    }

    #[test]
    fn rotation_takes_the_shortest_arc_when_quaternions_have_opposite_signs() {
        // -q is the same rotation as q; interpolating naively from identity towards
        // -Rz(90deg) would sweep through ~270 degrees instead of 90.
        let target = Quat::from_rotation_z(FRAC_PI_2);
        let mut f = Fixture::new(
            &["a"],
            vec![rotation_track("a", &[0.0, 1.0], &[Quat::IDENTITY, -target])],
        );
        let halfway = f.sample_at(0.5)[0].rotation;
        let expected = Quat::from_rotation_z(FRAC_PI_2 / 2.0);
        assert!(
            halfway.dot(expected).abs() > 0.99999,
            "halfway={halfway:?} expected={expected:?}"
        );
    }

    #[test]
    fn rotation_a_quarter_of_the_way_is_a_quarter_of_the_angle_from_the_first_key() {
        let mut f = Fixture::new(
            &["a"],
            vec![rotation_track(
                "a",
                &[0.0, 1.0],
                &[Quat::IDENTITY, Quat::from_rotation_z(FRAC_PI_2)],
            )],
        );
        let quarter = f.sample_at(0.25)[0].rotation;
        assert!(quarter.dot(Quat::from_rotation_z(FRAC_PI_2 / 4.0)) > 0.99999);
    }

    #[test]
    fn rotation_midpoint_is_a_unit_quaternion() {
        let mut f = Fixture::new(
            &["a"],
            vec![rotation_track(
                "a",
                &[0.0, 1.0],
                &[Quat::IDENTITY, Quat::from_rotation_y(PI * 0.99)],
            )],
        );
        let q = f.sample_at(0.37)[0].rotation;
        assert!((q.length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn unanimated_channels_keep_their_input_values() {
        let mut f = linear_x(&[0.0, 1.0], &[0.0, 1.0]);
        f.pose[0].scale = Vec3::splat(3.0);
        f.pose[0].rotation = Quat::from_rotation_x(1.0);
        let pose = *f.sample_at(0.5).first().unwrap();
        assert_eq!(pose.scale, Vec3::splat(3.0));
        assert_eq!(pose.rotation, Quat::from_rotation_x(1.0));
    }

    #[test]
    fn tracks_for_missing_joints_are_skipped_and_other_joints_untouched() {
        let mut f = Fixture::new(
            &["a", "b"],
            vec![
                translation_track("ghost", &[0.0, 1.0], &[5.0, 5.0], Interpolation::Linear),
                translation_track("b", &[0.0, 1.0], &[8.0, 8.0], Interpolation::Linear),
            ],
        );
        let pose = f.sample_at(0.5);
        assert_eq!(pose[0], Trs::IDENTITY);
        assert_eq!(pose[1].translation.x, 8.0);
    }

    #[test]
    fn sample_rejects_a_pose_of_the_wrong_size() {
        let mut f = linear_x(&[0.0, 1.0], &[0.0, 1.0]);
        let mut small: Vec<Trs> = vec![];
        let err = sample(&f.clip, &f.binding, &mut f.cursors, 0.0, &mut small).unwrap_err();
        assert!(matches!(
            err,
            SampleError::PoseSizeMismatch { pose: 0, bound: 1 }
        ));
    }

    #[test]
    fn sample_rejects_cursors_built_for_another_clip() {
        let mut f = linear_x(&[0.0, 1.0], &[0.0, 1.0]);
        let other = Clip::new("o".to_owned(), 1.0, Cycle::Clamp, vec![], vec![]).unwrap();
        let mut cursors = ClipCursors::new(&other);
        let err = sample(&f.clip, &f.binding, &mut cursors, 0.0, &mut f.pose).unwrap_err();
        assert!(matches!(
            err,
            SampleError::CursorMismatch {
                cursors: 0,
                clip: 1
            }
        ));
    }

    #[test]
    fn sample_rejects_a_binding_resolved_for_another_clip() {
        let mut f = linear_x(&[0.0, 1.0], &[0.0, 1.0]);
        let other = Clip::new("o".to_owned(), 1.0, Cycle::Clamp, vec![], vec![]).unwrap();
        let binding = ClipBinding::resolve(&other, &skeleton(&["a"])).unwrap();
        let err = sample(&f.clip, &binding, &mut f.cursors, 0.0, &mut f.pose).unwrap_err();
        assert!(matches!(
            err,
            SampleError::BindingMismatch {
                binding: 0,
                clip: 1
            }
        ));
    }

    proptest! {
        #[test]
        fn cursor_history_never_changes_the_sampled_value(
            gaps in prop::collection::vec(0.01_f32..2.0, 2..12),
            probes in prop::collection::vec(-1.0_f32..30.0, 1..20),
        ) {
            let mut times = vec![0.0_f32];
            for gap in &gaps {
                times.push(times.last().unwrap() + gap);
            }
            let values: Vec<f32> = (0..times.len()).map(|i| (i * i) as f32).collect();
            let mut warm = linear_x(&times, &values);
            let mut cold = linear_x(&times, &values);
            for probe in probes {
                let from_warm = warm.sample_at(probe)[0].translation.x;
                let from_cold = cold.sample_at(probe)[0].translation.x;
                // a freshly built cursor is the reference
                cold.cursors = ClipCursors::new(&cold.clip);
                prop_assert_eq!(from_warm, from_cold);
            }
        }
    }
}
