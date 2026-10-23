//! Ledge links, T-junction portals and the link bookkeeping around them, from world resolution to
//! search, string pulling and raycast.

use glam::Vec3;

use super::optimality::{corridor_cost, reference_optimum};
use super::support::{
    MeshBuilder, ledge_shortcut, linked_pair, locate, nav_ref, single_world, t_junction_pair,
    world_of,
};
use crate::navmesh::query::NavPoint;
use crate::navmesh::query::funnel::string_pull;
use crate::navmesh::query::raycast::{DEFAULT_MAX_STEPS, raycast};
use crate::navmesh::query::search::{Completeness, PathSearch, find_path};
use crate::navmesh::{LinkType, NavWorld, QueryFilter, flags};

const LEDGE_DOWN: u32 = 2;
const LEDGE_UP: u32 = 1;

fn assert_near(actual: Vec3, expected: Vec3) {
    assert!(
        actual.distance(expected) < 1e-4,
        "{actual:?} != {expected:?}"
    );
}

fn path(
    world: &NavWorld,
    from: Vec3,
    to: Vec3,
    filter: &QueryFilter,
) -> (
    NavPoint,
    NavPoint,
    crate::navmesh::query::search::PathResult,
) {
    let (start, goal) = (locate(world, from), locate(world, to));
    let result = find_path(world, &mut PathSearch::new(4096), start, goal, filter).unwrap();
    (start, goal, result)
}

/// Two platforms joined only by a ledge: left cell triangles 0 and 1, right cell 2 and 3.
fn two_platforms(link_type: u32) -> NavWorld {
    single_world(
        MeshBuilder::new(1)
            .flat_grid((0.0, 0.0), 1, 1)
            .flat_grid((3.0, 0.0), 1, 1)
            .link((0, 1), link_type, 1, (3, 2)),
    )
}

const LEFT: Vec3 = Vec3::new(0.7, 0.0, 0.3);
const RIGHT: Vec3 = Vec3::new(3.3, 0.0, 0.7);

// ---- world: link geometry ---------------------------------------------------------------------

#[test]
fn a_ledge_crosses_at_the_source_edge_middle_and_lands_at_the_target_edge_middle() {
    let world = two_platforms(LEDGE_DOWN);
    let link = world
        .links(nav_ref(&world, 1, 0))
        .find(|l| l.link_type.is_some())
        .unwrap();
    assert_eq!(link.link_type, Some(LinkType::LedgeDown));
    assert!(link.is_ledge());
    assert_near(link.crossing(), Vec3::new(1.0, 0.0, 0.5));
    assert_near(link.landing.unwrap(), Vec3::new(3.0, 0.0, 0.5));
    assert_near(link.span.start, Vec3::new(1.0, 0.0, 0.0));
    assert_near(link.span.end, Vec3::new(1.0, 0.0, 1.0));
}

#[test]
fn portal_and_toggle_portal_links_have_no_landing() {
    for kind in [0, 3] {
        let world = single_world(
            MeshBuilder::new(1)
                .flat_grid((0.0, 0.0), 1, 1)
                .flat_grid((2.0, 0.0), 1, 1)
                .link((0, 1), kind, 1, (3, 2)),
        );
        let link = world
            .links(nav_ref(&world, 1, 0))
            .find(|l| l.link_type.is_some())
            .unwrap();
        assert!(!link.is_ledge(), "link type {kind}");
    }
}

#[test]
fn a_t_junction_portal_is_clipped_to_the_overlap() {
    let (long, short) = t_junction_pair(0xA, 0xB);
    let world = world_of(vec![long.source(), short.source()]);
    let forward = world
        .link_between(nav_ref(&world, 0xA, 0), nav_ref(&world, 0xB, 1))
        .unwrap();
    assert_near(forward.span.start, Vec3::new(4.0, 0.0, 0.0));
    assert_near(forward.span.end, Vec3::new(4.0, 0.0, 1.0));
    assert_near(forward.crossing(), Vec3::new(4.0, 0.0, 0.5));

    let back = world
        .link_between(nav_ref(&world, 0xB, 1), nav_ref(&world, 0xA, 0))
        .unwrap();
    assert_near(back.span.start, Vec3::new(4.0, 0.0, 1.0));
    assert_near(back.span.end, Vec3::new(4.0, 0.0, 0.0));
    assert_eq!(world.stats().portal_fallbacks, 0);
}

