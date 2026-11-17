//! A convex hull grown by its Havok convex radius.
//!
//! Havok stores a convex shape as an inner hull plus a radius; the surface the game collides with
//! is the inner hull grown by that radius on every side. A radius of zero is the bare hull.

use glam::Vec3;
use rapier3d::prelude::SharedShape;

/// A hull whose points stay this close (engine units) to a plane is a plate, and one that stays
/// this close to a line is a segment: parry cannot build a rounded hull from coplanar points.
const FLAT_TOLERANCE: f32 = 1.0e-3;
/// Points nearer to each other than this are one point.
const COINCIDENT_POINTS: f32 = 1.0e-6;
/// The plate prism's half thickness: a fraction of the plate's largest extent, never below the
/// minimum. Thick enough for the hull builder to see volume, far below the radius it carries.
const PLATE_HALF_THICKNESS_PER_EXTENT: f32 = 1.0e-4;
const PLATE_MIN_HALF_THICKNESS: f32 = 1.0e-3;

/// What the points of a hull span.
enum Footprint {
    Solid,
    /// Coplanar: the plane passes through `origin` with the unit `normal`.
    Plate {
        origin: Vec3,
        normal: Vec3,
        extent: f32,
    },
    Segment {
        from: Vec3,
        to: Vec3,
    },
    Point(Vec3),
}

fn farthest_from(points: &[Vec3], origin: Vec3) -> Vec3 {
    points
        .iter()
        .copied()
        .max_by(|a, b| {
            a.distance_squared(origin)
                .total_cmp(&b.distance_squared(origin))
        })
        .unwrap_or(origin)
}

fn footprint(points: &[Vec3]) -> Footprint {
    let Some(&first) = points.first() else {
        return Footprint::Point(Vec3::ZERO);
    };
    let from = farthest_from(points, first);
    let to = farthest_from(points, from);
    let extent = from.distance(to);
    if extent <= COINCIDENT_POINTS {
        return Footprint::Point(first);
    }
    let axis = (to - from) / extent;
    let widest = points
        .iter()
        .copied()
        .max_by(|a, b| {
            (*a - from)
                .cross(axis)
                .length_squared()
                .total_cmp(&(*b - from).cross(axis).length_squared())
        })
        .unwrap_or(from);
    if (widest - from).cross(axis).length() <= FLAT_TOLERANCE {
        return Footprint::Segment { from, to };
    }
    let normal = axis.cross(widest - from).normalize();
    let thickness = points
        .iter()
        .map(|point| normal.dot(*point - from).abs())
        .fold(0.0, f32::max);
    if thickness <= FLAT_TOLERANCE {
        Footprint::Plate {
            origin: from,
            normal,
            extent,
        }
    } else {
        Footprint::Solid
    }
}

/// The plate's polygon lifted to a prism of the plate's tiny half thickness, so it has a volume
/// the hull builder accepts. With the radius, the plate is `2 * radius` thick plus that sliver.
fn plate_prism(points: &[Vec3], origin: Vec3, normal: Vec3, extent: f32) -> Vec<Vec3> {
    let half_thickness = (extent * PLATE_HALF_THICKNESS_PER_EXTENT).max(PLATE_MIN_HALF_THICKNESS);
    points
        .iter()
        .flat_map(|point| {
            let on_plane = *point - normal * normal.dot(*point - origin);
            [
                on_plane + normal * half_thickness,
                on_plane - normal * half_thickness,
            ]
        })
        .collect()
}

/// A cube of the plate's minimum half thickness around `point`: the hull builder needs volume,
/// and grown by the radius it is a ball at `point`.
fn point_cube(point: Vec3) -> Vec<Vec3> {
    (0..8)
        .map(|corner| {
            let sign = |bit: u32| if corner & bit == 0 { -1.0 } else { 1.0 };
            point + Vec3::new(sign(1), sign(2), sign(4)) * PLATE_MIN_HALF_THICKNESS
        })
        .collect()
}

