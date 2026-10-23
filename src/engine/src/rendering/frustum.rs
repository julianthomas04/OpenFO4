use glam::{Mat4, Vec3, Vec4};

/// Scales a plane `(n, d)` to a unit normal; a degenerate plane becomes zero,
/// which culls nothing.
pub fn normalize_plane(plane: Vec4) -> Vec4 {
    let length = plane.truncate().length();
    if length > 0.0 {
        plane / length
    } else {
        Vec4::ZERO
    }
}

/// The six clip-space boundary planes of a projection with a [0, 1] depth
/// range: left, right, bottom, top, depth 0, depth 1. Points inside satisfy
/// `dot(plane.xyz, p) + plane.w >= 0`.
pub fn clip_planes(m: &Mat4) -> [Vec4; 6] {
    let [r0, r1, r2, r3] = [m.row(0), m.row(1), m.row(2), m.row(3)];
    [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2].map(normalize_plane)
}

#[derive(Clone, Copy, Debug)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn from_points(positions: impl Iterator<Item = Vec3>) -> Option<Self> {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut any = false;
        for p in positions {
            min = min.min(p);
            max = max.max(p);
            any = true;
        }
        any.then_some(Self { min, max })
    }

    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn half_extents(&self) -> Vec3 {
        (self.max - self.min) * 0.5
    }

    /// Bounds of the eight transformed corners.
    #[must_use]
    pub fn transformed(&self, matrix: &Mat4) -> Self {
        let mut bounds = Self {
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
        };
        for corner in self.corners() {
            let point = matrix.transform_point3(corner);
            bounds.min = bounds.min.min(point);
            bounds.max = bounds.max.max(point);
        }
        bounds
    }

    /// True when the box lies entirely behind one of the inward-facing planes.
    pub fn is_outside(&self, planes: &[Vec4]) -> bool {
        planes.iter().any(|plane| {
            let normal = plane.truncate();
            let nearest_corner = Vec3::select(normal.cmpge(Vec3::ZERO), self.max, self.min);
            normal.dot(nearest_corner) + plane.w < 0.0
        })
    }

    pub fn corners(&self) -> [Vec3; 8] {
        let (mn, mx) = (self.min, self.max);
        [
            Vec3::new(mn.x, mn.y, mn.z),
            Vec3::new(mx.x, mn.y, mn.z),
            Vec3::new(mn.x, mx.y, mn.z),
            Vec3::new(mx.x, mx.y, mn.z),
            Vec3::new(mn.x, mn.y, mx.z),
            Vec3::new(mx.x, mn.y, mx.z),
            Vec3::new(mn.x, mx.y, mx.z),
            Vec3::new(mx.x, mx.y, mx.z),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_box() -> Aabb {
        Aabb {
            min: Vec3::splat(-1.0),
            max: Vec3::splat(1.0),
        }
    }

    /// Planes of the unit cube [-1, 1]^3, normals pointing inwards.
    fn cube_planes() -> [Vec4; 6] {
        [
            Vec4::new(1.0, 0.0, 0.0, 1.0),
            Vec4::new(-1.0, 0.0, 0.0, 1.0),
            Vec4::new(0.0, 1.0, 0.0, 1.0),
            Vec4::new(0.0, -1.0, 0.0, 1.0),
            Vec4::new(0.0, 0.0, 1.0, 1.0),
            Vec4::new(0.0, 0.0, -1.0, 1.0),
        ]
    }

    fn inside_all(planes: &[Vec4; 6], point: Vec3) -> bool {
        planes
            .iter()
            .all(|plane| plane.truncate().dot(point) + plane.w >= 0.0)
    }

    #[test]
    fn normalize_plane_scales_to_a_unit_normal() {
        let plane = normalize_plane(Vec4::new(0.0, 3.0, 4.0, 10.0));
        assert!(
            plane.abs_diff_eq(Vec4::new(0.0, 0.6, 0.8, 2.0), 1e-6),
            "{plane}"
        );
    }

    #[test]
    fn normalize_plane_turns_a_degenerate_plane_into_one_that_culls_nothing() {
        assert_eq!(normalize_plane(Vec4::new(0.0, 0.0, 0.0, 5.0)), Vec4::ZERO);
    }

    #[test]
    fn clip_planes_of_an_orthographic_box_bound_that_box() {
        // View-space box: x in [-2, 2], y in [-3, 3], z in [-10, 0].
        let projection = Mat4::orthographic_rh(-2.0, 2.0, -3.0, 3.0, 0.0, 10.0);
        let planes = clip_planes(&projection);
        let inside = |p: Vec3| inside_all(&planes, p);
        assert!(inside(Vec3::new(0.0, 0.0, -5.0)));
        assert!(inside(Vec3::new(1.9, -2.9, -0.1)));
        assert!(!inside(Vec3::new(2.1, 0.0, -5.0)));
        assert!(!inside(Vec3::new(-2.1, 0.0, -5.0)));
        assert!(!inside(Vec3::new(0.0, 3.1, -5.0)));
        assert!(!inside(Vec3::new(0.0, -3.1, -5.0)));
    }

    #[test]
    fn clip_planes_have_unit_normals() {
        let planes = clip_planes(&Mat4::perspective_rh(1.0, 1.5, 0.1, 100.0));
        for plane in planes {
            assert!((plane.truncate().length() - 1.0).abs() < 1e-5, "{plane}");
        }
    }

    #[test]
    fn clip_planes_of_a_perspective_frustum_accept_points_in_view_only() {
        let projection = Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 1.0, 100.0);
        let planes = clip_planes(&projection);
        // 90 degree fov: |x| <= -z and |y| <= -z for z in [-100, -1].
        assert!(inside_all(&planes, Vec3::new(0.0, 0.0, -50.0)));
        assert!(inside_all(&planes, Vec3::new(49.0, -49.0, -50.0)));
        assert!(!inside_all(&planes, Vec3::new(51.0, 0.0, -50.0)));
        assert!(!inside_all(&planes, Vec3::new(0.0, 0.0, -0.5)));
        assert!(!inside_all(&planes, Vec3::new(0.0, 0.0, -101.0)));
        assert!(!inside_all(&planes, Vec3::new(0.0, 0.0, 5.0)));
    }

    #[test]
    fn clip_planes_of_the_engine_camera_accept_what_the_camera_sees() {
        use crate::rendering::camera::Camera;
        let camera = Camera {
            eye: Vec3::new(10.0, 5.0, 0.0),
            target: Vec3::new(10.0, 5.0, -1.0),
            up: Vec3::Y,
            aspect: 1.0,
            fovy: 90.0,
            znear: 1.0,
            zfar: 1000.0,
        };
        let planes = clip_planes(&camera.build_view_projection_matrix());
        assert!(inside_all(&planes, Vec3::new(10.0, 5.0, -500.0)));
        assert!(!inside_all(&planes, Vec3::new(10.0, 5.0, 500.0)));
        assert!(!inside_all(&planes, Vec3::new(10.0, 5.0, -0.5)));
        assert!(!inside_all(&planes, Vec3::new(10.0 + 600.0, 5.0, -500.0)));
    }

    #[test]
    fn a_box_wholly_behind_one_plane_is_outside() {
        let far_right = Aabb {
            min: Vec3::new(1.5, -0.5, -0.5),
            max: Vec3::new(2.5, 0.5, 0.5),
        };
        assert!(far_right.is_outside(&cube_planes()));
        let far_left = Aabb {
            min: Vec3::new(-2.5, -0.5, -0.5),
            max: Vec3::new(-1.5, 0.5, 0.5),
        };
        assert!(far_left.is_outside(&cube_planes()));
    }

    #[test]
    fn a_box_straddling_a_plane_is_kept() {
        let straddling = Aabb {
            min: Vec3::new(0.5, -0.5, -0.5),
            max: Vec3::new(1.5, 0.5, 0.5),
        };
        assert!(!straddling.is_outside(&cube_planes()));
    }

    #[test]
    fn a_box_touching_a_plane_from_outside_is_kept() {
        let touching = Aabb {
            min: Vec3::new(1.0, -0.5, -0.5),
            max: Vec3::new(2.0, 0.5, 0.5),
        };
        assert!(!touching.is_outside(&cube_planes()));
    }

    #[test]
    fn a_box_outside_only_diagonally_is_kept_because_planes_are_tested_one_by_one() {
        let past_the_corner = Aabb {
            min: Vec3::new(0.9, 0.9, 0.9),
            max: Vec3::new(1.5, 1.5, 1.5),
        };
        assert!(!past_the_corner.is_outside(&cube_planes()));
    }

    #[test]
    fn a_box_is_judged_by_the_corner_nearest_the_plane_normal() {
        // Plane x + y >= 0: only the corner towards (+, +) decides.
        let planes = [Vec4::new(1.0, 1.0, 0.0, 0.0)];
        let behind = Aabb {
            min: Vec3::new(-3.0, -3.0, 0.0),
            max: Vec3::new(1.0, -1.5, 1.0),
        };
        let reaching = Aabb {
            min: Vec3::new(-3.0, -3.0, 0.0),
            max: Vec3::new(1.0, -0.5, 1.0),
        };
        assert!(behind.is_outside(&planes));
        assert!(!reaching.is_outside(&planes));
    }

    #[test]
    fn from_points_bounds_the_points() {
        let bounds =
            Aabb::from_points([Vec3::new(1.0, 5.0, -2.0), Vec3::new(-3.0, 2.0, 4.0)].into_iter())
                .unwrap();
        assert_eq!(bounds.min, Vec3::new(-3.0, 2.0, -2.0));
        assert_eq!(bounds.max, Vec3::new(1.0, 5.0, 4.0));
    }

    #[test]
    fn from_points_of_nothing_is_none() {
        assert!(Aabb::from_points(std::iter::empty()).is_none());
    }

    #[test]
    fn union_covers_both_boxes() {
        let a = unit_box();
        let b = Aabb {
            min: Vec3::new(0.0, 0.0, 0.0),
            max: Vec3::new(4.0, 0.5, 0.5),
        };
        let union = a.union(&b);
        assert_eq!(union.min, Vec3::splat(-1.0));
        assert_eq!(union.max, Vec3::new(4.0, 1.0, 1.0));
    }

    #[test]
    fn transformed_bounds_of_a_rotated_box_grow_to_enclose_it() {
        let bounds = Aabb {
            min: Vec3::new(-1.0, -1.0, 0.0),
            max: Vec3::new(1.0, 1.0, 2.0),
        };
        let rotated = bounds.transformed(&Mat4::from_rotation_z(std::f32::consts::FRAC_PI_4));
        let expected = std::f32::consts::SQRT_2;
        assert!((rotated.max.x - expected).abs() < 1e-5);
        assert!((rotated.min.y + expected).abs() < 1e-5);
        assert!((rotated.max.z - 2.0).abs() < 1e-5);
    }

    #[test]
    fn transformed_bounds_follow_a_translation() {
        let moved = unit_box().transformed(&Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0)));
        assert_eq!(moved.min, Vec3::new(9.0, -1.0, -1.0));
        assert_eq!(moved.max, Vec3::new(11.0, 1.0, 1.0));
    }
}
