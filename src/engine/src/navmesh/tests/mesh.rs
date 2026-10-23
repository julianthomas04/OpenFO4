use glam::Vec3;

use super::support::MeshBuilder;
use crate::navmesh::{NavMesh, NavMeshError, NavMeshSource};

fn valid_source() -> NavMeshSource {
    MeshBuilder::new(1)
        .flat_grid((0.0, 0.0), 2, 2)
        .door(1, 0xD00)
        .source()
}

fn rejects(edit: impl FnOnce(&mut NavMeshSource)) -> NavMeshError {
    let mut source = valid_source();
    edit(&mut source);
    NavMesh::new(source).expect_err("source must be rejected")
}

#[test]
fn builds_valid_mesh_with_queries() {
    let mesh = NavMesh::new(valid_source()).unwrap();
    assert_eq!(mesh.triangle_count(), 8);
    assert_eq!(mesh.bounds().min, Vec3::ZERO);
    assert_eq!(mesh.bounds().max, Vec3::new(2.0, 0.0, 2.0));
    assert_eq!(mesh.door_refs(1), [0xD00]);
    assert!(mesh.door_refs(0).is_empty());
}

#[test]
fn neighbours_follow_shared_edges_and_boundary_is_none() {
    let mesh = MeshBuilder::new(1).flat_grid((0.0, 0.0), 1, 1).mesh();
    assert_eq!(mesh.neighbour(0, 2), Some(1));
    assert_eq!(mesh.neighbour(1, 0), Some(0));
    assert_eq!(mesh.neighbour(0, 0), None);
}

#[test]
fn plane_gives_height_on_a_slope_and_none_on_a_wall() {
    let slope = MeshBuilder::new(1)
        .grid((0.0, 0.0), 1, 1, 4.0, |x, _| x * 0.5, |_, _| true)
        .mesh();
    assert!((slope.height_at(0, 2.0, 1.0).unwrap() - 1.0).abs() < 1e-5);

    let mut builder = MeshBuilder::new(2);
    builder.triangle(
        Vec3::ZERO,
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(1.0, 1.0, 0.0),
    );
    assert_eq!(builder.mesh().height_at(0, 0.5, 0.0), None);
}

#[test]
fn rejects_empty_mesh() {
    assert_eq!(
        rejects(|s| {
            s.triangles.clear();
            s.neighbours.clear();
            s.flags.clear();
        }),
        NavMeshError::Empty
    );
}

#[test]
fn rejects_mismatched_lengths() {
    assert!(matches!(
        rejects(|s| {
            s.neighbours.pop();
        }),
        NavMeshError::LengthMismatch {
            field: "neighbours",
            ..
        }
    ));
    assert!(matches!(
        rejects(|s| {
            s.flags.pop();
        }),
        NavMeshError::LengthMismatch { field: "flags", .. }
    ));
}

#[test]
fn rejects_bad_vertices_and_indices() {
    assert_eq!(
        rejects(|s| s.vertices[2].x = f32::NAN),
        NavMeshError::NonFiniteVertex(2)
    );
    assert!(matches!(
        rejects(|s| s.triangles[0][1] = 999),
        NavMeshError::VertexIndexOutOfRange {
            triangle: 0,
            index: 999,
            ..
        }
    ));
}

#[test]
fn rejects_out_of_range_neighbours_but_tolerates_asymmetry() {
    assert!(matches!(
        rejects(|s| s.neighbours[0][0] = 8),
        NavMeshError::NeighbourOutOfRange {
            triangle: 0,
            edge: 0,
            value: 8
        }
    ));
    assert!(matches!(
        rejects(|s| s.neighbours[0][0] = -2),
        NavMeshError::NeighbourOutOfRange { value: -2, .. }
    ));
    let mut asymmetric = valid_source();
    asymmetric.neighbours[0][0] = 5;
    assert!(NavMesh::new(asymmetric).is_ok());
}

#[test]
fn rejects_bad_links() {
    let link = |s: &mut NavMeshSource, triangle, edge, link_type| {
        s.edge_links.push(crate::navmesh::EdgeLinkSource {
            triangle,
            edge,
            link_type,
            target_form_id: 2,
            target_triangle: 0,
            target_edge: 0,
        });
    };
    assert!(matches!(
        rejects(|s| link(s, 99, 0, 0)),
        NavMeshError::EdgeLinkTriangleOutOfRange { triangle: 99, .. }
    ));
    assert!(matches!(
        rejects(|s| link(s, 0, 3, 0)),
        NavMeshError::EdgeLinkEdgeOutOfRange { edge: 3, .. }
    ));
    assert!(matches!(
        rejects(|s| link(s, 0, 0, 4)),
        NavMeshError::UnknownLinkType { link_type: 4, .. }
    ));
}

#[test]
fn accepts_every_known_link_type() {
    for link_type in 0..4 {
        let source = MeshBuilder::new(1)
            .flat_grid((0.0, 0.0), 1, 1)
            .link((0, 0), link_type, 2, (0, 0))
            .source();
        assert!(NavMesh::new(source).is_ok());
    }
}

#[test]
fn rejects_bad_door_links() {
    assert!(matches!(
        rejects(|s| s.door_links[0].triangle = 50),
        NavMeshError::DoorLinkTriangleOutOfRange { triangle: 50, .. }
    ));
}
