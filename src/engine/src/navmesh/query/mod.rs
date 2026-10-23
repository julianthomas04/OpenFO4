pub mod funnel;
pub mod nearest;
pub mod raycast;
pub mod search;

use glam::Vec3;

use super::NavRef;

/// A position known to lie on a specific triangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavPoint {
    pub nav_ref: NavRef,
    pub pos: Vec3,
}
