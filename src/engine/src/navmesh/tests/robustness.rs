//! Malformed or hostile input to every public entry point: out-of-range indices, non-finite
//! numbers, one-way adjacency, triangles without area, stale references after reloads.

use glam::{Vec2, Vec3};

use super::support::{
    MeshBuilder, WIDE_WINDOW, linked_pair, locate, nav_ref, nearest, serpentine, single_world,
    world_of,
};
use crate::navmesh::geometry::exit_through_triangle;
use crate::navmesh::query::NavPoint;
use crate::navmesh::query::funnel::{FunnelError, string_pull};
use crate::navmesh::query::nearest::{NearestError, NearestParams, find_nearest};
use crate::navmesh::query::raycast::{DEFAULT_MAX_STEPS, RaycastError, raycast};
use crate::navmesh::query::search::{
    Completeness, PathSearch, SearchError, SearchStatus, find_path,
};
use crate::navmesh::{EdgeLinkSource, NavMesh, NavMeshError, NavMeshSource, NavWorld, QueryFilter};

fn rejects(edit: impl FnOnce(&mut NavMeshSource)) -> NavMeshError {
    let mut source = MeshBuilder::new(1).flat_grid((0.0, 0.0), 2, 1).source();
    edit(&mut source);
    NavMesh::new(source).expect_err("source must be rejected")
}

fn link(triangle: u16, edge: u8, target_edge: u8) -> EdgeLinkSource {
    EdgeLinkSource {
        triangle,
        edge,
        link_type: 0,
        target_form_id: 2,
        target_triangle: 0,
        target_edge,
    }
}

// ---- mesh accessors and validation ------------------------------------------------------------

#[test]
fn out_of_range_arguments_give_none_instead_of_panicking() {
    let mesh = MeshBuilder::new(1).flat_grid((0.0, 0.0), 1, 1).mesh();
    assert!(mesh.flags(1).is_some());
    assert_eq!(mesh.flags(2), None);
    assert_eq!(mesh.triangle_vertices(2), None);
    assert!(mesh.triangle_vertices(1).is_some());
    assert_eq!(mesh.edge_vertices(0, 3), None);
    assert_eq!(mesh.edge_vertices(9, 0), None);
    assert!(mesh.edge_vertices(0, 2).is_some());
    assert_eq!(mesh.neighbour(0, 3), None);
    assert_eq!(mesh.neighbour(9, 0), None);
    assert_eq!(mesh.height_at(9, 0.0, 0.0), None);
    assert_eq!(mesh.door_refs(9), [] as [u32; 0]);
    assert!(!mesh.is_walkable(9));
    assert!(mesh.is_walkable(0));
}

#[test]
fn a_triangle_listing_itself_as_neighbour_is_rejected() {
    assert_eq!(
        rejects(|s| s.neighbours[1][0] = 1),
        NavMeshError::SelfNeighbour {
            triangle: 1,
            edge: 0
        }
    );
}

#[test]
fn an_edge_with_both_a_neighbour_and_an_external_link_is_rejected() {
    assert_eq!(
        rejects(|s| {
            assert_ne!(s.neighbours[0][2], -1, "fixture edge has a neighbour");
            s.edge_links.push(link(0, 2, 0));
        }),
        NavMeshError::EdgeHasNeighbourAndLink {
            triangle: 0,
            edge: 2
        }
    );
}

#[test]
fn a_link_edge_without_a_neighbour_is_accepted() {
    let mut source = MeshBuilder::new(1).flat_grid((0.0, 0.0), 2, 1).source();
    assert_eq!(source.neighbours[0][0], -1);
    source.edge_links.push(link(0, 0, 1));
    assert!(NavMesh::new(source).is_ok());
}

#[test]
fn a_link_targeting_an_edge_beyond_two_is_rejected() {
    assert!(matches!(
        rejects(|s| s.edge_links.push(link(0, 0, 3))),
        NavMeshError::EdgeLinkTargetEdgeOutOfRange { edge: 3, .. }
    ));
}

