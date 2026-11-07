//! Where the loading screen's camera stands and how wide it sees, read off the stage mesh the
//! game draws behind the model. The stage is two planes facing the camera at different depths
//! that both fill the screen from top to bottom; only one camera position and one field of view
//! make two planes of different heights do that.

use glam::{Mat4, Vec2, Vec3};

/// A rectangle of the stage facing the camera, in engine space (the camera looks along -Z).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StagePlane {
    pub centre: Vec2,
    pub half_extent: Vec2,
    pub depth: f32,
}

impl StagePlane {
    /// The plane through the axis-aligned bounds of `positions`; `None` without positions, or
    /// when they do not lie in one plane facing the camera or have no height.
    pub fn from_positions(positions: impl IntoIterator<Item = Vec3>) -> Option<Self> {
        let (min, max) = positions.into_iter().fold(None, |bounds, position| {
            Some(match bounds {
                None => (position, position),
                Some((min, max)) => (position.min(min), position.max(max)),
            })
        })?;
        let half_extent = ((max - min) / 2.0).truncate();
        let facing_the_camera = (max.z - min.z).abs() <= f32::EPSILON * max.z.abs().max(1.0);
        (facing_the_camera && half_extent.y > 0.0).then(|| Self {
            centre: ((min + max) / 2.0).truncate(),
            half_extent,
            depth: f32::midpoint(min.z, max.z),
        })
    }

    /// Width over height.
    pub fn aspect(&self) -> f32 {
        self.half_extent.x / self.half_extent.y
    }
}

#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum StageFramingError {
    #[error("the stage's planes stand at the same depth")]
    SameDepth,
    #[error("the stage's farther plane is not taller than its nearer one")]
    FarNotTaller,
}

/// The camera that sees both planes of the stage exactly from top to bottom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StageFraming {
    /// Where the camera stands, in the stage's space.
    pub eye: Vec3,
    pub vertical_fov_radians: f32,
}

impl StageFraming {
    /// Solves `near.half_height / (eye - near.depth) = far.half_height / (eye - far.depth)` for
    /// the camera's depth `eye`; the field of view follows from either plane. The camera stands
    /// on the axis through the nearer plane's centre.
    pub fn of_planes(a: StagePlane, b: StagePlane) -> Result<Self, StageFramingError> {
        if (a.depth - b.depth).abs() <= f32::EPSILON * a.depth.abs().max(b.depth.abs()).max(1.0) {
            return Err(StageFramingError::SameDepth);
        }
        let (near, far) = if a.depth > b.depth { (a, b) } else { (b, a) };
        let (near_half, far_half) = (near.half_extent.y, far.half_extent.y);
        if far_half <= near_half {
            return Err(StageFramingError::FarNotTaller);
        }
        let eye_depth = (far_half * near.depth - near_half * far.depth) / (far_half - near_half);
        let distance = eye_depth - near.depth;
        Ok(Self {
            eye: near.centre.extend(eye_depth),
            vertical_fov_radians: 2.0 * (near_half / distance).atan(),
        })
    }

    /// The view of the stage from [`Self::eye`], looking along -Z.
    pub fn stage_view(&self) -> Mat4 {
        Mat4::look_to_rh(self.eye, Vec3::NEG_Z, Vec3::Y)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// The overlay plane of the game's `LoadScreen02` in engine units: 16.75 game units deep,
    /// 186.5 x 105 half size.
    pub const VANILLA_OVERLAY: StagePlane = StagePlane {
        centre: Vec2::ZERO,
        half_extent: Vec2::new(18.65, 10.5),
        depth: -1.6756,
    };

    /// The background plane of the game's `LoadScreen02` in engine units: about 1000 game units
    /// deep, 557.5 x 313.75 half size.
    pub const VANILLA_BACKGROUND: StagePlane = StagePlane {
        centre: Vec2::ZERO,
        half_extent: Vec2::new(55.75, 31.375),
        depth: -100.0044,
    };

    /// The framing of the game's own stage.
    pub fn vanilla_framing() -> StageFraming {
        StageFraming::of_planes(VANILLA_OVERLAY, VANILLA_BACKGROUND).expect("the stage frames")
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[test]
    fn the_game_stage_gives_a_vertical_field_of_view_of_about_24_degrees() {
        let framing = vanilla_framing();
        assert!(
            (framing.vertical_fov_radians.to_degrees() - 23.98).abs() < 0.02,
            "{}",
            framing.vertical_fov_radians.to_degrees()
        );
        assert!(
            (framing.eye.z - 47.78).abs() < 0.02,
            "the camera stands 477.8 game units before the stage's origin: {}",
            framing.eye.z
        );
    }

    #[test]
    fn both_planes_fill_the_view_from_top_to_bottom() {
        let framing = vanilla_framing();
        for plane in [VANILLA_OVERLAY, VANILLA_BACKGROUND] {
            let view_proj = Mat4::perspective_rh(framing.vertical_fov_radians, 1.0, 0.1, 1000.0)
                * framing.stage_view();
            let top = view_proj.project_point3(Vec3::new(0.0, plane.half_extent.y, plane.depth));
            let bottom =
                view_proj.project_point3(Vec3::new(0.0, -plane.half_extent.y, plane.depth));
            assert!((top.y - 1.0).abs() < 1e-4, "{top:?}");
            assert!((bottom.y + 1.0).abs() < 1e-4, "{bottom:?}");
        }
    }

    #[test]
    fn the_order_of_the_planes_does_not_matter() {
        assert_eq!(
            StageFraming::of_planes(VANILLA_BACKGROUND, VANILLA_OVERLAY),
            Ok(vanilla_framing())
        );
    }

    #[test]
    fn the_camera_stands_on_the_axis_of_the_nearer_plane() {
        let shift = Vec2::new(3.0, -2.0);
        let overlay = StagePlane {
            centre: shift,
            ..VANILLA_OVERLAY
        };
        let framing = StageFraming::of_planes(overlay, VANILLA_BACKGROUND).unwrap();
        assert_eq!(framing.eye.truncate(), shift);
    }

    #[test]
    fn planes_that_no_single_camera_fits_are_rejected() {
        let level = StagePlane {
            depth: VANILLA_OVERLAY.depth,
            ..VANILLA_BACKGROUND
        };
        assert_eq!(
            StageFraming::of_planes(VANILLA_OVERLAY, level),
            Err(StageFramingError::SameDepth)
        );
        let as_tall = StagePlane {
            half_extent: VANILLA_OVERLAY.half_extent,
            ..VANILLA_BACKGROUND
        };
        assert_eq!(
            StageFraming::of_planes(VANILLA_OVERLAY, as_tall),
            Err(StageFramingError::FarNotTaller)
        );
    }

    #[test]
    fn a_plane_is_read_off_the_bounds_of_its_corners() {
        let corners = [
            (-18.65, -10.5),
            (18.65, -10.5),
            (18.65, 10.5),
            (-18.65, 10.5),
        ]
        .map(|(x, y)| Vec3::new(x, y, -1.6756));
        let plane = StagePlane::from_positions(corners).expect("a plane");
        assert_eq!(plane, VANILLA_OVERLAY);
        assert!((plane.aspect() - 16.0 / 9.0).abs() < 0.01);
    }

    #[test]
    fn positions_that_are_no_plane_facing_the_camera_give_none() {
        assert_eq!(StagePlane::from_positions([]), None);
        let tilted = [Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, 1.0, -1.0)];
        assert_eq!(StagePlane::from_positions(tilted), None);
        let flat = [Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 0.0, -1.0)];
        assert_eq!(StagePlane::from_positions(flat), None);
    }
}
