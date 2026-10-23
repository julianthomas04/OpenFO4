use glam::Vec3;

use super::support::{
    MeshBuilder, grid_with_hole, linked_pair, locate, serpentine, single_world, stacked_floors,
    world_of,
};
use crate::navmesh::query::search::{
    Completeness, PathResult, PathSearch, SearchError, SearchStatus, find_path,
};
use crate::navmesh::{LinkType, NavWorld, QueryFilter, flags};

fn path(
    world: &NavWorld,
    from: Vec3,
    to: Vec3,
    filter: &QueryFilter,
    max_nodes: usize,
) -> Result<PathResult, SearchError> {
    let mut search = PathSearch::new(max_nodes);
    find_path(
        world,
        &mut search,
        locate(world, from),
        locate(world, to),
        filter,
    )
}

fn open(world: &NavWorld, from: Vec3, to: Vec3) -> PathResult {
    path(world, from, to, &QueryFilter::default(), 1024).unwrap()
}

#[test]
fn same_triangle_is_a_trivial_complete_path() {
    let world = single_world(MeshBuilder::new(1).flat_grid((0.0, 0.0), 2, 2));
    let result = open(&world, Vec3::new(0.2, 0.0, 0.1), Vec3::new(0.7, 0.0, 0.2));
    assert_eq!(result.refs.len(), 1);
    assert_eq!(result.completeness, Completeness::Complete);
}

#[test]
fn open_grid_path_is_connected_and_complete() {
    let world = single_world(MeshBuilder::new(1).flat_grid((0.0, 0.0), 6, 6));
    let result = open(&world, Vec3::new(0.5, 0.0, 0.5), Vec3::new(5.5, 0.0, 5.5));
    assert_eq!(result.completeness, Completeness::Complete);
    for pair in result.refs.windows(2) {
        assert!(world.link_between(pair[0], pair[1]).is_some());
    }
}

#[test]
fn path_goes_around_a_hole() {
    let world = single_world(grid_with_hole(MeshBuilder::new(1)));
    let result = open(&world, Vec3::new(0.5, 0.0, 2.5), Vec3::new(4.5, 0.0, 2.5));
    assert_eq!(result.completeness, Completeness::Complete);
    assert!(
        result.refs.len() > 8,
        "a straight crossing would need only a few triangles"
    );
}

#[test]
fn disconnected_goal_yields_partial_path_to_closest_triangle() {
    let world = world_of(vec![
        MeshBuilder::new(1).flat_grid((0.0, 0.0), 3, 1).source(),
        MeshBuilder::new(2).flat_grid((10.0, 0.0), 1, 1).source(),
    ]);
    let result = open(&world, Vec3::new(0.5, 0.0, 0.5), Vec3::new(10.5, 0.0, 0.5));
    assert_eq!(result.completeness, Completeness::Partial);
    let last = *result.refs.last().unwrap();
    assert_eq!(world.mesh(last).unwrap().form_id(), 1);
    assert!(last.tri() >= 4, "partial path ends in the rightmost cell");
}

#[test]
fn excluded_flags_block_travel_but_not_for_other_filters() {
    let mut builder = MeshBuilder::new(1).flat_grid((0.0, 0.0), 4, 1);
    for tri in 4..6 {
        builder = builder.flag(tri, flags::NO_LARGE_CREATURES);
    }
    let world = single_world(builder);
    let (from, to) = (Vec3::new(0.5, 0.0, 0.5), Vec3::new(3.5, 0.0, 0.5));
    let normal = path(&world, from, to, &QueryFilter::default(), 64).unwrap();
    assert_eq!(normal.completeness, Completeness::Complete);
    let large = path(&world, from, to, &QueryFilter::for_large_creature(), 64).unwrap();
    assert_eq!(large.completeness, Completeness::Partial);
}

#[test]
fn deleted_triangles_are_never_entered() {
    let world = single_world(
        MeshBuilder::new(1)
            .flat_grid((0.0, 0.0), 3, 1)
            .flag(2, flags::DELETED),
    );
    let result = open(&world, Vec3::new(0.2, 0.0, 0.1), Vec3::new(2.8, 0.0, 0.9));
    assert_eq!(result.completeness, Completeness::Partial);
    assert!(result.refs.iter().all(|r| r.tri() != 2));
}