#[test]
fn non_overlapping_portal_edges_use_the_shorter_edge_and_are_counted() {
    let a = MeshBuilder::new(0xA)
        .flat_grid((0.0, 0.0), 1, 1)
        .link((0, 1), 0, 0xB, (0, 0));
    let b = MeshBuilder::new(0xB).flat_grid((1.0, 0.0), 1, 1);
    let mut world = NavWorld::new();
    world.add(a.mesh()).unwrap();
    let report = world.add(b.mesh()).unwrap();
    assert_eq!((report.linked, report.portal_fallbacks), (1, 1));
    assert_eq!(world.stats().portal_fallbacks, 1);
    let link = world
        .links(nav_ref(&world, 0xA, 0))
        .find(|l| l.link_type.is_some())
        .unwrap();
    assert_near(link.span.start, Vec3::new(1.0, 0.0, 0.0));
    assert_near(link.span.end, Vec3::new(1.0, 0.0, 1.0));
    world.remove(0xB).unwrap();
    assert_eq!(world.stats().portal_fallbacks, 0);
}

// ---- world: rejected, deferred and self links -------------------------------------------------

#[test]
fn a_rejected_link_stays_pending_and_resolves_when_a_larger_target_reloads() {
    let source = MeshBuilder::new(0xA)
        .flat_grid((0.0, 0.0), 1, 1)
        .link((0, 1), 0, 0xB, (9, 2));
    let mut world = NavWorld::new();
    world.add(source.mesh()).unwrap();
    let small = world
        .add(MeshBuilder::new(0xB).flat_grid((1.0, 0.0), 1, 1).mesh())
        .unwrap();
    assert_eq!(small.rejected.len(), 1);
    assert_eq!(small.rejected[0].source_form_id, 0xA);
    assert_eq!(
        world.pending_link_count(),
        1,
        "rejected links are retried later"
    );
    assert_eq!(world.resolved_link_count(), 0);

    world.remove(0xB).unwrap();
    let large = world
        .add(MeshBuilder::new(0xB).flat_grid((1.0, 0.0), 5, 1).mesh())
        .unwrap();
    assert_eq!((large.linked, large.rejected.len()), (1, 0));
    assert_eq!(world.pending_link_count(), 0);
    let target = world
        .links(nav_ref(&world, 0xA, 0))
        .find(|l| l.link_type.is_some())
        .unwrap()
        .target;
    assert_eq!(
        (world.mesh(target).unwrap().form_id(), target.tri()),
        (0xB, 9)
    );
}

#[test]
fn a_link_rejected_at_its_own_load_is_retried_when_the_target_reloads() {
    let mut world = NavWorld::new();
    world
        .add(MeshBuilder::new(0xB).flat_grid((1.0, 0.0), 1, 1).mesh())
        .unwrap();
    let source = MeshBuilder::new(0xA)
        .flat_grid((0.0, 0.0), 1, 1)
        .link((0, 1), 0, 0xB, (9, 2));
    let report = world.add(source.mesh()).unwrap();
    assert_eq!(report.rejected.len(), 1);
    assert_eq!(world.pending_link_count(), 1);

    world.remove(0xB).unwrap();
    assert_eq!(
        world.pending_link_count(),
        1,
        "unloading the target keeps it pending"
    );
    let report = world
        .add(MeshBuilder::new(0xB).flat_grid((1.0, 0.0), 5, 1).mesh())
        .unwrap();
    assert_eq!(report.linked, 1);
    assert_eq!(world.pending_link_count(), 0);
}

#[test]
fn unloading_the_source_forgets_its_rejected_links() {
    let mut world = NavWorld::new();
    world
        .add(MeshBuilder::new(0xB).flat_grid((1.0, 0.0), 1, 1).mesh())
        .unwrap();
    let source = MeshBuilder::new(0xA)
        .flat_grid((0.0, 0.0), 1, 1)
        .link((0, 1), 0, 0xB, (9, 2));
    world.add(source.mesh()).unwrap();
    world.remove(0xA).unwrap();
    assert_eq!(world.pending_link_count(), 0);
}

fn self_linked(form_id: u32) -> MeshBuilder {
    MeshBuilder::new(form_id)
        .flat_grid((0.0, 0.0), 1, 1)
        .link((0, 0), LEDGE_DOWN, form_id, (1, 1))
}

