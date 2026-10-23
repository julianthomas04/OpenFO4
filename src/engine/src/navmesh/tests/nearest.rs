use glam::Vec3;

use super::support::{
    MeshBuilder, WIDE_WINDOW, nearest as find_nearest, single_world, stacked_floors,
};
use crate::navmesh::query::nearest::NearestParams;
use crate::navmesh::{QueryFilter, flags};

const F: u32 = 1;

fn nearest_y(window: NearestParams, y: f32, x: f32) -> Option<f32> {
    let world = single_world(stacked_floors(F));
    find_nearest(
        &world,
        Vec3::new(x, y, 1.0),
        window,
        &QueryFilter::default(),
    )
    .map(|r| r.point.y)
}

#[test]
fn prefers_highest_surface_at_or_below_the_point() {
    assert_eq!(nearest_y(WIDE_WINDOW, 4.5, 2.0), Some(4.0));
    assert_eq!(nearest_y(WIDE_WINDOW, 2.0, 2.0), Some(0.0));
}

#[test]
fn step_tolerance_lets_a_slightly_higher_surface_count_as_below() {
    let tolerant = NearestParams {
        step_tolerance: 2.5,
        ..WIDE_WINDOW
    };
    assert_eq!(nearest_y(tolerant, 2.0, 2.0), Some(4.0));
}

#[test]
fn falls_back_to_lowest_surface_above_when_none_below() {
    assert_eq!(nearest_y(WIDE_WINDOW, -1.0, 2.0), Some(0.0));
    assert_eq!(
        nearest_y(WIDE_WINDOW, 1.0, 6.0),
        Some(0.0),
        "ramp is 2 high here, floor is below"
    );
}

#[test]
fn vertical_window_excludes_far_surfaces() {
    let narrow = NearestParams {
        below: 0.5,
        above: 0.5,
        ..WIDE_WINDOW
    };
    assert_eq!(nearest_y(narrow, 2.0, 2.0), None);
    assert_eq!(nearest_y(narrow, 4.2, 2.0), Some(4.0));
}

#[test]
fn point_height_comes_from_the_plane() {
    assert!((nearest_y(WIDE_WINDOW, 2.2, 6.0).unwrap() - 2.0).abs() < 1e-5);
}

#[test]
fn boundary_fallback_snaps_within_radius_only() {
    let world = single_world(MeshBuilder::new(F).flat_grid((0.0, 0.0), 2, 2));
    let snap = NearestParams {
        fallback_radius: 2.0,
        ..WIDE_WINDOW
    };
    let outside = Vec3::new(-1.0, 0.5, 1.0);
    let found = find_nearest(&world, outside, snap, &QueryFilter::default()).unwrap();
    assert!(!found.inside);
    assert!(found.point.distance(Vec3::new(0.0, 0.0, 1.0)) < 1e-5);

    let far = Vec3::new(-5.0, 0.5, 1.0);
    assert!(find_nearest(&world, far, snap, &QueryFilter::default()).is_none());
    assert!(find_nearest(&world, outside, WIDE_WINDOW, &QueryFilter::default()).is_none());
}

#[test]
fn excluded_and_deleted_triangles_are_skipped() {
    let world = single_world(
        MeshBuilder::new(F)
            .flat_grid((0.0, 0.0), 1, 1)
            .flag(0, flags::DELETED)
            .flag(1, flags::NO_LARGE_CREATURES),
    );
    let probe = |filter: &QueryFilter, x: f32, z: f32| {
        find_nearest(&world, Vec3::new(x, 0.0, z), WIDE_WINDOW, filter).map(|r| r.nav_ref.tri())
    };
    let default = QueryFilter::default();
    assert_eq!(probe(&default, 0.9, 0.1), None, "deleted triangle");
    assert_eq!(probe(&default, 0.1, 0.9), Some(1));
    assert_eq!(probe(&QueryFilter::for_large_creature(), 0.1, 0.9), None);
}

#[test]
fn queries_across_several_meshes_pick_the_matching_one() {
    let (a, b) = super::support::linked_pair(0xA, 0xB);
    let world = super::support::world_of(vec![a.source(), b.source()]);
    let at = |x: f32| {
        let r = find_nearest(
            &world,
            Vec3::new(x, 0.0, 0.5),
            WIDE_WINDOW,
            &QueryFilter::default(),
        )
        .unwrap();
        world.mesh(r.nav_ref).unwrap().form_id()
    };
    assert_eq!((at(1.0), at(3.0)), (0xA, 0xB));
}