#[test]
fn closed_doors_block_and_open_doors_pass() {
    let world = single_world(
        MeshBuilder::new(1)
            .flat_grid((0.0, 0.0), 4, 1)
            .door(4, 0xD0)
            .door(5, 0xD0),
    );
    let (from, to) = (Vec3::new(0.5, 0.0, 0.5), Vec3::new(3.5, 0.0, 0.5));
    let blocked = QueryFilter::default().blocking_door(0xD0);
    assert_eq!(
        path(&world, from, to, &blocked, 64).unwrap().completeness,
        Completeness::Partial
    );
    let other_door = QueryFilter::default().blocking_door(0xD1);
    assert_eq!(
        path(&world, from, to, &other_door, 64)
            .unwrap()
            .completeness,
        Completeness::Complete
    );
}

#[test]
fn costly_flags_steer_the_route() {
    let mut builder = MeshBuilder::new(1).flat_grid((0.0, 0.0), 6, 3);
    for col in 1..5 {
        let first = ((6 + col) * 2) as u16;
        builder = builder
            .flag(first, flags::WATER)
            .flag(first + 1, flags::WATER);
    }
    let world = single_world(builder);
    let (from, to) = (Vec3::new(0.5, 0.0, 1.5), Vec3::new(5.5, 0.0, 1.5));
    let through_water = |filter: &QueryFilter| {
        let result = path(&world, from, to, filter, 256).unwrap();
        result
            .refs
            .iter()
            .any(|r| world.mesh(*r).unwrap().flags(r.tri()).unwrap() & flags::WATER != 0)
    };
    assert!(through_water(&QueryFilter::default()));
    let avoid = QueryFilter::default()
        .with_multiplier(flags::WATER, 50.0)
        .unwrap();
    assert!(!through_water(&avoid));
}

#[test]
fn node_cap_returns_a_node_limit_path_toward_the_goal() {
    let world = single_world(MeshBuilder::new(1).flat_grid((0.0, 0.0), 20, 1));
    let result = path(
        &world,
        Vec3::new(0.2, 0.0, 0.2),
        Vec3::new(19.8, 0.0, 0.8),
        &QueryFilter::default(),
        6,
    )
    .unwrap();
    assert_eq!(result.completeness, Completeness::NodeLimit);
    assert!(result.refs.len() > 1);
}

#[test]
fn sliced_search_matches_the_convenience_call() {
    let world = single_world(serpentine(MeshBuilder::new(1)));
    let (from, to) = (
        locate(&world, Vec3::new(0.5, 0.0, 0.5)),
        locate(&world, Vec3::new(0.5, 0.0, 4.5)),
    );
    let filter = QueryFilter::default();
    let expected = find_path(&world, &mut PathSearch::new(512), from, to, &filter).unwrap();

    let mut search = PathSearch::new(512);
    search.init(&world, from, to, &filter).unwrap();
    let mut slices = 0;
    while search.update(&world, 3) == SearchStatus::InProgress {
        slices += 1;
    }
    assert!(slices > 1);
    assert_eq!(search.finalize(&world).unwrap(), expected);
    assert_eq!(expected.completeness, Completeness::Complete);
}

#[test]
fn search_object_is_reusable() {
    let world = single_world(serpentine(MeshBuilder::new(1)));
    let mut search = PathSearch::new(512);
    let a = locate(&world, Vec3::new(0.5, 0.0, 0.5));
    let b = locate(&world, Vec3::new(0.5, 0.0, 4.5));
    let first = find_path(&world, &mut search, a, b, &QueryFilter::default()).unwrap();
    let second = find_path(&world, &mut search, a, b, &QueryFilter::default()).unwrap();
    assert_eq!(first, second);
}

#[test]
fn unloading_during_a_sliced_search_leaves_no_stale_refs_in_the_result() {
    let mut world = single_world(serpentine(MeshBuilder::new(1)));
    let (from, to) = (
        locate(&world, Vec3::new(0.5, 0.0, 0.5)),
        locate(&world, Vec3::new(0.5, 0.0, 4.5)),
    );
    let mut search = PathSearch::new(512);
    search
        .init(&world, from, to, &QueryFilter::default())
        .unwrap();
    assert_eq!(search.update(&world, 2), SearchStatus::InProgress);
    world.remove(1).unwrap();
    assert_eq!(
        search.run(&world),
        SearchStatus::Done(Completeness::Partial),
        "stale triangles are walls, so the search just runs dry"
    );
    assert_eq!(search.finalize(&world), Err(SearchError::PathInvalidated));
}

