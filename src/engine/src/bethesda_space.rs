//! Bethesda (Creation Engine) coordinates to engine space. Bethesda space is
//! Z-up in game units; the engine is Y-up with one engine unit = 10 game units.
//! Every placement, light and authored ragdoll pose goes through these, so they
//! agree on one convention.

use glam::{Quat, Vec3};
use std::f32::consts::FRAC_PI_2;

/// Engine units per Bethesda game unit.
pub const UNITS_TO_ENGINE: f32 = 0.1;

/// Bethesda game units per engine unit.
pub const UNITS_FROM_ENGINE: f32 = 10.0;

/// Engine units per metre of the game world, the Havok metre: it is 1 / 0.0142875 = 69.99 game
/// units, 6.999 engine units (one engine unit is about 14.3 cm), and this is the 7 that collision
/// geometry is baked with. Every constant stated in metres converts through it.
///
/// Rapier is tuned for metre-scale worlds and keeps its internal tolerances (contact margin,
/// sleep thresholds, CCD) right at other scales through `IntegrationParameters.length_unit`, so
/// it must be told the world is this many times metre-scale.
pub const ENGINE_UNITS_PER_METER: f32 = 7.0;

/// A Bethesda vector `(x, y, z)` with its axes mapped to engine axes, unscaled.
pub fn axes_to_engine(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, z, -y)
}

/// An engine vector with its axes mapped back to Bethesda axes, unscaled.
pub fn axes_from_engine(v: Vec3) -> Vec3 {
    Vec3::new(v.x, -v.z, v.y)
}

/// An engine position or offset in Bethesda game units.
pub fn position_from_engine(v: Vec3) -> Vec3 {
    axes_from_engine(v) * UNITS_FROM_ENGINE
}

/// A Bethesda position or offset in engine units.
pub fn position_to_engine(x: f32, y: f32, z: f32) -> Vec3 {
    axes_to_engine(x, y, z) * UNITS_TO_ENGINE
}

/// Engine-space rotation of Bethesda Euler angles (radians), as REFR rotations
/// and XRGD ragdoll bone rotations store them.
pub fn rotation_to_engine(x: f32, y: f32, z: f32) -> Quat {
    Quat::from_rotation_x(-x) * Quat::from_rotation_z(y) * Quat::from_rotation_y(-z)
}

/// Compass heading in degrees for an engine camera yaw (radians, `forward = (cos, _, sin)`):
/// Bethesda north (+Y, engine -Z) is 0, increasing clockwise, rounded to 0.1 and in `[0, 360)`.
pub fn compass_heading_degrees(yaw: f32) -> f32 {
    let degrees = (yaw + FRAC_PI_2).to_degrees();
    let rounded = (degrees.rem_euclid(360.0) * 10.0).round() / 10.0;
    if rounded >= 360.0 { 0.0 } else { rounded }
}

/// Engine camera yaw (radians) that faces a Bethesda Z rotation (radians): rotation 0 faces
/// north (+Y, engine -Z), and the compass forward `(sin z, cos z)` in Bethesda space is
/// `(sin z, 0, -cos z)` in engine space, so `yaw = z - PI/2`.
pub fn yaw_from_rotation_z(rotation_z: f32) -> f32 {
    rotation_z - FRAC_PI_2
}

