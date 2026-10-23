use glam::Vec3;

use super::support::{
    MeshBuilder, grid_with_hole, locate, path_length, serpentine, serpentine_on, single_world,
};
use crate::navmesh::query::funnel::{FunnelError, string_pull};
use crate::navmesh::query::search::{PathSearch, find_path};
use crate::navmesh::{NavRef, NavWorld, QueryFilter};

fn corridor(world: &NavWorld, from: Vec3, to: Vec3) -> (Vec<NavRef>, Vec3, Vec3) {
    let (start, goal) = (locate(world, from), locate(world, to));
    let result = find_path(
        world,
        &mut PathSearch::new(1024),
        start,
        goal,
        &QueryFilter::default(),
    )
    .unwrap();
    (result.refs, start.pos, goal.pos)
}

fn mirrored(p: Vec3, sign: f32) -> Vec3 {
    Vec3::new(p.x, p.y, p.z * sign)
}

/// `sign` is -1 for meshes built through the game-space conversion, which negates Z.
fn pulled(world: &NavWorld, sign: f32, from: Vec3, to: Vec3, radius: f32) -> Vec<Vec3> {
    let (refs, start, goal) = corridor(world, mirrored(from, sign), mirrored(to, sign));
    string_pull(world, &refs, start, goal, radius).unwrap()
}

fn both_windings(build: impl Fn(MeshBuilder) -> MeshBuilder) -> [(NavWorld, f32); 2] {
    // Game-space meshes are built with counter-clockwise winding in game XY and then converted,
    // which mirrors them into clockwise winding on the engine XZ plane.
    let plain = build(MeshBuilder::new(1));
    let converted = build(MeshBuilder::new(1).game_space());
    [(single_world(plain), 1.0), (single_world(converted), -1.0)]
}

fn assert_close(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-3, "{a} != {b}");
}

#[test]
fn open_floor_pulls_to_a_straight_line() {
    for (world, sign) in both_windings(|b| b.flat_grid((0.0, 0.0), 6, 6)) {
        let points = pulled(
            &world,
            sign,
            Vec3::new(0.5, 0.0, 0.5),
            Vec3::new(5.5, 0.0, 5.5),
            0.0,
        );
        assert_close(path_length(&points), Vec3::new(5.0, 0.0, 5.0).length());
    }
}

#[test]
fn single_triangle_corridor_is_start_and_goal() {
    let sign = 1.0;
    let world = single_world(MeshBuilder::new(1).flat_grid((0.0, 0.0), 1, 1));
    let points = pulled(
        &world,
        sign,
        Vec3::new(0.9, 0.0, 0.1),
        Vec3::new(0.6, 0.0, 0.3),
        0.0,
    );
    assert_eq!(
        points,
        vec![Vec3::new(0.9, 0.0, 0.1), Vec3::new(0.6, 0.0, 0.3)]
    );
}

#[test]
fn identical_start_and_goal_collapse_to_one_point() {
    let sign = 1.0;
    let world = single_world(MeshBuilder::new(1).flat_grid((0.0, 0.0), 2, 1));
    let spot = Vec3::new(0.7, 0.0, 0.2);
    assert_eq!(pulled(&world, sign, spot, spot, 0.0), vec![spot]);
}

#[test]
fn bends_around_hole_corners_in_both_windings() {
    let expected = 2.0 * (1.5f32 * 1.5 + 0.5 * 0.5).sqrt() + 1.0;
    for (world, sign) in both_windings(grid_with_hole) {
        let points = pulled(
            &world,
            sign,
            Vec3::new(0.5, 0.0, 2.5),
            Vec3::new(4.5, 0.0, 2.5),
            0.0,
        );
        assert_eq!(points.len(), 4);
        assert_close(path_length(&points), expected);
    }
}

#[test]
fn serpentine_length_matches_the_hand_computed_corridor_in_both_windings() {
    // Inner corners of the S: (6,1) and (6,2) on the way down, (1,3) and (1,4) on the way back.
    let corners = [
        Vec3::new(0.5, 0.0, 0.5),
        Vec3::new(6.0, 0.0, 1.0),
        Vec3::new(6.0, 0.0, 2.0),
        Vec3::new(1.0, 0.0, 3.0),
        Vec3::new(1.0, 0.0, 4.0),
        Vec3::new(6.5, 0.0, 4.5),
    ];
    for (world, sign) in both_windings(serpentine) {
        let points = pulled(&world, sign, corners[0], corners[5], 0.0);
        assert_close(path_length(&points), path_length(&corners));
    }
}

