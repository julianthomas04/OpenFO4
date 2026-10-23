//! What counts as a pose the broad phase can be fed.
//!
//! One collider with a NaN or overflowing bound makes parry's binned BVH build index past its
//! last bin, so poses and bounds are checked against these limits before they reach the world.

use glam::Vec3;
use rapier3d::prelude::*;

/// Largest coordinate magnitude the broad phase may be fed; the BVH builder squares extents, so
/// anything past this overflows `f32` long before it is a plausible world position.
const COORDINATE_LIMIT: f32 = 1.0e18;

pub(super) fn vec_is_sound(value: Vec3) -> bool {
    value.is_finite() && value.abs().max_element() <= COORDINATE_LIMIT
}

pub(super) fn pose_is_sound(pose: &Pose) -> bool {
    vec_is_sound(pose.translation) && pose.rotation.is_finite()
}

pub(super) fn aabb_is_sound(aabb: &Aabb) -> bool {
    vec_is_sound(aabb.mins) && vec_is_sound(aabb.maxs) && aabb.mins.cmple(aabb.maxs).all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_infinite_or_overflowing_coordinate_is_unsound() {
        assert!(!vec_is_sound(Vec3::X * f32::INFINITY));
        assert!(!vec_is_sound(Vec3::Y * 2.0 * COORDINATE_LIMIT));
        assert!(!vec_is_sound(Vec3::new(f32::NAN, 0.0, 0.0)));
        assert!(vec_is_sound(Vec3::new(10.0, 0.0, -4.0)));
    }
}
