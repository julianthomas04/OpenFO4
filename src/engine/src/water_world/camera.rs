//! Whether the camera is under water, as Fallout 4 decides it each frame
//! (the water system's camera test and update): the water
//! height is queried at the bottom and then the top point of the camera's
//! virtual near plane, a point is under water when the height is at or above
//! it, and the share of the near plane's height below the surface is the
//! immersion fraction. The fraction maps to the four immersion states.

use glam::Vec3;

use crate::bethesda_space::UNITS_TO_ENGINE;

/// Distance of the virtual near plane the corners are projected onto: 15.0
/// game units, independent of the render camera's near plane.
pub const VIRTUAL_NEAR_DISTANCE: f32 = 15.0 * UNITS_TO_ENGINE;

/// Immersion fraction at which state 1 (shallow) turns into state 2:
/// `frac < 0.5 ? 1 : frac < 1 ? 2 : 3`.
const SHALLOW_STATE_LIMIT: f32 = 0.5;

/// How deep the camera's near plane sits in the water.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Immersion {
    /// State 0: above the water, or the surface exactly at the plane's lowest
    /// point (fraction 0).
    Above,
    /// State 1: under, and the surface is below half the near plane's height.
    Shallow,
    /// State 2: under, and the surface is at least halfway up but below the top.
    Deep,
    /// State 3: the whole near plane is under water.
    Full,
}

impl Immersion {
    fn of_fraction(fraction: f32) -> Self {
        if fraction >= 1.0 {
            Self::Full
        } else if fraction >= SHALLOW_STATE_LIMIT {
            Self::Deep
        } else if fraction > 0.0 {
            Self::Shallow
        } else {
            Self::Above
        }
    }
}

/// Where the camera sits relative to the water, as the frame uniform needs it.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct CameraWater {
    pub underwater: bool,
    /// World-space height of the water surface at the camera (engine units);
    /// meaningless (0) when there is no water.
    pub surface_height: f32,
    /// The share of the near plane's height under the surface, 0 to 1.
    pub fraction: f32,
    pub immersion: Immersion,
    /// The near plane point whose water height decided the test, where the
    /// water volume the camera is in can be looked up; meaningless (zero)
    /// when there is no water.
    pub probe: Vec3,
}

impl CameraWater {
    pub const NONE: Self = Self {
        underwater: false,
        surface_height: 0.0,
        fraction: 0.0,
        immersion: Immersion::Above,
        probe: Vec3::ZERO,
    };
}

impl Default for CameraWater {
    fn default() -> Self {
        Self::NONE
    }
}

/// The virtual near plane: the points the water height is queried at and the
/// heights of its lowest and highest corner (the fraction's range).
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct NearPlane {
    pub bottom: Vec3,
    pub top: Vec3,
    /// Height of the plane's lowest corner.
    pub lowest: f32,
    /// Height of the plane's highest corner.
    pub highest: f32,
}

impl NearPlane {
    /// A plane through `bottom` and `top` whose corners span exactly their
    /// heights (a camera without roll).
    pub fn spanning(bottom: Vec3, top: Vec3) -> Self {
        Self {
            bottom,
            top,
            lowest: bottom.y.min(top.y),
            highest: bottom.y.max(top.y),
        }
    }

    /// `eye`, unit `forward` and `up` hint (any vector not parallel to
    /// `forward`), the vertical field of view in radians, the aspect ratio
    /// (width over height) and the near distance. The corner heights follow
    /// `z_i = z0 + d * (fwd_z +/- up_z * ext_v +/- right_z * ext_h)`.
    ///
    /// The extents `ext_v`, `ext_h` are the half-extents of the near plane at unit
    /// distance (`tan(fovy / 2)` and that times the aspect). The query
    /// points are the vertical middle line of the plane.
    pub fn new(eye: Vec3, forward: Vec3, up_hint: Vec3, fovy: f32, aspect: f32, near: f32) -> Self {
        let up = (up_hint - forward * up_hint.dot(forward))
            .try_normalize()
            .unwrap_or(Vec3::Y);
        let right = forward.cross(up).normalize_or_zero();
        let center = eye + forward * near;
        let ext_v = (fovy * 0.5).tan();
        let ext_h = ext_v * aspect;
        let half_height = near * ext_v;
        let corner_heights =
            [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)].map(|(vertical, horizontal)| {
                eye.y + near * (forward.y + vertical * up.y * ext_v + horizontal * right.y * ext_h)
            });
        Self {
            bottom: center - up * half_height,
            top: center + up * half_height,
            lowest: corner_heights.into_iter().fold(f32::INFINITY, f32::min),
            highest: corner_heights.into_iter().fold(f32::NEG_INFINITY, f32::max),
        }
    }
}

/// The camera's relation to the water. `height_at` answers the water height
/// query for an engine-space point (heights in engine units).
pub fn camera_water(near: NearPlane, height_at: impl Fn(Vec3) -> Option<f32>) -> CameraWater {
    let under = |point: Vec3| {
        height_at(point)
            .filter(|height| *height >= point.y)
            .map(|height| (height, point))
    };
    let Some((height, probe)) = under(near.bottom).or_else(|| under(near.top)) else {
        return CameraWater::NONE;
    };
    let fraction = if near.highest > near.lowest {
        ((height - near.lowest) / (near.highest - near.lowest)).clamp(0.0, 1.0)
    } else {
        1.0
    };
    CameraWater {
        underwater: true,
        surface_height: height,
        fraction,
        immersion: Immersion::of_fraction(fraction),
        probe,
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact in f32 or compared with an epsilon"
)]
mod tests {
    use super::*;