/// The Bethesda Z rotation (radians) whose facing reads as `degrees` on the compass: the exact
/// inverse of [`compass_heading_degrees`] after [`yaw_from_rotation_z`].
pub fn rotation_z_from_compass_heading_degrees(degrees: f32) -> f32 {
    degrees.to_radians()
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals parsed or copied, not computed"
)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn assert_vec_close(actual: Vec3, expected: Vec3) {
        assert!(
            actual.abs_diff_eq(expected, 1e-5),
            "expected {expected:?}, got {actual:?}"
        );
    }

    #[test]
    fn axes_map_bethesda_z_up_to_engine_y_up_with_y_negated() {
        assert_eq!(axes_to_engine(1.0, 2.0, 3.0), Vec3::new(1.0, 3.0, -2.0));
    }

    #[test]
    fn axes_from_engine_inverts_axes_to_engine() {
        let bethesda = Vec3::new(1.0, -2.0, 3.0);
        assert_vec_close(
            axes_from_engine(axes_to_engine(bethesda.x, bethesda.y, bethesda.z)),
            bethesda,
        );
    }

    #[test]
    fn position_from_engine_inverts_position_to_engine() {
        let engine = position_to_engine(120.0, -340.0, 56.0);
        assert_vec_close(position_from_engine(engine), Vec3::new(120.0, -340.0, 56.0));
    }

    #[test]
    fn position_scales_game_units_to_tenths() {
        assert_vec_close(
            position_to_engine(100.0, 200.0, 300.0),
            Vec3::new(10.0, 30.0, -20.0),
        );
    }

    #[test]
    fn bethesda_north_is_engine_negative_z() {
        assert_vec_close(
            position_to_engine(0.0, 10.0, 0.0),
            Vec3::new(0.0, 0.0, -1.0),
        );
    }

    #[test]
    fn zero_rotation_is_identity() {
        assert!(rotation_to_engine(0.0, 0.0, 0.0).abs_diff_eq(Quat::IDENTITY, 1e-6));
    }

    #[test]
    fn heading_of_ninety_degrees_turns_north_facing_object_to_face_east() {
        let north = axes_to_engine(0.0, 1.0, 0.0);
        let east = axes_to_engine(1.0, 0.0, 0.0);
        assert_vec_close(rotation_to_engine(0.0, 0.0, FRAC_PI_2) * north, east);
    }

    #[test]
    fn positive_pitch_tilts_the_forward_axis_down() {
        let north = axes_to_engine(0.0, 1.0, 0.0);
        let down = axes_to_engine(0.0, 0.0, -1.0);
        assert_vec_close(rotation_to_engine(FRAC_PI_2, 0.0, 0.0) * north, down);
    }

    fn yaw_facing(direction: Vec3) -> f32 {
        direction.z.atan2(direction.x)
    }

    #[test]
    fn compass_heading_reads_zero_north_and_increases_clockwise() {
        let heading = |bethesda: Vec3| {
            compass_heading_degrees(yaw_facing(axes_to_engine(
                bethesda.x, bethesda.y, bethesda.z,
            )))
        };
        assert_eq!(heading(Vec3::Y), 0.0);
        assert_eq!(heading(Vec3::X), 90.0);
        assert_eq!(heading(-Vec3::Y), 180.0);
        assert_eq!(heading(-Vec3::X), 270.0);
    }

    #[test]
    fn compass_heading_wraps_into_zero_inclusive_to_360_exclusive() {
        assert_eq!(compass_heading_degrees(-FRAC_PI_2 - 1e-9), 0.0);
        assert_eq!(compass_heading_degrees(-FRAC_PI_2 - 0.0001), 0.0);
        assert_eq!(compass_heading_degrees(-FRAC_PI_2 - 0.01), 359.4);
        assert_eq!(
            compass_heading_degrees(-FRAC_PI_2 + 4.0 * std::f32::consts::TAU),
            0.0
        );
        assert_eq!(
            compass_heading_degrees(-FRAC_PI_2 - std::f32::consts::TAU),
            0.0
        );
    }

    proptest! {
        #[test]
        fn rotation_preserves_vector_length_for_arbitrary_angles(
            angles in prop::array::uniform3(-7.0f32..7.0),
            v in prop::array::uniform3(-100.0f32..100.0),
        ) {
            let q = rotation_to_engine(angles[0], angles[1], angles[2]);
            let vector = Vec3::from(v);
            prop_assert!(((q * vector).length() - vector.length()).abs() < 1e-3);
        }

        #[test]
        fn heading_alone_never_changes_height(
            heading in -7.0f32..7.0,
            v in prop::array::uniform3(-100.0f32..100.0),
        ) {
            let vector = axes_to_engine(v[0], v[1], v[2]);
            let turned = rotation_to_engine(0.0, 0.0, heading) * vector;
            prop_assert!((turned.y - vector.y).abs() < 1e-3);
        }
    }

    #[test]
    fn a_compass_heading_survives_the_round_trip_through_a_rotation() {
        for tenths in 0..3600 {
            let degrees = tenths as f32 / 10.0;
            let yaw = yaw_from_rotation_z(rotation_z_from_compass_heading_degrees(degrees));
            assert_eq!(compass_heading_degrees(yaw), degrees);
        }
    }

    #[test]
    fn a_rotation_of_zero_faces_north_and_a_quarter_turn_faces_east() {
        assert_eq!(compass_heading_degrees(yaw_from_rotation_z(0.0)), 0.0);
        assert_eq!(
            compass_heading_degrees(yaw_from_rotation_z(FRAC_PI_2)),
            90.0
        );
    }
}
