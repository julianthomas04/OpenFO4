use glam::Mat4;

use super::skeleton::{Skeleton, Trs};

#[derive(Debug, thiserror::Error)]
pub enum PoseError {
    #[error("skeleton has {skeleton} joints but got {local} local and {model} model slots")]
    LengthMismatch {
        skeleton: usize,
        local: usize,
        model: usize,
    },
}

/// Single forward pass; valid because `Skeleton` guarantees parents precede children.
pub fn local_to_model(
    skeleton: &Skeleton,
    local: &[Trs],
    model: &mut [Mat4],
) -> Result<(), PoseError> {
    if local.len() != skeleton.len() || model.len() != skeleton.len() {
        return Err(PoseError::LengthMismatch {
            skeleton: skeleton.len(),
            local: local.len(),
            model: model.len(),
        });
    }
    for (joint, parent) in skeleton.parents().iter().enumerate() {
        let local_matrix = local[joint].to_matrix();
        model[joint] = match parent {
            Some(parent) => model[*parent as usize] * local_matrix,
            None => local_matrix,
        };
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

    use super::*;

    fn skeleton(parents: Vec<Option<u16>>) -> Skeleton {
        let names = (0..parents.len()).map(|i| format!("j{i}")).collect();
        let rest = vec![Trs::IDENTITY; parents.len()];
        Skeleton::new(names, parents, rest).unwrap()
    }

    fn translation(x: f32, y: f32, z: f32) -> Trs {
        Trs {
            translation: Vec3::new(x, y, z),
            ..Trs::IDENTITY
        }
    }

    #[test]
    fn child_inherits_parent_rotation() {
        // Parent turned 90 degrees about Z; the child sits 1 unit along the parent's X,
        // which is world +Y.
        let skeleton = skeleton(vec![None, Some(0)]);
        let local = [
            Trs {
                rotation: Quat::from_rotation_z(FRAC_PI_2),
                ..Trs::IDENTITY
            },
            translation(1.0, 0.0, 0.0),
        ];
        let mut model = [Mat4::IDENTITY; 2];
        local_to_model(&skeleton, &local, &mut model).unwrap();
        let child_origin = model[1].transform_point3(Vec3::ZERO);
        assert!(
            child_origin.abs_diff_eq(Vec3::new(0.0, 1.0, 0.0), 1e-6),
            "{child_origin}"
        );
    }

    #[test]
    fn deep_chain_accumulates_translations() {
        let skeleton = skeleton(vec![None, Some(0), Some(1), Some(2)]);
        let local = [translation(1.0, 0.0, 0.0); 4];
        let mut model = [Mat4::IDENTITY; 4];
        local_to_model(&skeleton, &local, &mut model).unwrap();
        assert_eq!(model[3].w_axis.truncate(), Vec3::new(4.0, 0.0, 0.0));
    }

    #[test]
    fn sibling_roots_do_not_affect_each_other() {
        let skeleton = skeleton(vec![None, None]);
        let local = [translation(5.0, 0.0, 0.0), translation(0.0, 2.0, 0.0)];
        let mut model = [Mat4::IDENTITY; 2];
        local_to_model(&skeleton, &local, &mut model).unwrap();
        assert_eq!(model[1].w_axis.truncate(), Vec3::new(0.0, 2.0, 0.0));
    }

    #[test]
    fn parent_scale_scales_child_offset() {
        let skeleton = skeleton(vec![None, Some(0)]);
        let local = [
            Trs {
                scale: Vec3::splat(2.0),
                ..Trs::IDENTITY
            },
            translation(1.0, 0.0, 0.0),
        ];
        let mut model = [Mat4::IDENTITY; 2];
        local_to_model(&skeleton, &local, &mut model).unwrap();
        assert_eq!(model[1].w_axis.truncate(), Vec3::new(2.0, 0.0, 0.0));
    }

    #[test]
    fn mismatched_slice_lengths_are_rejected() {
        let skeleton = skeleton(vec![None, Some(0)]);
        let mut model = [Mat4::IDENTITY; 2];
        let err = local_to_model(&skeleton, &[Trs::IDENTITY], &mut model).unwrap_err();
        assert!(matches!(
            err,
            PoseError::LengthMismatch {
                skeleton: 2,
                local: 1,
                model: 2
            }
        ));
        let err = local_to_model(&skeleton, &[Trs::IDENTITY; 2], &mut model[..1]).unwrap_err();
        assert!(matches!(
            err,
            PoseError::LengthMismatch {
                skeleton: 2,
                local: 2,
                model: 1
            }
        ));
    }
}
