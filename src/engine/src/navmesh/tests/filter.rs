use glam::Vec3;

use super::support::MeshBuilder;
use crate::navmesh::{FilterError, NavMesh, QueryFilter, flags};

fn assert_close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
}

fn doors_mesh() -> NavMesh {
    MeshBuilder::new(1)
        .flat_grid((0.0, 0.0), 1, 1)
        .door(0, 0xD1)
        .door(0, 0xD2)
        .door(1, 0xD3)
        .mesh()
}

#[test]
fn a_triangle_with_several_doors_is_blocked_when_any_of_them_is_blocked() {
    let mesh = doors_mesh();
    assert_eq!(mesh.door_refs(0), [0xD1, 0xD2]);
    assert!(QueryFilter::default().is_passable(&mesh, 0));
    let first = QueryFilter::default().blocking_door(0xD1);
    assert!(!first.is_passable(&mesh, 0));
    assert!(first.is_passable(&mesh, 1));
    let second = QueryFilter::default().blocking_door(0xD2);
    assert!(!second.is_passable(&mesh, 0));
    let unrelated = QueryFilter::default().blocking_door(0xD3);
    assert!(unrelated.is_passable(&mesh, 0));
    assert!(!unrelated.is_passable(&mesh, 1));
}

#[test]
fn clones_do_not_share_later_door_blocks() {
    let mesh = doors_mesh();
    let base = QueryFilter::default().blocking_door(0xD3);
    let widened = base.clone().blocking_door(0xD1);
    assert!(base.is_passable(&mesh, 0));
    assert!(!widened.is_passable(&mesh, 0));
}

#[test]
fn out_of_range_triangles_are_impassable_instead_of_panicking() {
    let mesh = doors_mesh();
    assert!(!QueryFilter::default().is_passable(&mesh, 2));
    assert!(!QueryFilter::default().is_passable(&mesh, u16::MAX));
    assert_eq!(QueryFilter::default().traversal_multiplier(&mesh, 2), None);
}

#[test]
fn deleted_excluded_and_arealess_triangles_are_impassable() {
    let mut builder = MeshBuilder::new(1).flat_grid((0.0, 0.0), 1, 1);
    let wall = builder.triangle(
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(0.0, 2.0, 1.0),
    );
    let mesh = builder
        .flag(0, flags::DELETED)
        .flag(1, flags::NO_LARGE_CREATURES)
        .mesh();
    let filter = QueryFilter::default();
    assert!(!filter.is_passable(&mesh, 0));
    assert!(filter.is_passable(&mesh, 1));
    assert!(!QueryFilter::for_large_creature().is_passable(&mesh, 1));
    assert!(!mesh.is_walkable(wall));
    assert!(!filter.is_passable(&mesh, wall));
}

#[test]
fn multipliers_multiply_per_set_flag_and_default_to_one() {
    let mesh = MeshBuilder::new(1)
        .flat_grid((0.0, 0.0), 1, 1)
        .flag(0, flags::WATER | flags::PREFERRED)
        .flag(1, flags::WATER)
        .mesh();
    let filter = QueryFilter::default()
        .with_multiplier(flags::WATER, 4.0)
        .unwrap()
        .with_multiplier(flags::PREFERRED, 0.5)
        .unwrap();
    assert_eq!(filter.traversal_multiplier(&mesh, 0), Some(2.0));
    assert_eq!(filter.traversal_multiplier(&mesh, 1), Some(4.0));
    assert_eq!(
        QueryFilter::default().traversal_multiplier(&mesh, 0),
        Some(1.0)
    );
}

#[test]
fn the_heuristic_floor_is_the_product_of_all_cheap_multipliers_capped_at_one() {
    assert_close(QueryFilter::default().min_cost_multiplier(), 1.0);
    let expensive_only = QueryFilter::default()
        .with_multiplier(flags::WATER, 5.0)
        .unwrap();
    assert_close(expensive_only.min_cost_multiplier(), 1.0);
    let cheap = QueryFilter::default()
        .with_multiplier(flags::PREFERRED, 0.5)
        .unwrap()
        .with_multiplier(flags::DOOR, 0.5)
        .unwrap();
    assert_close(cheap.min_cost_multiplier(), 0.25);
}

#[test]
fn invalid_multipliers_are_rejected() {
    assert_eq!(
        QueryFilter::default()
            .with_multiplier(flags::WATER | flags::DOOR, 2.0)
            .unwrap_err(),
        FilterError::NotSingleFlag(flags::WATER | flags::DOOR)
    );
    for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(matches!(
            QueryFilter::default().with_multiplier(flags::WATER, bad),
            Err(FilterError::InvalidMultiplier(_))
        ));
    }
}