#[test]
fn a_self_link_resolves_within_its_own_mesh_and_unloads_cleanly() {
    let mut world = NavWorld::new();
    let report = world.add(self_linked(7).mesh()).unwrap();
    assert_eq!(
        (report.linked, report.deferred, report.rejected.len()),
        (1, 0, 0)
    );
    let from = nav_ref(&world, 7, 0);
    let link = world
        .links(from)
        .find(|l| l.link_type.is_some())
        .expect("the self link is traversable");
    assert_eq!(link.target, nav_ref(&world, 7, 1));
    assert_eq!(world.resolved_link_count(), 1);

    let unload = world.remove(7).unwrap();
    assert_eq!(unload.links_deferred, 0);
    assert_eq!(world.resident_count(), 0);
    assert_eq!(world.pending_link_count(), 0);
    assert!(!world.is_valid(from));
}

#[test]
fn a_reused_slot_carries_neither_stale_refs_nor_the_old_self_link() {
    let mut world = NavWorld::new();
    let first = world.add(self_linked(7).mesh()).unwrap();
    let stale = nav_ref(&world, 7, 0);
    world.remove(7).unwrap();

    let other = world
        .add(MeshBuilder::new(8).flat_grid((0.0, 0.0), 1, 1).mesh())
        .unwrap();
    assert_eq!(other.slot, first.slot, "the slot is reused");
    assert_ne!(other.salt, first.salt);
    assert!(!world.is_valid(stale));
    assert_eq!(world.resolved_link_count(), 0);

    world.remove(8).unwrap();
    let again = world.add(self_linked(7).mesh()).unwrap();
    assert_eq!(again.linked, 1);
    assert_eq!(world.resolved_link_count(), 1);
}

#[test]
fn a_self_link_does_not_disturb_links_to_other_meshes() {
    let mut world = NavWorld::new();
    let (a, b) = linked_pair(0xA, 0xB);
    world.add(b.mesh()).unwrap();
    world
        .add(a.link((0, 0), LEDGE_UP, 0xA, (1, 1)).mesh())
        .unwrap();
    assert_eq!(world.resolved_link_count(), 3);
    world.remove(0xA).unwrap();
    assert_eq!(world.resolved_link_count(), 0);
    assert_eq!(world.pending_link_count(), 1, "B's link back to A waits");
}

// ---- search -----------------------------------------------------------------------------------

#[test]
fn a_ledge_is_the_only_way_across_and_only_in_its_direction() {
    let world = two_platforms(LEDGE_DOWN);
    let filter = QueryFilter::default();
    let (_, _, across) = path(&world, LEFT, RIGHT, &filter);
    assert_eq!(across.completeness, Completeness::Complete);
    assert_eq!(across.refs, [nav_ref(&world, 1, 0), nav_ref(&world, 1, 3)]);

    let (_, _, back) = path(&world, RIGHT, LEFT, &filter);
    assert_eq!(back.completeness, Completeness::Partial);

    let no_ledges = QueryFilter::default().excluding_link_type(LinkType::LedgeDown);
    let (_, _, blocked) = path(&world, LEFT, RIGHT, &no_ledges);
    assert_eq!(blocked.completeness, Completeness::Partial);
}

#[test]
fn ledge_up_links_are_traversable_too() {
    let world = two_platforms(LEDGE_UP);
    let (_, _, across) = path(&world, LEFT, RIGHT, &QueryFilter::default());
    assert_eq!(across.completeness, Completeness::Complete);
}

#[test]
fn the_search_takes_a_cheap_ledge_and_the_detour_once_the_jump_costs_more() {
    let world = single_world(ledge_shortcut(1, LEDGE_DOWN).flag(3, flags::WATER));
    let (start, goal) = (locate(&world, LEFT), locate(&world, RIGHT));
    assert_eq!(goal.nav_ref.tri(), 3);

    let plain = QueryFilter::default();
    let (_, _, direct) = path(&world, LEFT, RIGHT, &plain);
    assert_eq!(direct.refs.len(), 2, "one jump beats walking around");
    let cost = corridor_cost(&world, &plain, &direct.refs, start.pos, goal.pos);
    let best = reference_optimum(&world, &plain, start, goal).unwrap();
    assert!((cost - best).abs() < 1e-4);

    let wet = QueryFilter::default()
        .with_multiplier(flags::WATER, 10.0)
        .unwrap();
    let (_, _, around) = path(&world, LEFT, RIGHT, &wet);
    assert!(
        around.refs.len() > 2,
        "the jump is charged at the landing triangle's price"
    );
    assert_eq!(around.completeness, Completeness::Complete);
    let cost = corridor_cost(&world, &wet, &around.refs, start.pos, goal.pos);
    let best = reference_optimum(&world, &wet, start, goal).unwrap();
    assert!((cost - best).abs() < 1e-3, "{cost} vs {best}");
}