    fn plane(bottom: f32, top: f32) -> NearPlane {
        NearPlane::spanning(Vec3::new(0.0, bottom, 0.0), Vec3::new(0.0, top, 0.0))
    }

    fn flat(height: f32) -> impl Fn(Vec3) -> Option<f32> {
        move |_| Some(height)
    }

    #[test]
    fn the_virtual_near_plane_is_15_game_units_ahead() {
        assert!((VIRTUAL_NEAR_DISTANCE - 1.5).abs() < 1e-6);
    }

    #[test]
    fn no_water_height_means_not_underwater() {
        assert_eq!(camera_water(plane(0.0, 1.0), |_| None), CameraWater::NONE);
    }

    #[test]
    fn a_near_plane_above_the_surface_is_not_underwater() {
        assert_eq!(camera_water(plane(2.0, 3.0), flat(1.0)), CameraWater::NONE);
    }

    #[test]
    fn a_surface_a_quarter_up_the_near_plane_is_shallow() {
        let water = camera_water(plane(0.0, 4.0), flat(1.0));
        assert!(water.underwater);
        assert_eq!(water.immersion, Immersion::Shallow);
        assert!((water.fraction - 0.25).abs() < 1e-6);
        assert_eq!(water.surface_height, 1.0);
    }

    #[test]
    fn a_surface_three_quarters_up_is_deep() {
        let water = camera_water(plane(0.0, 4.0), flat(3.0));
        assert_eq!(water.immersion, Immersion::Deep);
        assert!((water.fraction - 0.75).abs() < 1e-6);
    }

    #[test]
    fn exactly_half_way_up_is_state_two() {
        let water = camera_water(plane(0.0, 4.0), flat(2.0));
        assert_eq!(water.immersion, Immersion::Deep);
    }

    #[test]
    fn a_surface_above_the_top_point_is_fully_under() {
        let water = camera_water(plane(0.0, 4.0), flat(9.0));
        assert_eq!(water.immersion, Immersion::Full);
        assert_eq!(water.fraction, 1.0);
    }

    #[test]
    fn the_surface_exactly_at_the_bottom_point_is_under_but_in_state_zero() {
        let water = camera_water(plane(1.0, 2.0), flat(1.0));
        assert!(water.underwater);
        assert_eq!(water.fraction, 0.0);
        assert_eq!(water.immersion, Immersion::Above);
    }

    #[test]
    fn a_flat_near_plane_under_the_surface_is_fully_immersed() {
        let water = camera_water(plane(1.0, 1.0), flat(5.0));
        assert_eq!(water.fraction, 1.0);
        assert_eq!(water.immersion, Immersion::Full);
    }

    #[test]
    fn the_top_point_is_tried_when_the_bottom_point_has_no_water() {
        // Water exists only around the higher point.
        let height_at = |point: Vec3| (point.y > 1.5).then_some(10.0);
        let water = camera_water(plane(0.0, 2.0), height_at);
        assert!(water.underwater);
        assert_eq!(water.immersion, Immersion::Full);
        assert_eq!(water.probe, Vec3::new(0.0, 2.0, 0.0));
    }

    #[test]
    fn the_bottom_points_water_wins_over_the_tops() {
        let height_at = |point: Vec3| Some(if point.y < 1.0 { 0.5 } else { 7.0 });
        let water = camera_water(plane(0.0, 2.0), height_at);
        assert_eq!(water.surface_height, 0.5);
        assert_eq!(water.probe, Vec3::ZERO);
    }

    #[test]
    fn the_near_plane_extends_along_the_cameras_up_direction() {
        let near = NearPlane::new(
            Vec3::new(0.0, 10.0, 0.0),
            Vec3::NEG_Z,
            Vec3::Y,
            std::f32::consts::FRAC_PI_2,
            1.0,
            0.5,
        );
        assert!(near.bottom.abs_diff_eq(Vec3::new(0.0, 9.5, -0.5), 1e-5));
        assert!(near.top.abs_diff_eq(Vec3::new(0.0, 10.5, -0.5), 1e-5));
        assert!((near.lowest - 9.5).abs() < 1e-5);
        assert!((near.highest - 10.5).abs() < 1e-5);
    }

    #[test]
    fn looking_straight_down_still_gives_a_near_plane() {
        let near = NearPlane::new(
            Vec3::ZERO,
            Vec3::NEG_Y,
            Vec3::Y,
            std::f32::consts::FRAC_PI_2,
            16.0 / 9.0,
            1.0,
        );
        assert!(near.bottom.is_finite() && near.top.is_finite());
        assert!((near.bottom - near.top).length() > 1.9);
    }

    #[test]
    fn a_rolled_camera_spans_the_corners_heights_not_the_middle_lines() {
        let forward = Vec3::NEG_Z;
        let rolled_up = Vec3::new(1.0, 1.0, 0.0).normalize();
        let level = NearPlane::new(Vec3::ZERO, forward, Vec3::Y, 1.0, 2.0, 1.0);
        let rolled = NearPlane::new(Vec3::ZERO, forward, rolled_up, 1.0, 2.0, 1.0);
        assert!(rolled.highest - rolled.lowest > level.highest - level.lowest);
        let ext_v = 0.5f32.tan();
        let expected = 2.0 * (ext_v * rolled_up.y + 2.0 * ext_v * rolled_up.x);
        assert!((rolled.highest - rolled.lowest - expected).abs() < 1e-5);
    }

    #[test]
    fn straight_down_all_corners_share_one_height() {
        let near = NearPlane::new(Vec3::ZERO, Vec3::NEG_Y, Vec3::NEG_Z, 1.0, 1.5, 1.0);
        assert!((near.highest - near.lowest).abs() < 1e-6);
        assert_eq!(near.lowest, -1.0);
    }
}