#[test]
fn one_way_adjacency_is_tolerated_counted_and_directional() {
    let mut source = MeshBuilder::new(1).flat_grid((0.0, 0.0), 1, 1).source();
    assert_eq!((source.neighbours[0][2], source.neighbours[1][0]), (1, 0));
    source.neighbours[1][0] = -1;
    let world = world_of(vec![source]);
    assert_eq!(world.stats().asymmetric_neighbours, 1);
    let mesh = world.mesh(nav_ref(&world, 1, 0)).unwrap();
    assert_eq!(mesh.asymmetric_neighbour_count(), 1);

    let (zero, one) = (nav_ref(&world, 1, 0), nav_ref(&world, 1, 1));
    let filter = QueryFilter::default();
    let (a, b) = (
        locate(&world, Vec3::new(0.8, 0.0, 0.1)),
        locate(&world, Vec3::new(0.1, 0.0, 0.8)),
    );
    assert_eq!((a.nav_ref, b.nav_ref), (zero, one));
    let forward = find_path(&world, &mut PathSearch::new(16), a, b, &filter).unwrap();
    assert_eq!(forward.completeness, Completeness::Complete);
    let backward = find_path(&world, &mut PathSearch::new(16), b, a, &filter).unwrap();
    assert_eq!(backward.completeness, Completeness::Partial);

    assert!(string_pull(&world, &[zero, one], a.pos, b.pos, 0.0).is_ok());
    assert_eq!(
        string_pull(&world, &[one, zero], b.pos, a.pos, 0.0),
        Err(FunnelError::NotConnected {
            from: one,
            to: zero
        })
    );
}

#[test]
fn symmetric_meshes_report_no_asymmetry() {
    let mesh = MeshBuilder::new(1).flat_grid((0.0, 0.0), 3, 2).mesh();
    assert_eq!(mesh.asymmetric_neighbour_count(), 0);
}

#[test]
fn several_doors_on_one_triangle_are_accepted_in_authored_order() {
    let mesh = MeshBuilder::new(1)
        .flat_grid((0.0, 0.0), 1, 1)
        .door(0, 0xD2)
        .door(0, 0xD1)
        .mesh();
    assert_eq!(mesh.door_refs(0), [0xD2, 0xD1]);
}

fn wall_triangle_mesh() -> (NavMesh, u16) {
    let mut builder = MeshBuilder::new(1).flat_grid((0.0, 0.0), 1, 1);
    let wall = builder.triangle(
        Vec3::new(2.0, 0.0, 0.0),
        Vec3::new(2.0, 0.0, 1.0),
        Vec3::new(2.0, 1.0, 1.0),
    );
    (builder.mesh(), wall)
}

#[test]
fn triangles_without_xz_area_are_counted_and_not_walkable() {
    let (mesh, wall) = wall_triangle_mesh();
    assert_eq!(mesh.zero_area_triangle_count(), 1);
    assert!(!mesh.is_walkable(wall));
    assert!(mesh.is_walkable(0) && mesh.is_walkable(1));

    let mut world = NavWorld::new();
    let report = world.add(mesh).unwrap();
    assert_eq!(report.zero_area_triangles, 1);
    assert_eq!(world.stats().zero_area_triangles, 1);
}

#[test]
fn plane_heights_stay_exact_far_from_the_origin() {
    let far = 100_000.0;
    let mut builder = MeshBuilder::new(1);
    builder.triangle(
        Vec3::new(far, 0.0, far),
        Vec3::new(far + 1.0, 0.5, far),
        Vec3::new(far, 0.0, far + 1.0),
    );
    let mesh = builder.mesh();
    let height = mesh.height_at(0, far + 0.5, far + 0.25).unwrap();
    assert!((height - 0.25).abs() < 1e-4, "{height}");
}

// ---- search -----------------------------------------------------------------------------------

