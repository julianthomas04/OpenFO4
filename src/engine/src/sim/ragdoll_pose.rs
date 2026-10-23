//! Authored ragdoll poses. A REFR of a ragdoll prop may carry an XRGD subrecord:
//! the pose its bones were Havok-settled into in the Creation Kit, which is how
//! the game shows and places it (a skeleton slumped on the floor rather than in
//! its model's rest pose). One 28-byte entry per ragdoll body, in the sidecar's
//! body order: 4 bytes unused, then the bone's position (3 × f32, game units)
//! and rotation (3 × f32 Euler radians, the REFR rotation convention), both
//! relative to the parent body. The root's frame is relative to the REFR.
//! Refs authored against a model with a different bone count carry extra
//! trailing entries, which are ignored, or too few, in which case the bodies
//! past the last entry keep their rest offset from their parent.

use rapier3d::prelude::Pose;

use super::ragdoll_spec::{RagdollSpec, RigidFrame};
use crate::bethesda_space;
use crate::utils::debug_once;

const ENTRY_SIZE: usize = 28;
const ENTRY_HEADER: usize = 4;
/// Largest difference (engine units, before placement scale) between a bone's
/// authored offset from its parent and the same offset at rest. Bones are rigid,
/// so a larger one means the entries do not belong to these bodies.
const BONE_LENGTH_TOLERANCE: f32 = 0.1;

#[derive(Debug, thiserror::Error)]
pub(crate) enum RagdollPoseError {
    #[error("{0} bytes is not a whole number of {ENTRY_SIZE}-byte entries")]
    Truncated(usize),
    #[error("entry {0} is not finite")]
    NonFinite(usize),
    #[error("body {0} is its own ancestor")]
    Cycle(usize),
    #[error("body {body} sits {authored:.3} units from its parent, {rest:.3} at rest")]
    BoneLength {
        body: usize,
        authored: f32,
        rest: f32,
    },
}

/// World pose of every body of `spec` in the authored pose. `refr_pose` and
/// `scale` are the placement's, as for the rest pose.
pub(crate) fn authored_body_poses(
    model: &str,
    spec: &RagdollSpec,
    xrgd: &[u8],
    refr_pose: Pose,
    scale: f32,
) -> Result<Vec<Pose>, RagdollPoseError> {
    let entries = xrgd.len() / ENTRY_SIZE;
    if entries != spec.bodies.len() {
        debug_once(&format!("ragdoll-pose-entries:{model}"), || {
            format!(
                "ragdoll: {model}: XRGD has {entries} entries for {} bodies",
                spec.bodies.len()
            )
        });
    }
    let parents = body_parents(spec);
    let mut locals = decode(xrgd, spec.bodies.len())?;
    locals.extend((locals.len()..spec.bodies.len()).map(|body| rest_local(spec, &parents, body)));
    check_bone_lengths(spec, &locals, &parents)?;
    let mut world: Vec<Option<Pose>> = vec![None; spec.bodies.len()];
    for body in 0..spec.bodies.len() {
        resolve(body, &locals, &parents, refr_pose, scale, &mut world)?;
    }
    Ok(world
        .into_iter()
        .map(|pose| pose.unwrap_or(refr_pose))
        .collect())
}

/// The entries of the first `bodies` bodies as parent-relative engine-space
/// frames, before placement scale.
fn decode(xrgd: &[u8], bodies: usize) -> Result<Vec<Pose>, RagdollPoseError> {
    if !xrgd.len().is_multiple_of(ENTRY_SIZE) {
        return Err(RagdollPoseError::Truncated(xrgd.len()));
    }
    xrgd.as_chunks::<ENTRY_SIZE>()
        .0
        .iter()
        .take(bodies)
        .enumerate()
        .map(|(index, entry)| {
            let mut values = [0.0_f32; 6];
            for (value, bytes) in values
                .iter_mut()
                .zip(entry[ENTRY_HEADER..].as_chunks::<4>().0.iter())
            {
                *value = f32::from_le_bytes(*bytes);
            }
            if !values.iter().all(|value| value.is_finite()) {
                return Err(RagdollPoseError::NonFinite(index));
            }
            let [px, py, pz, rx, ry, rz] = values;
            Ok(Pose::from_parts(
                bethesda_space::position_to_engine(px, py, pz),
                bethesda_space::rotation_to_engine(rx, ry, rz),
            ))
        })
        .collect()
}