#[test]
fn excluding_ledges_sends_the_search_around_instead() {
    let world = single_world(ledge_shortcut(1, LEDGE_DOWN));
    let no_ledges = QueryFilter::default().excluding_link_type(LinkType::LedgeDown);
    let (_, _, around) = path(&world, LEFT, RIGHT, &no_ledges);
    assert_eq!(around.completeness, Completeness::Complete);
    assert!(around.refs.len() > 2);
}

#[test]
fn searching_through_a_t_junction_uses_the_clipped_crossing() {
    let (long, short) = t_junction_pair(0xA, 0xB);
    let world = world_of(vec![long.source(), short.source()]);
    let filter = QueryFilter::default();
    let (start, goal, result) = path(
        &world,
        Vec3::new(1.0, 0.0, 3.5),
        Vec3::new(4.8, 0.0, 0.9),
        &filter,
    );
    assert_eq!(result.completeness, Completeness::Complete);
    let cost = corridor_cost(&world, &filter, &result.refs, start.pos, goal.pos);
    let best = reference_optimum(&world, &filter, start, goal).unwrap();
    assert!((cost - best).abs() < 1e-4);
    let expected = start.pos.distance(Vec3::new(2.0, 0.0, 2.0))
        + Vec3::new(2.0, 0.0, 2.0).distance(Vec3::new(4.0, 0.0, 0.5))
        + Vec3::new(4.0, 0.0, 0.5).distance(goal.pos);
    assert!((cost - expected).abs() < 1e-4, "{cost} vs {expected}");
}

// ---- string pulling ---------------------------------------------------------------------------

#[test]
fn a_ledge_splits_the_pulled_path_into_crossing_and_landing_points() {
    let world = two_platforms(LEDGE_DOWN);
    let (start, goal, result) = path(&world, LEFT, RIGHT, &QueryFilter::default());
    for radius in [0.0, 0.2] {
        let points = string_pull(&world, &result.refs, start.pos, goal.pos, radius).unwrap();
        assert_eq!(points.len(), 4, "radius {radius}: {points:?}");
        assert_near(points[0], start.pos);
        assert_near(points[1], Vec3::new(1.0, 0.0, 0.5));
        assert_near(points[2], Vec3::new(3.0, 0.0, 0.5));
        assert_near(points[3], goal.pos);
    }
}

#[test]
fn pulling_continues_from_the_landing_point_with_its_own_corners() {
    let builder = MeshBuilder::new(1)
        .flat_grid((0.0, 0.0), 1, 1)
        .grid(
            (3.0, 0.0),
            3,
            3,
            1.0,
            |_, _| 0.0,
            |col, row| col == 0 || row == 2,
        )
        .link((0, 1), LEDGE_DOWN, 1, (3, 2));
    let world = single_world(builder);
    let (start, goal, result) = path(
        &world,
        Vec3::new(0.7, 0.0, 0.3),
        Vec3::new(5.5, 0.0, 2.5),
        &QueryFilter::default(),
    );
    assert_eq!(result.completeness, Completeness::Complete);
    let points = string_pull(&world, &result.refs, start.pos, goal.pos, 0.0).unwrap();
    assert_near(points[1], Vec3::new(1.0, 0.0, 0.5));
    assert_near(points[2], Vec3::new(3.0, 0.0, 0.5));
    assert!(
        points.len() >= 5,
        "the L-shaped landing area needs a corner: {points:?}"
    );
    assert_near(*points.last().unwrap(), goal.pos);
    let corner = points[3];
    assert!(
        (corner.x - 4.0).abs() < 1e-4 && (corner.z - 2.0).abs() < 1e-4,
        "{corner:?}"
    );
}

