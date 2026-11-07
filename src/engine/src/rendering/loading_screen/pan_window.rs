//! How far the viewer may move a loading screen's model across the screen with the pan keys.
//!
//! The game's rule is not documented. A report from modders says that shrinking a model's
//! bounding sphere in its NIF stops the pan keys from moving it, so the window is inferred to be
//! derived from that sphere: the model may be moved up to the sphere's radius, in each direction
//! of the camera's plane, away from where its placement puts it.

/// The radius of a sphere that holds every one of `points`, centred on the middle of their
/// bounding box. Points with a coordinate that is not a number are ignored; no points make a
/// sphere of no size.
pub fn bounding_sphere_radius(points: impl IntoIterator<Item = [f32; 3]>) -> f32 {
    let points: Vec<[f32; 3]> = points
        .into_iter()
        .filter(|point| point.iter().all(|coordinate| coordinate.is_finite()))
        .collect();
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for point in &points {
        for axis in 0..3 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    let centre: [f32; 3] = std::array::from_fn(|axis| f32::midpoint(min[axis], max[axis]));
    points
        .iter()
        .map(|point| {
            point
                .iter()
                .zip(centre)
                .map(|(coordinate, middle)| (coordinate - middle).powi(2))
                .sum::<f32>()
                .sqrt()
        })
        .fold(0.0, f32::max)
}

/// The distance the model may be moved from its placement along each of the camera plane's axes:
/// the radius of its bounding sphere at the placement's `scale`.
pub fn pan_limit(sphere_radius: f32, scale: f32) -> f32 {
    let limit = sphere_radius * scale.abs();
    if limit.is_finite() { limit } else { 0.0 }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the expected values are exact for these inputs"
)]
mod tests {
    use super::*;

    #[test]
    fn the_sphere_is_centred_on_the_middle_of_the_box_and_reaches_the_farthest_point() {
        let radius = bounding_sphere_radius([[-1.0, 0.0, 0.0], [3.0, 0.0, 0.0]]);
        assert_eq!(radius, 2.0);
        let radius = bounding_sphere_radius([[0.0; 3], [2.0, 2.0, 2.0], [1.0, 1.0, 1.0]]);
        assert!((radius - 3.0_f32.sqrt()).abs() < 1e-6, "{radius}");
    }

    #[test]
    fn a_single_point_or_no_point_has_no_size() {
        assert_eq!(bounding_sphere_radius([[4.0, 5.0, 6.0]]), 0.0);
        assert_eq!(bounding_sphere_radius([]), 0.0);
    }

    #[test]
    fn points_that_are_not_numbers_are_ignored() {
        let radius = bounding_sphere_radius([[0.0; 3], [f32::NAN, 0.0, 0.0], [2.0, 0.0, 0.0]]);
        assert_eq!(radius, 1.0);
        assert_eq!(bounding_sphere_radius([[f32::INFINITY, 0.0, 0.0]]), 0.0);
    }

    #[test]
    fn the_limit_is_the_radius_at_the_scale_of_the_placement() {
        assert_eq!(pan_limit(2.0, 1.5), 3.0);
        assert_eq!(
            pan_limit(2.0, -1.5),
            3.0,
            "a mirrored placement still has a size"
        );
        assert_eq!(pan_limit(0.0, 2.0), 0.0);
    }

    #[test]
    fn a_limit_that_is_not_a_number_is_none() {
        assert_eq!(pan_limit(f32::NAN, 1.0), 0.0);
        assert_eq!(pan_limit(1.0, f32::INFINITY), 0.0);
    }
}
