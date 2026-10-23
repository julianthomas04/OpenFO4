use glam::Vec3;

use super::support::{MeshBuilder, grid_with_hole, linked_pair, locate, single_world, world_of};
use crate::navmesh::query::raycast::{DEFAULT_MAX_STEPS, RaycastError, raycast};
use crate::navmesh::{LinkType, QueryFilter, flags};

fn cast(
    world: &crate::navmesh::NavWorld,
    from: Vec3,
    to: Vec3,
    filter: &QueryFilter,
) -> crate::navmesh::query::raycast::RaycastResult {
    raycast(world, locate(world, from), to, filter, DEFAULT_MAX_STEPS).unwrap()
}

#[test]
fn clear_segment_reaches_its_end_and_visits_each_triangle_once() {
    let world = single_world(MeshBuilder::new(1).flat_grid((0.0, 0.0), 4, 4));
    let result = cast(
        &world,
        Vec3::new(0.5, 0.0, 0.5),
        Vec3::new(3.5, 0.0, 3.2),
        &QueryFilter::default(),
    );
    assert!(result.hit.is_none());
    let mut unique = result.visited.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), result.visited.len());
    assert!(result.visited.len() > 3);
}

#[test]
fn leaving_the_mesh_reports_fraction_and_outward_normal() {
    let world = single_world(MeshBuilder::new(1).flat_grid((0.0, 0.0), 2, 1));
    let result = cast(
        &world,
        Vec3::new(0.5, 0.0, 0.5),
        Vec3::new(3.5, 0.0, 0.5),
        &QueryFilter::default(),
    );
    let hit = result.hit.unwrap();
    assert!((hit.t - 0.5).abs() < 1e-4);
    assert!(hit.normal.distance(Vec3::X) < 1e-5);
}

#[test]
fn hole_is_a_wall_with_its_normal_pointing_into_the_hole() {
    let world = single_world(grid_with_hole(MeshBuilder::new(1)));
    let result = cast(
        &world,
        Vec3::new(0.5, 0.0, 2.5),
        Vec3::new(4.5, 0.0, 2.5),
        &QueryFilter::default(),
    );
    let hit = result.hit.unwrap();
    assert!((hit.t - 0.375).abs() < 1e-4);
    assert!(hit.normal.distance(Vec3::X) < 1e-5);
}

#[test]
fn rays_cross_links_unless_the_link_type_is_filtered() {
    let (a, b) = linked_pair(0xA, 0xB);
    let world = world_of(vec![a.source(), b.source()]);
    let (from, to) = (Vec3::new(0.5, 0.0, 0.25), Vec3::new(3.5, 0.0, 0.25));
    let open = cast(&world, from, to, &QueryFilter::default());
    assert!(open.hit.is_none());
    assert!(
        open.visited
            .iter()
            .any(|r| world.mesh(*r).unwrap().form_id() == 0xB)
    );

    let filter = QueryFilter::default().excluding_link_type(LinkType::Portal);
    let blocked = cast(&world, from, to, &filter);
    assert!((blocked.hit.unwrap().t - 0.5).abs() < 1e-4);
}

#[test]
fn filtered_triangles_act_as_walls() {
    let world = single_world(
        MeshBuilder::new(1)
            .flat_grid((0.0, 0.0), 3, 1)
            .flag(2, flags::NO_LARGE_CREATURES)
            .flag(3, flags::NO_LARGE_CREATURES),
    );
    let (from, to) = (Vec3::new(0.5, 0.0, 0.5), Vec3::new(2.5, 0.0, 0.5));
    assert!(
        cast(&world, from, to, &QueryFilter::default())
            .hit
            .is_none()
    );
    let hit = cast(&world, from, to, &QueryFilter::for_large_creature())
        .hit
        .unwrap();
    assert!((hit.t - 0.25).abs() < 1e-4);
}

#[test]
fn step_limit_and_stale_start_are_errors() {
    let mut world = single_world(MeshBuilder::new(1).flat_grid((0.0, 0.0), 8, 1));
    let start = locate(&world, Vec3::new(0.5, 0.0, 0.5));
    let far = Vec3::new(7.5, 0.0, 0.5);
    assert_eq!(
        raycast(&world, start, far, &QueryFilter::default(), 2),
        Err(RaycastError::StepLimit(2))
    );
    world.remove(1).unwrap();
    assert_eq!(
        raycast(&world, start, far, &QueryFilter::default(), 10),
        Err(RaycastError::StaleStart)
    );
}
