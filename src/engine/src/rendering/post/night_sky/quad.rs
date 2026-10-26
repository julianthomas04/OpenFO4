//! The sky sprites of the moon and the sun: a quad on the tangent plane of the unit sphere.

use glam::Vec3;

/// Where the up axis of a sky quad points while its body stands near the zenith or nadir and
/// the world up axis cannot orient it.
const POLAR_UP_FALLBACK: Vec3 = Vec3::NEG_Z;
const POLAR_COS_LIMIT: f32 = 0.999;

/// The unit right and up axes of a sky quad as seen from a camera looking along the unit
/// `direction`, with the world up axis pointing up the picture. Where the body passes the
/// zenith or nadir the reference axis switches and the quad rolls by a sudden quarter turn.
pub(super) fn quad_basis(direction: Vec3) -> (Vec3, Vec3) {
    let reference = if direction.y.abs() > POLAR_COS_LIMIT {
        POLAR_UP_FALLBACK
    } else {
        Vec3::Y
    };
    let right = direction.cross(reference).normalize();
    (right, right.cross(direction))
}