/// The pose hierarchy: the sidecar's node parents, else (older sidecars) the parent of
/// the constraint each body is the child of.
fn body_parents(spec: &RagdollSpec) -> Vec<Option<usize>> {
    if let Some(node_parents) = &spec.node_parents {
        return node_parents.clone();
    }
    let mut parents = vec![None; spec.bodies.len()];
    for constraint in &spec.constraints {
        parents[constraint.child] = Some(constraint.parent);
    }
    parents
}

/// The rest pose of `body` relative to its parent body, or to the model origin for a root.
fn rest_local(spec: &RagdollSpec, parents: &[Option<usize>], body: usize) -> Pose {
    let rest = rigid_pose(&spec.bodies[body].rest);
    match parents[body] {
        Some(parent) => rigid_pose(&spec.bodies[parent].rest).inverse() * rest,
        None => rest,
    }
}

fn rigid_pose(frame: &RigidFrame) -> Pose {
    Pose::from_parts(frame.translation, frame.rotation)
}

fn check_bone_lengths(
    spec: &RagdollSpec,
    locals: &[Pose],
    parents: &[Option<usize>],
) -> Result<(), RagdollPoseError> {
    for (body, parent) in parents.iter().enumerate() {
        let Some(parent) = *parent else { continue };
        let (parent_rest, child_rest) = (&spec.bodies[parent].rest, &spec.bodies[body].rest);
        let rest =
            parent_rest.rotation.inverse() * (child_rest.translation - parent_rest.translation);
        let authored = locals[body].translation.length();
        if (authored - rest.length()).abs() > BONE_LENGTH_TOLERANCE {
            return Err(RagdollPoseError::BoneLength {
                body,
                authored,
                rest: rest.length(),
            });
        }
    }
    Ok(())
}