#[test]
fn partial_unload_trims_the_finalized_path_to_the_valid_prefix() {
    let (a, b) = linked_pair(0xA, 0xB);
    let mut world = world_of(vec![a.source(), b.source()]);
    let mut search = PathSearch::new(64);
    let from = locate(&world, Vec3::new(0.5, 0.0, 0.5));
    let to = locate(&world, Vec3::new(3.5, 0.0, 0.5));
    search
        .init(&world, from, to, &QueryFilter::default())
        .unwrap();
    assert_eq!(
        search.run(&world),
        SearchStatus::Done(Completeness::Complete)
    );
    world.remove(0xB).unwrap();
    let trimmed = search.finalize(&world).unwrap();
    assert_eq!(trimmed.completeness, Completeness::Partial);
    assert!(trimmed.refs.iter().all(|r| world.is_valid(*r)));
}

#[test]
fn invalid_endpoints_are_rejected() {
    let world = single_world(
        MeshBuilder::new(1)
            .flat_grid((0.0, 0.0), 2, 1)
            .flag(0, flags::DELETED),
    );
    let good = locate(&world, Vec3::new(1.5, 0.0, 0.5));
    let deleted = crate::navmesh::query::NavPoint {
        nav_ref: world.nav_ref(1, 0).unwrap(),
        pos: Vec3::ZERO,
    };
    let mut search = PathSearch::new(8);
    let filter = QueryFilter::default();
    assert_eq!(
        search.init(&world, deleted, good, &filter),
        Err(SearchError::InvalidStart)
    );
    assert_eq!(
        search.init(&world, good, deleted, &filter),
        Err(SearchError::InvalidGoal)
    );
    assert_eq!(search.finalize(&world), Err(SearchError::NotInitialised));
}

#[test]
fn paths_cross_meshes_through_links_and_one_way_links_are_directional() {
    let a = MeshBuilder::new(0xA)
        .flat_grid((0.0, 0.0), 2, 1)
        .link((2, 1), 0, 0xB, (1, 2));
    let b = MeshBuilder::new(0xB).flat_grid((2.0, 0.0), 2, 1);
    let world = world_of(vec![a.source(), b.source()]);
    let (left, right) = (Vec3::new(0.5, 0.0, 0.5), Vec3::new(3.5, 0.0, 0.5));
    let forward = open(&world, left, right);
    assert_eq!(forward.completeness, Completeness::Complete);
    assert!(
        forward
            .refs
            .iter()
            .any(|r| world.mesh(*r).unwrap().form_id() == 0xB)
    );
    assert_eq!(
        open(&world, right, left).completeness,
        Completeness::Partial
    );
}

#[test]
fn excluded_link_types_block_crossing() {
    let (a, b) = linked_pair(0xA, 0xB);
    let world = world_of(vec![a.source(), b.source()]);
    let (left, right) = (Vec3::new(0.5, 0.0, 0.5), Vec3::new(3.5, 0.0, 0.5));
    let filter = QueryFilter::default().excluding_link_type(LinkType::Portal);
    assert_eq!(
        path(&world, left, right, &filter, 64).unwrap().completeness,
        Completeness::Partial
    );
}

#[test]
fn ramp_connects_stacked_floors() {
    let world = single_world(stacked_floors(1));
    let result = open(&world, Vec3::new(0.5, 0.0, 0.5), Vec3::new(0.5, 4.0, 0.5));
    assert_eq!(result.completeness, Completeness::Complete);
    let heights: Vec<f32> = result
        .refs
        .iter()
        .map(|r| {
            world
                .mesh(*r)
                .unwrap()
                .triangle_vertices(r.tri())
                .unwrap()
                .iter()
                .map(|v| v.y)
                .sum::<f32>()
                / 3.0
        })
        .collect();
    assert!(
        heights.iter().any(|h| (h - 2.0).abs() < 1.5),
        "path climbs the ramp"
    );
}