#[test]
fn collinear_portals_do_not_produce_extra_corners() {
    let sign = 1.0;
    let world = single_world(MeshBuilder::new(1).flat_grid((0.0, 0.0), 5, 1));
    let points = pulled(
        &world,
        sign,
        Vec3::new(0.1, 0.0, 0.5),
        Vec3::new(4.9, 0.0, 0.5),
        0.0,
    );
    assert_eq!(points.len(), 2);
}

#[test]
fn radius_larger_than_the_portal_collapses_it_to_its_midpoint() {
    for (world, sign) in both_windings(serpentine) {
        let points = pulled(
            &world,
            sign,
            Vec3::new(0.5, 0.0, 0.5),
            Vec3::new(6.5, 0.0, 4.5),
            50.0,
        );
        assert!(points.iter().all(|p| p.is_finite()));
        assert_eq!(
            points.first(),
            Some(&mirrored(Vec3::new(0.5, 0.0, 0.5), sign))
        );
        assert_eq!(
            points.last(),
            Some(&mirrored(Vec3::new(6.5, 0.0, 4.5), sign))
        );
    }
}

#[test]
fn agent_radius_keeps_corners_off_the_wall_ends() {
    let sign = 1.0;
    let world = single_world(serpentine(MeshBuilder::new(1)));
    let (from, to) = (Vec3::new(0.5, 0.0, 0.5), Vec3::new(6.5, 0.0, 4.5));
    let bare = pulled(&world, sign, from, to, 0.0);
    let padded = pulled(&world, sign, from, to, 0.3);
    assert!(path_length(&padded) > path_length(&bare));
    assert!(
        padded
            .iter()
            .skip(1)
            .take(padded.len() - 2)
            .all(|corner| { bare.iter().all(|wall| wall.distance(*corner) >= 0.3 - 1e-4) })
    );
}

#[test]
fn corner_heights_follow_the_walked_surface() {
    let world = single_world(serpentine_on(MeshBuilder::new(1), |x, _| x * 0.5));
    let points = pulled(
        &world,
        1.0,
        Vec3::new(0.5, 0.25, 0.5),
        Vec3::new(6.5, 3.25, 4.5),
        0.0,
    );
    assert!(points.len() >= 4);
    for point in &points[1..points.len() - 1] {
        assert_close(point.y, point.x * 0.5);
    }
}

#[test]
fn rejects_unusable_corridors() {
    let mut world = single_world(MeshBuilder::new(1).flat_grid((0.0, 0.0), 3, 1));
    let (a, b) = (world.nav_ref(1, 0).unwrap(), world.nav_ref(1, 5).unwrap());
    let pull =
        |world: &NavWorld, refs: &[NavRef]| string_pull(world, refs, Vec3::ZERO, Vec3::X, 0.0);
    assert_eq!(pull(&world, &[]), Err(FunnelError::EmptyCorridor));
    assert_eq!(
        pull(&world, &[a, b]),
        Err(FunnelError::NotConnected { from: a, to: b })
    );
    world.remove(1).unwrap();
    assert_eq!(pull(&world, &[a]), Err(FunnelError::StaleRef(a)));
}

#[test]
fn zero_area_triangle_in_the_corridor_is_an_error() {
    let mut builder = MeshBuilder::new(1).flat_grid((0.0, 0.0), 1, 1);
    let sliver = builder.triangle(
        Vec3::new(1.0, 0.0, 1.0),
        Vec3::new(2.0, 0.0, 1.0),
        Vec3::new(3.0, 0.0, 1.0),
    );
    let mut source = builder.source();
    source.neighbours[1][1] = sliver as i16;
    source.neighbours[usize::from(sliver)][0] = 1;
    let world = super::support::world_of(vec![source]);
    let (a, b) = (
        world.nav_ref(1, sliver).unwrap(),
        world.nav_ref(1, 1).unwrap(),
    );
    assert_eq!(
        string_pull(&world, &[a, b], Vec3::ZERO, Vec3::X, 0.0),
        Err(FunnelError::DegenerateTriangle(a))
    );
}