/// Resolves `body` and every unresolved ancestor, root first.
fn resolve(
    body: usize,
    locals: &[Pose],
    parents: &[Option<usize>],
    refr_pose: Pose,
    scale: f32,
    world: &mut [Option<Pose>],
) -> Result<(), RagdollPoseError> {
    let mut chain = vec![body];
    let mut anchor = refr_pose;
    while let Some(&current) = chain.last() {
        if let Some(pose) = world[current] {
            chain.pop();
            anchor = pose;
            break;
        }
        match parents[current] {
            Some(parent) if chain.contains(&parent) => return Err(RagdollPoseError::Cycle(parent)),
            Some(parent) => chain.push(parent),
            None => break,
        }
    }
    while let Some(current) = chain.pop() {
        let local = locals[current];
        anchor *= Pose::from_parts(local.translation * scale, local.rotation);
        world[current] = Some(anchor);
    }
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

    use glam::{Quat, Vec3};
    use serde_json::json;

    use super::*;
    use crate::sim::ragdoll_spec::parse_ragdoll_spec;

    /// The XRGD entry layout: 4 unused bytes, position, Euler rotation.
    fn entry(position: [f32; 3], rotation: [f32; 3]) -> Vec<u8> {
        std::iter::repeat_n(0_u8, ENTRY_HEADER)
            .chain(
                position
                    .iter()
                    .chain(&rotation)
                    .flat_map(|v| v.to_le_bytes()),
            )
            .collect()
    }

    fn blob(entries: &[Vec<u8>]) -> Vec<u8> {
        entries.concat()
    }

    fn body_json(rest: [f32; 3], node_parent: Option<usize>) -> serde_json::Value {
        json!({
            "node": "n", "mass": 1.0, "anchor": false, "damping": [0.0, 0.0],
            "rest": { "t": rest, "r": [0.0, 0.0, 0.0, 1.0] },
            "shapes": [{ "type": "sphere", "radius": 0.5 }],
            "node_parent": node_parent,
        })
    }

    /// Root at the origin and a child 1 engine unit along +X (Bethesda +X is engine +X).
    fn chain_spec() -> RagdollSpec {
        let sidecar = json!({
            "bodies": [body_json([0.0, 0.0, 0.0], None), body_json([1.0, 0.0, 0.0], Some(0))],
            "constraints": [],
        });
        parse_ragdoll_spec(&serde_json::to_vec(&sidecar).unwrap()).unwrap()
    }

    fn refr(translation: Vec3) -> Pose {
        Pose::from_translation(translation)
    }

    #[test]
    fn identity_entries_reproduce_the_rest_pose_at_the_placement() {
        let spec = chain_spec();
        let xrgd = blob(&[entry([0.0; 3], [0.0; 3]), entry([10.0, 0.0, 0.0], [0.0; 3])]);
        let poses =
            authored_body_poses("m", &spec, &xrgd, refr(Vec3::new(5.0, 0.0, 0.0)), 1.0).unwrap();
        assert!(
            poses[0]
                .translation
                .abs_diff_eq(Vec3::new(5.0, 0.0, 0.0), 1e-5)
        );
        assert!(
            poses[1]
                .translation
                .abs_diff_eq(Vec3::new(6.0, 0.0, 0.0), 1e-5)
        );
    }

    #[test]
    fn placement_scale_stretches_bone_offsets_but_not_the_root_anchor() {
        let spec = chain_spec();
        let xrgd = blob(&[entry([0.0; 3], [0.0; 3]), entry([10.0, 0.0, 0.0], [0.0; 3])]);
        let poses =
            authored_body_poses("m", &spec, &xrgd, refr(Vec3::new(5.0, 0.0, 0.0)), 2.0).unwrap();
        assert!(
            poses[1]
                .translation
                .abs_diff_eq(Vec3::new(7.0, 0.0, 0.0), 1e-5)
        );
    }

    #[test]
    fn a_rotated_root_swings_its_child_around_it() {
        // Bethesda yaw of 90 degrees (z) becomes -90 degrees about engine Y, which carries
        // the child from +X to +Z (hand calculation: R_y(-90) * (1, 0, 0) = (0, 0, 1)).
        let spec = chain_spec();
        let xrgd = blob(&[
            entry([0.0; 3], [0.0, 0.0, FRAC_PI_2]),
            entry([10.0, 0.0, 0.0], [0.0; 3]),
        ]);
        let poses = authored_body_poses("m", &spec, &xrgd, Pose::IDENTITY, 1.0).unwrap();
        assert!(
            poses[1]
                .translation
                .abs_diff_eq(Vec3::new(0.0, 0.0, 1.0), 1e-5),
            "{:?}",
            poses[1].translation
        );
        let expected = Quat::from_rotation_y(-FRAC_PI_2);
        assert!(poses[0].rotation.dot(expected).abs() > 0.99999);
    }

    #[test]
    fn missing_trailing_entries_keep_their_rest_offset_from_the_parent() {
        let spec = chain_spec();
        let xrgd = blob(&[entry([0.0; 3], [0.0; 3])]);
        let poses =
            authored_body_poses("m", &spec, &xrgd, refr(Vec3::new(0.0, 3.0, 0.0)), 1.0).unwrap();
        assert!(
            poses[1]
                .translation
                .abs_diff_eq(Vec3::new(1.0, 3.0, 0.0), 1e-5)
        );
    }

    #[test]
    fn surplus_trailing_entries_are_ignored() {
        let spec = chain_spec();
        let xrgd = blob(&[
            entry([0.0; 3], [0.0; 3]),
            entry([10.0, 0.0, 0.0], [0.0; 3]),
            entry([999.0, 999.0, 999.0], [0.0; 3]),
        ]);
        let poses = authored_body_poses("m", &spec, &xrgd, Pose::IDENTITY, 1.0).unwrap();
        assert_eq!(poses.len(), 2);
    }

    #[test]
    fn a_blob_that_is_not_a_whole_number_of_entries_is_rejected() {
        let spec = chain_spec();
        let mut xrgd = blob(&[entry([0.0; 3], [0.0; 3])]);
        xrgd.pop();
        let err = authored_body_poses("m", &spec, &xrgd, Pose::IDENTITY, 1.0).unwrap_err();
        assert!(matches!(err, RagdollPoseError::Truncated(27)));
    }

    #[test]
    fn a_non_finite_entry_is_rejected_with_its_index() {
        let spec = chain_spec();
        let xrgd = blob(&[
            entry([0.0; 3], [0.0; 3]),
            entry([f32::NAN, 0.0, 0.0], [0.0; 3]),
        ]);
        let err = authored_body_poses("m", &spec, &xrgd, Pose::IDENTITY, 1.0).unwrap_err();
        assert!(matches!(err, RagdollPoseError::NonFinite(1)));
    }

    #[test]
    fn a_bone_stretched_beyond_tolerance_is_rejected() {
        let spec = chain_spec();
        // 2 engine units authored against a rest length of 1.
        let xrgd = blob(&[entry([0.0; 3], [0.0; 3]), entry([20.0, 0.0, 0.0], [0.0; 3])]);
        let err = authored_body_poses("m", &spec, &xrgd, Pose::IDENTITY, 1.0).unwrap_err();
        assert!(matches!(err, RagdollPoseError::BoneLength { body: 1, .. }));
    }

    #[test]
    fn a_bone_within_the_length_tolerance_is_accepted() {
        let spec = chain_spec();
        // 1.05 engine units: 0.05 longer than at rest, inside the 0.1 tolerance.
        let xrgd = blob(&[entry([0.0; 3], [0.0; 3]), entry([10.5, 0.0, 0.0], [0.0; 3])]);
        assert!(authored_body_poses("m", &spec, &xrgd, Pose::IDENTITY, 1.0).is_ok());
    }

    #[test]
    fn without_node_parents_the_constraint_hierarchy_is_used() {
        let sidecar = json!({
            "bodies": [
                { "node": "a", "mass": 1.0, "anchor": false, "damping": [0.0, 0.0],
                  "rest": { "t": [0.0, 0.0, 0.0], "r": [0.0, 0.0, 0.0, 1.0] },
                  "shapes": [{ "type": "sphere", "radius": 0.5 }] },
                { "node": "b", "mass": 1.0, "anchor": false, "damping": [0.0, 0.0],
                  "rest": { "t": [1.0, 0.0, 0.0], "r": [0.0, 0.0, 0.0, 1.0] },
                  "shapes": [{ "type": "sphere", "radius": 0.5 }] },
            ],
            "constraints": [{
                "type": "limited_hinge", "child": 1, "parent": 0,
                "frame_child": { "t": [0, 0, 0], "r": [0, 0, 0, 1] },
                "frame_parent": { "t": [0, 0, 0], "r": [0, 0, 0, 1] },
                "hinge": [-1.0, 1.0], "friction": 0.0,
            }],
        });
        let spec = parse_ragdoll_spec(&serde_json::to_vec(&sidecar).unwrap()).unwrap();
        assert!(spec.node_parents.is_none());
        let xrgd = blob(&[entry([0.0; 3], [0.0; 3])]);
        let poses = authored_body_poses("m", &spec, &xrgd, Pose::IDENTITY, 1.0).unwrap();
        assert!(
            poses[1]
                .translation
                .abs_diff_eq(Vec3::new(1.0, 0.0, 0.0), 1e-5)
        );
    }

    proptest::proptest! {
        #[test]
        fn arbitrary_blobs_never_panic_and_either_fail_or_yield_one_pose_per_body(
            bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..200),
        ) {
            let spec = chain_spec();
            if let Ok(poses) = authored_body_poses("m", &spec, &bytes, Pose::IDENTITY, 1.0) {
                proptest::prop_assert_eq!(poses.len(), spec.bodies.len());
            }
        }
    }
}