#[test]
fn non_finite_start_or_goal_positions_are_rejected_and_the_search_stays_usable() {
    let world = single_world(serpentine(MeshBuilder::new(1)));
    let good = locate(&world, Vec3::new(0.5, 0.0, 0.5));
    let far = locate(&world, Vec3::new(0.5, 0.0, 4.5));
    let filter = QueryFilter::default();
    let mut search = PathSearch::new(512);
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let broken = NavPoint {
            pos: Vec3::new(bad, 0.0, 0.0),
            ..good
        };
        assert_eq!(
            search.init(&world, broken, far, &filter),
            Err(SearchError::NonFinitePosition)
        );
        assert_eq!(
            search.init(&world, good, broken, &filter),
            Err(SearchError::NonFinitePosition)
        );
        assert_eq!(search.finalize(&world), Err(SearchError::NotInitialised));
    }
    let result = find_path(&world, &mut search, good, far, &filter).unwrap();
    assert_eq!(result.completeness, Completeness::Complete);
}

#[test]
fn a_target_reloaded_during_a_sliced_search_never_leaves_stale_refs() {
    let (a, b) = linked_pair(0xA, 0xB);
    let mut world = world_of(vec![a.source(), b.source()]);
    let from = locate(&world, Vec3::new(0.5, 0.0, 0.2));
    let to = locate(&world, Vec3::new(3.7, 0.0, 0.3));
    let filter = QueryFilter::default();

    let mut probe = PathSearch::new(64);
    probe.init(&world, from, to, &filter).unwrap();
    let mut steps = 0;
    while probe.update(&world, 1) == SearchStatus::InProgress {
        steps += 1;
    }
    assert!(steps >= 3, "the route needs several expansions");

    let mut search = PathSearch::new(64);
    search.init(&world, from, to, &filter).unwrap();
    search.update(&world, steps);
    let old_salt = nav_ref(&world, 0xB, 0).salt();
    world.remove(0xB).unwrap();
    let (_, b_again) = linked_pair(0xA, 0xB);
    world.add(b_again.mesh()).unwrap();
    assert_ne!(nav_ref(&world, 0xB, 0).salt(), old_salt);

    assert_eq!(
        search.run(&world),
        SearchStatus::Done(Completeness::Partial),
        "the old B triangles are walls and A's explored side never re-expands"
    );
    let trimmed = search.finalize(&world).unwrap();
    assert_eq!(trimmed.completeness, Completeness::Partial);
    assert!(trimmed.refs.iter().all(|r| world.is_valid(*r)));
}

#[test]
fn a_goal_reloaded_under_a_new_salt_is_unreachable_for_the_old_search_but_found_by_a_new_one() {
    let (a, b) = linked_pair(0xA, 0xB);
    let mut world = world_of(vec![a.source(), b.source()]);
    let from = locate(&world, Vec3::new(0.5, 0.0, 0.2));
    let to = locate(&world, Vec3::new(3.7, 0.0, 0.3));
    let mut search = PathSearch::new(64);
    search
        .init(&world, from, to, &QueryFilter::default())
        .unwrap();
    assert_eq!(search.update(&world, 1), SearchStatus::InProgress);

    world.remove(0xB).unwrap();
    let (_, b_again) = linked_pair(0xA, 0xB);
    world.add(b_again.mesh()).unwrap();
    let new_goal = locate(&world, Vec3::new(3.7, 0.0, 0.3));
    assert_ne!(
        new_goal.nav_ref, to.nav_ref,
        "same slot and triangle, new salt"
    );

    // The goal reference given to `init` is now stale: the search only ever finds new triangles,
    // so it cannot reach it, and nothing stale may leak into the result.
    assert_eq!(
        search.run(&world),
        SearchStatus::Done(Completeness::Partial)
    );
    let result = search.finalize(&world).unwrap();
    assert!(result.refs.iter().all(|r| world.is_valid(*r)));
    assert_eq!(result.completeness, Completeness::Partial);

    let fresh = find_path(
        &world,
        &mut PathSearch::new(64),
        from,
        new_goal,
        &QueryFilter::default(),
    )
    .unwrap();
    assert_eq!(fresh.completeness, Completeness::Complete);
}

