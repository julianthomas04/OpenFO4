//! Mass of a collider whose solid includes its round hulls' radius.
//!
//! Rapier computes the mass properties of a rounded hull from the inner hull alone, so a plate
//! stored as a thin hull plus a radius would weigh nothing and a small hull would lose the
//! rounding's share of its volume. The solid used here instead is a polytope approximating the
//! grown hull: the hull of every inner point pushed out by the radius along the 26 directions of
//! a cube's corners, edges and faces. Back-solving the density from that volume, and giving the
//! collider the properties of that solid, lands the body's data mass exactly; the approximation
//! only shapes the inertia and the centre of mass. Shapes without a round hull are untouched.

use glam::Vec3;
use rapier3d::prelude::*;

fn growth_directions() -> impl Iterator<Item = Vec3> {
    (-1..=1).flat_map(|x| {
        (-1..=1).flat_map(move |y| {
            (-1..=1).filter_map(move |z| {
                let direction = Vec3::new(x as f32, y as f32, z as f32);
                (direction != Vec3::ZERO).then(|| direction.normalize())
            })
        })
    })
}

/// The polytope approximating `shape` with its round hulls grown, or `None` when the shape has no
/// round hull (or the polytope cannot be built).
fn grown_solid(shape: &SharedShape) -> Option<SharedShape> {
    if let Some(round) = shape.as_round_convex_polyhedron() {
        let radius = round.border_radius;
        let points: Vec<Vec3> = round
            .inner_shape
            .points()
            .iter()
            .flat_map(|point| growth_directions().map(move |direction| *point + direction * radius))
            .collect();
        return SharedShape::convex_hull(&points);
    }
    let leaves = shape.as_compound()?.shapes();
    let grown: Vec<(Pose, Option<SharedShape>)> = leaves
        .iter()
        .map(|(pose, leaf)| (*pose, grown_solid(leaf)))
        .collect();
    if grown.iter().all(|(_, leaf)| leaf.is_none()) {
        return None;
    }
    let solid = leaves
        .iter()
        .zip(grown)
        .map(|((_, leaf), (pose, grown))| (pose, grown.unwrap_or_else(|| leaf.clone())))
        .collect();
    Some(SharedShape::compound(solid))
}

/// The volume the density multiplies: the grown solid's for a shape with round hulls, else the
/// shape's own.
pub(super) fn solid_volume(shape: &SharedShape) -> Real {
    grown_solid(shape)
        .as_ref()
        .unwrap_or(shape)
        .mass_properties(1.0)
        .mass()
}

/// A collider of `shape` weighing `density` times [`solid_volume`].
pub(super) fn solid_collider(shape: &SharedShape, density: Real) -> ColliderBuilder {
    let builder = ColliderBuilder::new(shape.clone());
    match grown_solid(shape) {
        Some(grown) => builder.mass_properties(grown.mass_properties(density)),
        None => builder.density(density),
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::PI;

    use super::*;
    use crate::sim::rounded_hull::grown_hull;

    fn box_points(half: Vec3) -> Vec<Vec3> {
        (0..8)
            .map(|corner| {
                Vec3::new(
                    if corner & 1 == 0 { -half.x } else { half.x },
                    if corner & 2 == 0 { -half.y } else { half.y },
                    if corner & 4 == 0 { -half.z } else { half.z },
                )
            })
            .collect()
    }

    fn plate_points(half: f32) -> Vec<Vec3> {
        vec![
            Vec3::new(-half, 0.0, -half),
            Vec3::new(half, 0.0, -half),
            Vec3::new(half, 0.0, half),
            Vec3::new(-half, 0.0, half),
        ]
    }

    #[test]
    fn shapes_without_a_round_hull_keep_their_own_volume_and_density() {
        let cuboid = SharedShape::cuboid(0.5, 0.5, 0.5);
        assert!((solid_volume(&cuboid) - 1.0).abs() < 1e-6);
        let collider = solid_collider(&cuboid, 3.0).build();
        assert!((collider.mass() - 3.0).abs() < 1e-5);
    }

    #[test]
    fn a_bare_hull_keeps_its_own_volume() {
        let hull = grown_hull(&box_points(Vec3::splat(0.5)), 0.0).unwrap();
        assert!((solid_volume(&hull) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_round_hull_weighs_its_grown_volume() {
        let radius = 0.1;
        let hull = grown_hull(&box_points(Vec3::splat(0.5)), radius).unwrap();
        let steiner = 1.0
            + 6.0 * radius
            + 3.0 * PI * radius * radius
            + 4.0 / 3.0 * PI * radius * radius * radius;
        let volume = solid_volume(&hull);
        assert!(
            (volume - steiner).abs() / steiner < 0.02,
            "{volume} {steiner}"
        );
        assert!(volume > 1.0);
    }

    #[test]
    fn a_plate_has_the_volume_of_its_thickness() {
        let plate = grown_hull(&plate_points(1.0), 0.1).unwrap();
        let volume = solid_volume(&plate);
        let expected = 4.0 * 0.2 + 4.0 * 2.0 * PI * 0.01 / 2.0 + 4.0 / 3.0 * PI * 0.001;
        assert!(
            (volume - expected).abs() / expected < 0.05,
            "{volume} {expected}"
        );
    }

    #[test]
    fn the_collider_of_a_round_hull_weighs_density_times_the_grown_volume() {
        let hull = grown_hull(&box_points(Vec3::splat(0.5)), 0.1).unwrap();
        let collider = solid_collider(&hull, 2.0).build();
        assert!((collider.mass() - 2.0 * solid_volume(&hull)).abs() < 1e-4);
        assert!(collider.mass_properties().principal_inertia().min_element() > 0.0);
    }

    #[test]
    fn round_leaves_of_a_compound_count_with_their_radius_and_pose() {
        let leaf = |radius: f32, x: f32| {
            (
                Pose::from_translation(Vec3::new(x, 0.0, 0.0)),
                grown_hull(&box_points(Vec3::splat(0.5)), radius).unwrap(),
            )
        };
        let compound = SharedShape::compound(vec![leaf(0.1, 0.0), leaf(0.0, 3.0)]);
        let alone = |radius: f32| solid_volume(&leaf(radius, 0.0).1);
        let expected = alone(0.1) + alone(0.0);
        assert!((solid_volume(&compound) - expected).abs() < 1e-4);
    }

    #[test]
    fn a_compound_without_round_hulls_is_left_alone() {
        let compound = SharedShape::compound(vec![
            (Pose::IDENTITY, SharedShape::ball(1.0)),
            (
                Pose::from_translation(Vec3::X * 3.0),
                SharedShape::ball(1.0),
            ),
        ]);
        assert!(grown_solid(&compound).is_none());
    }

    #[test]
    fn a_thin_plate_still_lands_the_requested_mass() {
        use crate::sim::bake::CachedShape;
        use crate::sim::dynamic_body::density_for_mass;

        let plate = CachedShape {
            shape: grown_hull(&plate_points(1.0), 0.1).unwrap(),
            local_pose: Pose::IDENTITY,
            joint: None,
            material: 0,
        };
        let shapes = [plate];
        let density = density_for_mass(&shapes, Some(2.0));
        let collider = solid_collider(&shapes[0].shape, density).build();
        assert!((collider.mass() - 2.0).abs() < 1e-3, "{}", collider.mass());
    }
}