/// The hull of `points` grown by `radius`, or `None` when there are no points, a point or the
/// radius is not finite, or the hull builder rejects them. A radius of zero builds the bare hull
/// exactly as `SharedShape::convex_hull` does (a flat hull stays a zero-thickness polyhedron, a
/// line or a point has none). With a radius the degenerate hulls are what Havok stores them as:
/// coplanar points a plate, points on a line the capsule around that segment, one point a ball.
pub(crate) fn grown_hull(points: &[Vec3], radius: f32) -> Option<SharedShape> {
    if points.iter().any(|point| !point.is_finite()) || !radius.is_finite() {
        return None;
    }
    if radius <= 0.0 {
        return SharedShape::convex_hull(points);
    }
    if points.is_empty() {
        return None;
    }
    match footprint(points) {
        Footprint::Solid => SharedShape::round_convex_hull(points, radius),
        Footprint::Plate {
            origin,
            normal,
            extent,
        } => SharedShape::round_convex_hull(&plate_prism(points, origin, normal, extent), radius),
        Footprint::Segment { from, to } => Some(SharedShape::capsule(from, to, radius)),
        Footprint::Point(point) => SharedShape::round_convex_hull(&point_cube(point), radius),
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use rapier3d::parry::query::Ray;
    use rapier3d::prelude::Pose;

    use super::*;

    fn unit_box_points() -> Vec<Vec3> {
        (0..8)
            .map(|corner| {
                Vec3::new(
                    if corner & 1 == 0 { -0.5 } else { 0.5 },
                    if corner & 2 == 0 { -0.5 } else { 0.5 },
                    if corner & 4 == 0 { -0.5 } else { 0.5 },
                )
            })
            .collect()
    }

    fn square_plate_points(half: f32) -> Vec<Vec3> {
        vec![
            Vec3::new(-half, 0.0, -half),
            Vec3::new(half, 0.0, -half),
            Vec3::new(half, 0.0, half),
            Vec3::new(-half, 0.0, half),
        ]
    }

    /// Distance from `origin` along `direction` to the shape, `None` when it is missed.
    fn hit_distance(shape: &SharedShape, origin: Vec3, direction: Vec3) -> Option<f32> {
        shape.cast_ray(&Pose::IDENTITY, &Ray::new(origin, direction), 100.0, true)
    }

    #[test]
    fn a_radius_of_zero_builds_the_bare_hull() {
        let shape = grown_hull(&unit_box_points(), 0.0).unwrap();
        assert!(shape.as_convex_polyhedron().is_some());
        assert!(shape.as_round_convex_polyhedron().is_none());
    }

    #[test]
    fn a_radius_builds_a_round_hull_with_that_radius() {
        let shape = grown_hull(&unit_box_points(), 0.25).unwrap();
        assert_eq!(
            shape.as_round_convex_polyhedron().unwrap().border_radius,
            0.25
        );
    }

    #[test]
    fn a_ray_hits_the_grown_surface_that_it_misses_on_the_bare_hull() {
        let bare = grown_hull(&unit_box_points(), 0.0).unwrap();
        let grown = grown_hull(&unit_box_points(), 0.25).unwrap();
        let origin = Vec3::new(0.0, 0.0, 2.0);
        let ray_past_the_face = Vec3::new(0.0, 0.0, -1.0);
        assert!((hit_distance(&bare, origin, ray_past_the_face).unwrap() - 1.5).abs() < 1e-5);
        assert!((hit_distance(&grown, origin, ray_past_the_face).unwrap() - 1.25).abs() < 1e-3);

        let grazing_origin = Vec3::new(0.65, 0.0, 2.0);
        assert_eq!(hit_distance(&bare, grazing_origin, ray_past_the_face), None);
        assert!(hit_distance(&grown, grazing_origin, ray_past_the_face).is_some());
    }

    #[test]
    fn a_hull_with_a_radius_larger_than_its_half_extent_is_a_valid_plate() {
        let thin_slab: Vec<Vec3> = unit_box_points()
            .into_iter()
            .map(|point| Vec3::new(point.x, point.y * 0.01, point.z))
            .collect();
        let shape = grown_hull(&thin_slab, 0.3).unwrap();
        let from_above = hit_distance(&shape, Vec3::new(0.0, 5.0, 0.0), -Vec3::Y).unwrap();
        assert!((from_above - (5.0 - 0.305)).abs() < 1e-3, "{from_above}");
    }

    #[test]
    fn a_flat_hull_without_radius_stays_a_zero_thickness_polyhedron() {
        let shape = grown_hull(&square_plate_points(1.0), 0.0).unwrap();
        assert!(shape.as_convex_polyhedron().is_some());
        let line = [Vec3::ZERO, Vec3::X, Vec3::X * 2.0, Vec3::X * 3.0];
        assert!(grown_hull(&line, 0.0).is_none());
    }

    #[test]
    fn a_flat_hull_with_a_radius_is_a_plate_twice_the_radius_thick() {
        let shape = grown_hull(&square_plate_points(1.0), 0.1).unwrap();
        let top = hit_distance(&shape, Vec3::new(0.0, 5.0, 0.0), -Vec3::Y).unwrap();
        let bottom = hit_distance(&shape, Vec3::new(0.0, -5.0, 0.0), Vec3::Y).unwrap();
        let thickness = 10.0 - top - bottom;
        assert!((thickness - 0.2).abs() < 5e-3, "{thickness}");
        assert!((top - bottom).abs() < 1e-3, "centred on the plane");
    }

    #[test]
    fn a_flat_hull_reaches_the_radius_past_its_rim() {
        let shape = grown_hull(&square_plate_points(1.0), 0.1).unwrap();
        let rim_distance = hit_distance(&shape, Vec3::new(5.0, 0.0, 0.0), -Vec3::X).unwrap();
        assert!((5.0 - rim_distance - 1.1).abs() < 5e-3, "{rim_distance}");
    }

    #[test]
    fn a_tilted_flat_hull_is_a_plate_along_its_own_plane() {
        let tilt = glam::Quat::from_rotation_z(0.6);
        let points: Vec<Vec3> = square_plate_points(2.0)
            .into_iter()
            .map(|point| tilt * point)
            .collect();
        let shape = grown_hull(&points, 0.05).unwrap();
        let normal = tilt * Vec3::Y;
        let top = hit_distance(&shape, normal * 5.0, -normal).unwrap();
        assert!((5.0 - top - 0.05).abs() < 5e-3, "{top}");
    }

    #[test]
    fn a_large_flat_plate_is_still_built() {
        let shape = grown_hull(&square_plate_points(12.5), 0.35).unwrap();
        let top = hit_distance(&shape, Vec3::new(3.0, 5.0, -4.0), -Vec3::Y).unwrap();
        assert!((5.0 - top - 0.35).abs() < 1e-2, "{top}");
    }

    #[test]
    fn a_hull_along_a_line_is_the_capsule_around_its_segment() {
        let line = [Vec3::ZERO, Vec3::X, Vec3::X * 2.0, Vec3::X * 3.0];
        let shape = grown_hull(&line, 0.2).unwrap();
        let capsule = shape.as_capsule().unwrap();
        assert_eq!(capsule.radius, 0.2);
        assert!((capsule.segment.length() - 3.0).abs() < 1e-5);
    }

    #[test]
    fn coincident_points_are_a_ball_around_that_point() {
        let centre = Vec3::new(1.0, 2.0, 3.0);
        let shape = grown_hull(&[centre; 5], 0.2).unwrap();
        let aabb = shape.compute_local_aabb();
        assert!((aabb.center() - centre).length() < 1e-5);
        assert!((aabb.half_extents().x - 0.2).abs() < 2e-3);
        let hit = hit_distance(&shape, centre + Vec3::new(0.0, 0.0, 3.0), -Vec3::Z).unwrap();
        assert!((hit - 2.8).abs() < 5e-3, "{hit}");
    }

    #[test]
    fn no_points_have_no_hull() {
        assert!(grown_hull(&[], 0.2).is_none());
    }

    /// Four base corners in a plane and an apex `height` above the middle.
    fn pyramid(height: f32) -> Vec<Vec3> {
        let mut points = square_plate_points(1.0);
        points.push(Vec3::new(0.0, height, 0.0));
        points
    }

    #[test]
    fn a_hull_thinner_than_the_flat_tolerance_is_a_plate_and_a_thicker_one_is_solid() {
        assert!(matches!(
            footprint(&pyramid(FLAT_TOLERANCE * 0.9)),
            Footprint::Plate { .. }
        ));
        assert!(matches!(
            footprint(&pyramid(FLAT_TOLERANCE * 1.1)),
            Footprint::Solid
        ));
    }

    #[test]
    fn a_hull_within_the_line_tolerance_is_a_segment_and_a_wider_one_is_a_plate() {
        let line = |width: f32| {
            vec![
                Vec3::ZERO,
                Vec3::X * 3.0,
                Vec3::new(1.5, width, 0.0),
                Vec3::X,
            ]
        };
        assert!(matches!(
            footprint(&line(FLAT_TOLERANCE * 0.9)),
            Footprint::Segment { .. }
        ));
        assert!(matches!(
            footprint(&line(FLAT_TOLERANCE * 1.1)),
            Footprint::Plate { .. }
        ));
    }

    #[test]
    fn hulls_on_both_sides_of_the_flat_tolerance_build_with_a_radius() {
        assert!(grown_hull(&pyramid(FLAT_TOLERANCE * 1.1), 0.1).is_some());
        assert!(grown_hull(&pyramid(FLAT_TOLERANCE * 0.9), 0.1).is_some());
    }

    #[test]
    fn non_finite_points_or_radius_are_rejected() {
        let mut points = unit_box_points();
        assert!(grown_hull(&points, f32::NAN).is_none());
        assert!(grown_hull(&points, f32::INFINITY).is_none());
        points[3].y = f32::NAN;
        assert!(grown_hull(&points, 0.1).is_none());
        assert!(grown_hull(&points, 0.0).is_none());
    }

    /// Prints the cost of 200,000 ray casts against a bare and a rounded hull; run with
    /// `cargo test --lib -- --ignored --nocapture ray_cost`.
    #[test]
    #[ignore = "timing measurement, prints numbers"]
    fn ray_cost_of_a_rounded_hull_against_a_bare_one() {
        let time = |radius: f32| {
            let shape = grown_hull(&unit_box_points(), radius).unwrap();
            let start = std::time::Instant::now();
            let mut hits = 0_u32;
            for i in 0..200_000_u32 {
                let x = (i % 100) as f32 * 0.01 - 0.5;
                let ray = Ray::new(Vec3::new(x, 0.1, 3.0), -Vec3::Z);
                hits += u32::from(shape.cast_ray(&Pose::IDENTITY, &ray, 10.0, true).is_some());
            }
            println!(
                "radius {radius}: {:?} for 200000 rays, {hits} hits",
                start.elapsed()
            );
        };
        time(0.0);
        time(0.07);
    }
}