#[test]
fn finalize_into_reuses_the_buffer_and_matches_finalize() {
    let world = single_world(serpentine(MeshBuilder::new(1)));
    let (from, to) = (
        locate(&world, Vec3::new(0.5, 0.0, 0.5)),
        locate(&world, Vec3::new(0.5, 0.0, 4.5)),
    );
    let mut search = PathSearch::new(512);
    let mut refs = vec![nav_ref(&world, 1, 0); 3];
    assert_eq!(
        search.finalize_into(&world, &mut refs),
        Err(SearchError::NotInitialised)
    );
    assert!(
        refs.is_empty(),
        "an error leaves no half-written path behind"
    );
    search
        .init(&world, from, to, &QueryFilter::default())
        .unwrap();
    search.run(&world);
    let expected = search.finalize(&world).unwrap();
    let completeness = search.finalize_into(&world, &mut refs).unwrap();
    assert_eq!(
        (refs.clone(), completeness),
        (expected.refs, expected.completeness)
    );
    search.finalize_into(&world, &mut refs).unwrap();
    assert_eq!(
        refs.len(),
        refs.iter().collect::<std::collections::HashSet<_>>().len()
    );
}

fn wall_between_floors() -> (NavWorld, NavPoint, NavPoint, NavPoint) {
    let mut builder = MeshBuilder::new(1);
    let left = builder.triangle(
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 1.0),
    );
    let wall = builder.triangle(
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 1.0),
        Vec3::new(1.0, 1.0, 0.0),
    );
    let right = builder.triangle(
        Vec3::new(1.0, 0.0, 1.0),
        Vec3::new(1.0, 1.0, 0.0),
        Vec3::new(2.0, 0.0, 1.0),
    );
    let world = single_world(builder);
    let on = |tri: u16, pos: Vec3| NavPoint {
        nav_ref: nav_ref(&world, 1, tri),
        pos,
    };
    let points = (
        on(left, Vec3::new(0.8, 0.0, 0.3)),
        on(wall, Vec3::new(1.0, 0.5, 0.5)),
        on(right, Vec3::new(1.3, 0.3, 0.7)),
    );
    (world, points.0, points.1, points.2)
}

#[test]
fn a_triangle_without_area_is_never_entered_or_started_on() {
    let (world, left, wall, right) = wall_between_floors();
    let filter = QueryFilter::default();
    let mut search = PathSearch::new(64);
    let result = find_path(&world, &mut search, left, right, &filter).unwrap();
    assert_eq!(
        result.completeness,
        Completeness::Partial,
        "the only connection between the floors is a vertical face"
    );
    assert!(result.refs.iter().all(|r| *r != wall.nav_ref));
    assert_eq!(
        search.init(&world, wall, right, &filter),
        Err(SearchError::InvalidStart)
    );
    assert_eq!(
        search.init(&world, left, wall, &filter),
        Err(SearchError::InvalidGoal)
    );
}

#[test]
fn rays_stop_at_triangles_without_area_and_cannot_start_on_one() {
    let (world, left, wall, _) = wall_between_floors();
    let filter = QueryFilter::default();
    let hit = raycast(
        &world,
        left,
        Vec3::new(1.3, 0.0, 0.67),
        &filter,
        DEFAULT_MAX_STEPS,
    )
    .unwrap()
    .hit
    .expect("the vertical face is a wall");
    assert!(hit.t > 0.0 && hit.t < 1.0);
    assert_eq!(
        raycast(
            &world,
            wall,
            Vec3::new(1.3, 0.0, 0.67),
            &filter,
            DEFAULT_MAX_STEPS
        ),
        Err(RaycastError::DegenerateStart)
    );
}

#[test]
fn nearest_never_returns_a_triangle_without_area_even_as_a_boundary_snap() {
    let (world, _, wall, _) = wall_between_floors();
    let snap = NearestParams {
        below: 0.1,
        above: 0.1,
        step_tolerance: 0.1,
        fallback_radius: 20.0,
    };
    for x in [0.9, 1.0, 1.1] {
        let found = nearest(
            &world,
            Vec3::new(x, 10.0, 0.5),
            snap,
            &QueryFilter::default(),
        )
        .unwrap();
        assert!(!found.inside);
        assert_ne!(found.nav_ref, wall.nav_ref);
    }
}

