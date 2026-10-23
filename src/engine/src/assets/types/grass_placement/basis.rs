//! A blade's orientation from its stored yaw and up axis, as built when an
//! instance is placed.
//!
//! Without fit-to-slope the blade is the model yawed about Bethesda Z. With it
//! the yaw is applied in a frame built from the terrain normal `n`: the
//! tangent `t` is perpendicular to `n` and zero along the axis where `|n|` is
//! smallest (ties prefer x, then y), the bitangent is `b = t x n`, and the
//! yaw matrix columns `(c, s, 0)`, `(-s, c, 0)`, `(0, 0, 1)` become
//! `c*b + s*t`, `c*t - s*b` and `n`.

use glam::{Mat3, Vec3};

use crate::bethesda_space::{axes_from_engine, axes_to_engine};

/// Tangent perpendicular to `normal` (Bethesda space), unit length; the zero
/// vector for a degenerate normal.
fn fit_tangent(normal: Vec3) -> Vec3 {
    let abs = normal.abs();
    let tangent = if abs.x <= abs.y && abs.x <= abs.z {
        Vec3::new(0.0, -normal.z, normal.y)
    } else if abs.y <= abs.x && abs.y <= abs.z {
        Vec3::new(normal.z, 0.0, -normal.x)
    } else {
        Vec3::new(-normal.y, normal.x, 0.0)
    };
    if tangent.length() > 1.0e-6 {
        tangent.normalize()
    } else {
        Vec3::ZERO
    }
}

/// The rotation taking a model's engine-space axes (Y up, as the converted
/// GLBs have) to the blade's orientation in engine space.
///
/// `up` is the blade's stored up axis (engine space); `fit_to_slope` is the
/// grass type's flag. Only without the flag is `up` ignored.
pub fn blade_rotation(yaw_cos: f32, up: Vec3, fit_to_slope: bool) -> Mat3 {
    let c = yaw_cos;
    let s = (1.0 - c * c).max(0.0).sqrt();
    let (column_x, column_y, column_z) = if fit_to_slope {
        let normal = axes_from_engine(up);
        let tangent = fit_tangent(normal);
        let bitangent = tangent.cross(normal);
        (
            c * bitangent + s * tangent,
            c * tangent - s * bitangent,
            normal,
        )
    } else {
        (Vec3::new(c, s, 0.0), Vec3::new(-s, c, 0.0), Vec3::Z)
    };
    // Engine axes of a model: x = Bethesda x, y = Bethesda z, z = -Bethesda y.
    Mat3::from_cols(
        axes_to_engine(column_x.x, column_x.y, column_x.z),
        axes_to_engine(column_z.x, column_z.y, column_z.z),
        -axes_to_engine(column_y.x, column_y.y, column_y.z),
    )
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals or exact table entries"
)]
mod tests {
    use super::*;

    const EPSILON: f32 = 1e-5;

    fn assert_close(actual: Vec3, expected: Vec3) {
        assert!(
            (actual - expected).length() < EPSILON,
            "{actual:?} vs {expected:?}"
        );
    }

    #[test]
    fn without_fit_the_blade_yaws_about_engine_up() {
        // yaw 90 degrees: c = 0, s = 1. Bethesda x -> +y; engine x -> -z.
        let rotation = blade_rotation(0.0, Vec3::Y, false);
        assert_close(rotation * Vec3::Y, Vec3::Y);
        assert_close(rotation * Vec3::X, Vec3::new(0.0, 0.0, -1.0));
        assert_close(rotation * Vec3::Z, Vec3::X);
    }

    #[test]
    fn without_fit_the_up_argument_is_ignored() {
        assert_eq!(
            blade_rotation(0.3, Vec3::new(0.6, 0.8, 0.0), false),
            blade_rotation(0.3, Vec3::Y, false)
        );
    }

    #[test]
    fn with_fit_the_model_up_maps_to_the_terrain_normal() {
        let normal = Vec3::new(0.3, 0.9, -0.2).normalize();
        for yaw_cos in [-0.8, 0.0, 0.5, 1.0] {
            let rotation = blade_rotation(yaw_cos, normal, true);
            assert_close(rotation * Vec3::Y, normal);
        }
    }

    #[test]
    fn with_fit_the_rotation_is_orthonormal_and_right_handed() {
        let normal = Vec3::new(-0.5, 0.7, 0.4).normalize();
        let rotation = blade_rotation(0.35, normal, true);
        assert!((rotation.determinant() - 1.0).abs() < EPSILON);
        assert!(
            (rotation * rotation.transpose() - Mat3::IDENTITY).abs_diff_eq(Mat3::ZERO, EPSILON)
        );
    }

    #[test]
    fn with_fit_the_tangent_axis_follows_the_smallest_normal_component() {
        // Bethesda normal (0.6, 0.8, 0) is smallest in z: t = (-ny, nx, 0)/|..| = (-0.8, 0.6, 0);
        // b = t x n = (0, 0, -1); c = 1 puts the model x on b and the model y on t.
        let up_engine = axes_to_engine(0.6, 0.8, 0.0);
        let rotation = blade_rotation(1.0, up_engine, true);
        assert_close(rotation * Vec3::X, axes_to_engine(0.0, 0.0, -1.0));
        assert_close(rotation * Vec3::Z, -axes_to_engine(-0.8, 0.6, 0.0));
    }

    #[test]
    fn the_third_column_is_the_cross_of_the_first_two() {
        // The shader receives only `right` and `up` and rebuilds the frame as
        // mat3(right, up, cross(right, up)).
        let normal = Vec3::new(0.3, 0.9, -0.2).normalize();
        for (yaw_cos, up, fit) in [
            (0.35, normal, true),
            (-0.8, normal, true),
            (0.35, Vec3::Y, false),
            (-0.8, Vec3::Y, false),
        ] {
            let rotation = blade_rotation(yaw_cos, up, fit);
            let (right, up) = (rotation.x_axis, rotation.y_axis);
            assert_close(rotation.z_axis, right.cross(up));
        }
    }

    #[test]
    fn a_vertical_normal_yields_a_valid_frame() {
        // Bethesda (0,0,1): |nx| = |ny| = 0 -> x-smallest branch, t = (0,-1,0), b = (-1,0,0).
        let rotation = blade_rotation(1.0, Vec3::Y, true);
        assert_close(rotation * Vec3::Y, Vec3::Y);
        assert_close(rotation * Vec3::X, axes_to_engine(-1.0, 0.0, 0.0));
    }
}