#[test]
fn consecutive_ledges_keep_every_crossing_and_landing_point() {
    let world = single_world(
        MeshBuilder::new(1)
            .flat_grid((0.0, 0.0), 1, 1)
            .flat_grid((3.0, 0.0), 1, 1)
            .flat_grid((6.0, 0.0), 1, 1)
            .link((0, 1), LEDGE_DOWN, 1, (3, 2))
            .link((2, 1), LEDGE_DOWN, 1, (5, 2)),
    );
    let (start, goal, result) = path(
        &world,
        Vec3::new(0.7, 0.0, 0.3),
        Vec3::new(6.3, 0.0, 0.7),
        &QueryFilter::default(),
    );
    assert_eq!(result.completeness, Completeness::Complete);
    let points = string_pull(&world, &result.refs, start.pos, goal.pos, 0.0).unwrap();
    let expected = [
        start.pos,
        Vec3::new(1.0, 0.0, 0.5),
        Vec3::new(3.0, 0.0, 0.5),
        Vec3::new(4.0, 0.0, 0.5),
        Vec3::new(6.0, 0.0, 0.5),
        goal.pos,
    ];
    assert_eq!(points.len(), expected.len(), "{points:?}");
    for (actual, expected) in points.iter().zip(expected) {
        assert_near(*actual, expected);
    }
}

#[test]
fn a_t_junction_corridor_bends_at_the_end_of_the_clipped_portal() {
    let (long, short) = t_junction_pair(0xA, 0xB);
    let world = world_of(vec![long.source(), short.source()]);
    let filter = QueryFilter::default();
    let (from, to) = (Vec3::new(1.0, 0.0, 3.9), Vec3::new(4.8, 0.0, 0.9));
    let (start, goal, forward) = path(&world, from, to, &filter);
    let points = string_pull(&world, &forward.refs, start.pos, goal.pos, 0.0).unwrap();
    assert_eq!(points.len(), 3, "{points:?}");
    assert_near(points[1], Vec3::new(4.0, 0.0, 1.0));

    let (start, goal, back) = path(&world, to, from, &filter);
    let points = string_pull(&world, &back.refs, start.pos, goal.pos, 0.0).unwrap();
    assert_eq!(points.len(), 3, "{points:?}");
    assert_near(points[1], Vec3::new(4.0, 0.0, 1.0));
}

// ---- raycast ----------------------------------------------------------------------------------

#[test]
fn a_ray_crosses_a_t_junction_only_where_the_linked_edges_overlap() {
    let (long, short) = t_junction_pair(0xA, 0xB);
    let world = world_of(vec![long.source(), short.source()]);
    let filter = QueryFilter::default();
    let cast = |z: f32| {
        raycast(
            &world,
            locate(&world, Vec3::new(1.0, 0.0, z)),
            Vec3::new(6.0, 0.0, z),
            &filter,
            DEFAULT_MAX_STEPS,
        )
        .unwrap()
    };

    let through = cast(0.5);
    assert!(
        through
            .visited
            .iter()
            .any(|r| world.mesh(*r).unwrap().form_id() == 0xB)
    );
    assert!(
        (through.hit.unwrap().t - 0.8).abs() < 1e-4,
        "stops at the short mesh's far edge"
    );

    let beside = cast(3.5);
    assert!(
        beside
            .visited
            .iter()
            .all(|r| world.mesh(*r).unwrap().form_id() == 0xA),
        "z = 3.5 is outside the portal, so the long mesh's edge is a wall there"
    );
    let hit = beside.hit.unwrap();
    assert!((hit.t - 0.6).abs() < 1e-4);
    assert!(hit.normal.distance(Vec3::X) < 1e-5);
}

#[test]
fn a_ledge_is_a_wall_for_rays() {
    let world = two_platforms(LEDGE_DOWN);
    let result = raycast(
        &world,
        locate(&world, LEFT),
        Vec3::new(3.3, 0.0, 0.3),
        &QueryFilter::default(),
        DEFAULT_MAX_STEPS,
    )
    .unwrap();
    let hit = result.hit.expect("the ledge edge stops the ray");
    assert!((hit.t - 0.3 / 2.6).abs() < 1e-4, "{}", hit.t);
    assert!(hit.normal.distance(Vec3::X) < 1e-5);
    assert_eq!(result.visited.len(), 1);
}