// ---- raycast ----------------------------------------------------------------------------------

#[test]
fn raycast_rejects_non_finite_endpoints() {
    let world = single_world(MeshBuilder::new(1).flat_grid((0.0, 0.0), 2, 1));
    let start = locate(&world, Vec3::new(0.5, 0.0, 0.5));
    let filter = QueryFilter::default();
    let ok_end = Vec3::new(1.5, 0.0, 0.5);
    assert!(raycast(&world, start, ok_end, &filter, 8).is_ok());
    for bad in [f32::NAN, f32::INFINITY] {
        assert_eq!(
            raycast(&world, start, Vec3::new(bad, 0.0, 0.5), &filter, 8),
            Err(RaycastError::NonFiniteInput)
        );
        let broken_start = NavPoint {
            pos: Vec3::new(0.5, 0.0, bad),
            ..start
        };
        assert_eq!(
            raycast(&world, broken_start, ok_end, &filter, 8),
            Err(RaycastError::NonFiniteInput)
        );
    }
}

#[test]
fn a_ray_parallel_to_an_edge_never_exits_through_it() {
    let triangle = [
        Vec2::new(0.0, 0.0),
        Vec2::new(1.0, 0.0),
        Vec2::new(0.0, 1.0),
    ];
    // Just outside the bottom edge (float noise), moving parallel to it.
    let exit = exit_through_triangle(triangle, Vec2::new(0.25, -0.0001), Vec2::X).unwrap();
    assert_eq!(exit.edge, 1, "the hypotenuse, not the parallel bottom edge");
    assert!((exit.t - 0.7501).abs() < 1e-4, "{}", exit.t);
}

// ---- nearest ----------------------------------------------------------------------------------

#[test]
fn nearest_rejects_non_finite_positions_and_parameters() {
    let world = single_world(MeshBuilder::new(1).flat_grid((0.0, 0.0), 1, 1));
    let filter = QueryFilter::default();
    let here = Vec3::new(0.5, 0.0, 0.5);
    assert!(
        find_nearest(&world, here, WIDE_WINDOW, &filter)
            .unwrap()
            .is_some()
    );
    for bad in [f32::NAN, f32::INFINITY] {
        assert_eq!(
            find_nearest(&world, Vec3::new(bad, 0.0, 0.5), WIDE_WINDOW, &filter),
            Err(NearestError::NonFiniteInput)
        );
        for params in [
            NearestParams {
                below: bad,
                ..WIDE_WINDOW
            },
            NearestParams {
                above: bad,
                ..WIDE_WINDOW
            },
            NearestParams {
                step_tolerance: bad,
                ..WIDE_WINDOW
            },
            NearestParams {
                fallback_radius: bad,
                ..WIDE_WINDOW
            },
        ] {
            assert_eq!(
                find_nearest(&world, here, params, &filter),
                Err(NearestError::NonFiniteInput)
            );
        }
    }
}

#[test]
fn points_beside_a_mesh_footprint_find_nothing_without_the_fallback() {
    let world = single_world(MeshBuilder::new(1).flat_grid((0.0, 0.0), 2, 2));
    let filter = QueryFilter::default();
    for outside in [
        Vec3::new(-0.1, 0.0, 1.0),
        Vec3::new(2.1, 0.0, 1.0),
        Vec3::new(1.0, 0.0, -0.1),
        Vec3::new(1.0, 0.0, 2.1),
    ] {
        assert_eq!(
            nearest(&world, outside, WIDE_WINDOW, &filter),
            None,
            "{outside:?}"
        );
    }
    assert!(nearest(&world, Vec3::new(2.0, 0.0, 2.0), WIDE_WINDOW, &filter).is_some());
}

fn twin_floors() -> NavWorld {
    world_of(vec![
        MeshBuilder::new(1).flat_grid((0.0, 0.0), 1, 1).source(),
        MeshBuilder::new(2).flat_grid((0.0, 0.0), 1, 1).source(),
    ])
}

#[test]
fn equal_heights_resolve_to_the_lowest_reference() {
    let world = twin_floors();
    let filter = QueryFilter::default();
    let first = nav_ref(&world, 1, 0).slot();
    let at_or_below = nearest(&world, Vec3::new(0.7, 0.0, 0.2), WIDE_WINDOW, &filter).unwrap();
    assert_eq!(at_or_below.nav_ref.slot(), first);
    let above = nearest(&world, Vec3::new(0.7, -1.0, 0.2), WIDE_WINDOW, &filter).unwrap();
    assert_eq!(above.nav_ref.slot(), first);
}

#[test]
fn equally_distant_boundary_snaps_resolve_to_the_lowest_reference() {
    let world = twin_floors();
    let snap = NearestParams {
        fallback_radius: 2.0,
        ..WIDE_WINDOW
    };
    let found = nearest(
        &world,
        Vec3::new(-1.0, 0.0, 0.5),
        snap,
        &QueryFilter::default(),
    )
    .unwrap();
    assert_eq!(found.nav_ref.slot(), nav_ref(&world, 1, 0).slot());
}

#[test]
fn the_fallback_radius_is_inclusive() {
    let world = single_world(MeshBuilder::new(1).flat_grid((0.0, 0.0), 1, 1));
    let filter = QueryFilter::default();
    let at = Vec3::new(-2.0, 0.0, 0.5);
    let exact = NearestParams {
        fallback_radius: 2.0,
        ..WIDE_WINDOW
    };
    let found = nearest(&world, at, exact, &filter).unwrap();
    assert!(!found.inside);
    assert!(found.point.distance(Vec3::new(0.0, 0.0, 0.5)) < 1e-5);
    let short = NearestParams {
        fallback_radius: 1.999,
        ..WIDE_WINDOW
    };
    assert_eq!(nearest(&world, at, short, &filter), None);
}

// ---- funnel -----------------------------------------------------------------------------------

#[test]
fn string_pull_rejects_non_finite_input() {
    let world = single_world(MeshBuilder::new(1).flat_grid((0.0, 0.0), 1, 1));
    let corridor = [nav_ref(&world, 1, 0)];
    let (start, goal) = (Vec3::new(0.8, 0.0, 0.1), Vec3::new(0.7, 0.0, 0.2));
    assert!(string_pull(&world, &corridor, start, goal, 0.0).is_ok());
    for bad in [f32::NAN, f32::INFINITY] {
        let broken = Vec3::new(bad, 0.0, 0.0);
        assert_eq!(
            string_pull(&world, &corridor, broken, goal, 0.0),
            Err(FunnelError::NonFiniteInput)
        );
        assert_eq!(
            string_pull(&world, &corridor, start, broken, 0.0),
            Err(FunnelError::NonFiniteInput)
        );
        assert_eq!(
            string_pull(&world, &corridor, start, goal, bad),
            Err(FunnelError::NonFiniteInput)
        );
    }
}

#[test]
fn a_radius_beyond_half_the_portal_collapses_every_portal_to_its_midpoint() {
    let world = single_world(serpentine(MeshBuilder::new(1)));
    let (from, to) = (Vec3::new(0.5, 0.0, 0.5), Vec3::new(6.5, 0.0, 4.5));
    let (start, goal) = (locate(&world, from), locate(&world, to));
    let corridor = find_path(
        &world,
        &mut PathSearch::new(512),
        start,
        goal,
        &QueryFilter::default(),
    )
    .unwrap()
    .refs;
    let mut through_midpoints = vec![start.pos];
    for pair in corridor.windows(2) {
        through_midpoints.push(world.link_between(pair[0], pair[1]).unwrap().crossing());
    }
    through_midpoints.push(goal.pos);
    let expected: f32 = through_midpoints
        .windows(2)
        .map(|w| w[0].distance(w[1]))
        .sum();

    for radius in [0.75, 50.0] {
        let points = string_pull(&world, &corridor, start.pos, goal.pos, radius).unwrap();
        let length: f32 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
        assert!(
            (length - expected).abs() < 1e-3,
            "radius {radius}: {length} vs {expected}"
        );
    }
}
