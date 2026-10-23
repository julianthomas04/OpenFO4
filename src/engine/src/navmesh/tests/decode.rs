use glam::Vec3;

use crate::navmesh::flags::{EDGE_1_LINK, PREFERRED, WATER};
use crate::navmesh::{
    BuildError, DecodeError, DoorLinkSource, EdgeLinkSource, NavMesh, NavMeshRecord, NavWorld,
    SUPPORTED_FORMAT_VERSION, build_meshes, decode,
};

struct Tri {
    v: [u16; 3],
    edges: [i16; 3],
    flags: u16,
}

fn record(
    vertices: &[[f32; 3]],
    triangles: &[Tri],
    links: &[(u8, u8, u16, u32)],
    doors: &[(u16, u32)],
) -> NavMeshRecord {
    let mut vertex_blob = Vec::new();
    for coordinate in vertices.iter().flatten() {
        vertex_blob.extend_from_slice(&coordinate.to_le_bytes());
    }
    let mut triangle_blob = Vec::new();
    for t in triangles {
        for index in t.v {
            triangle_blob.extend_from_slice(&index.to_le_bytes());
        }
        for edge in t.edges {
            triangle_blob.extend_from_slice(&edge.to_le_bytes());
        }
        triangle_blob.extend_from_slice(&t.flags.to_le_bytes());
    }
    let mut link_blob = Vec::new();
    for &(link_type, target_edge, target_triangle, target_form_id) in links {
        link_blob.extend_from_slice(&[link_type, target_edge]);
        link_blob.extend_from_slice(&target_triangle.to_le_bytes());
        link_blob.extend_from_slice(&target_form_id.to_le_bytes());
    }
    let mut door_blob = Vec::new();
    for &(triangle, door) in doors {
        door_blob.extend_from_slice(&triangle.to_le_bytes());
        door_blob.extend_from_slice(&door.to_le_bytes());
    }
    NavMeshRecord {
        form_key: "000800:Test.esm".to_owned(),
        form_id: 0x800,
        format_version: SUPPORTED_FORMAT_VERSION,
        vertex_count: vertices.len(),
        triangle_count: triangles.len(),
        edge_link_count: links.len(),
        door_link_count: doors.len(),
        vertices: vertex_blob,
        triangles: triangle_blob,
        edge_links: link_blob,
        door_links: door_blob,
    }
}

/// Two triangles forming a quad in game space (x east, y north, z up), 100 units wide.
fn quad() -> NavMeshRecord {
    record(
        &QUAD_VERTICES,
        &[
            Tri {
                v: [0, 1, 2],
                edges: [-1, 1, -1],
                flags: PREFERRED,
            },
            Tri {
                v: [0, 2, 3],
                edges: [0, -1, -1],
                flags: WATER,
            },
        ],
        &[],
        &[],
    )
}

#[test]
fn vertices_are_converted_to_engine_space() {
    let source = decode(&quad()).unwrap();
    assert_eq!(source.form_id, 0x800);
    assert_eq!(
        source.vertices,
        vec![
            Vec3::new(0.0, 5.0, 0.0),
            Vec3::new(10.0, 5.0, 0.0),
            Vec3::new(10.0, 5.0, -10.0),
            Vec3::new(0.0, 5.0, -10.0),
        ]
    );
}

#[test]
fn triangles_neighbours_and_flags_pass_through() {
    let source = decode(&quad()).unwrap();
    assert_eq!(source.triangles, vec![[0, 1, 2], [0, 2, 3]]);
    assert_eq!(source.neighbours, vec![[-1, 1, -1], [0, -1, -1]]);
    assert_eq!(source.flags, vec![PREFERRED, WATER]);
    assert_eq!(source.edge_links, []);
}

const QUAD_VERTICES: [[f32; 3]; 4] = [
    [0.0, 0.0, 50.0],
    [100.0, 0.0, 50.0],
    [100.0, 100.0, 50.0],
    [0.0, 100.0, 50.0],
];

#[test]
fn flagged_edges_become_edge_links_and_unlinked_neighbours() {
    let record = record(
        &QUAD_VERTICES,
        &[
            Tri {
                v: [0, 1, 2],
                edges: [-1, 1, -1],
                flags: 0,
            },
            Tri {
                v: [0, 2, 3],
                edges: [0, 1, 1],
                flags: EDGE_1_LINK,
            },
        ],
        &[(1, 3, 7, 0x1234), (0, 2, 9, 0x00AB_CDEF)],
        &[],
    );

    let source = decode(&record).unwrap();

    assert_eq!(source.neighbours[1], [0, -1, 1]);
    assert_eq!(
        source.edge_links,
        vec![EdgeLinkSource {
            triangle: 1,
            edge: 1,
            link_type: 0,
            target_form_id: 0x00AB_CDEF,
            target_triangle: 9,
            target_edge: 2,
        }]
    );
}

#[test]
fn an_unreferenced_link_is_ignored() {
    let mut record = quad();
    record.edge_link_count = 1;
    record.edge_links = vec![0, 0, 0, 0, 1, 0, 0, 0];
    assert_eq!(decode(&record).unwrap().edge_links, []);
}

#[test]
fn link_flag_without_link_index_in_range_is_an_error() {
    let mut record = quad();
    record.triangles[6 + 14..8 + 14].copy_from_slice(&5i16.to_le_bytes());
    record.triangles[12 + 14..14 + 14].copy_from_slice(&1u16.to_le_bytes());
    assert_eq!(
        decode(&record).unwrap_err(),
        DecodeError::EdgeLinkIndexOutOfRange {
            triangle: 1,
            edge: 0,
            index: 5,
            count: 0
        }
    );
}

#[test]
fn negative_link_index_under_a_link_flag_is_an_error() {
    let mut record = quad();
    record.triangles[12..14].copy_from_slice(&EDGE_1_LINK.to_le_bytes());
    record.triangles[8..10].copy_from_slice(&(-1i16).to_le_bytes());
    assert!(matches!(
        decode(&record),
        Err(DecodeError::EdgeLinkIndexOutOfRange { index: -1, .. })
    ));
}

#[test]
fn door_links_are_decoded() {
    let mut record = quad();
    record.door_link_count = 1;
    record.door_links = [
        1u16.to_le_bytes().to_vec(),
        0x55AAu32.to_le_bytes().to_vec(),
    ]
    .concat();
    assert_eq!(
        decode(&record).unwrap().door_links,
        vec![DoorLinkSource {
            triangle: 1,
            door_ref_form_id: 0x55AA
        }]
    );
}

#[test]
fn unsupported_version_is_rejected() {
    let mut record = quad();
    record.format_version = 12;
    assert_eq!(
        decode(&record).unwrap_err(),
        DecodeError::UnsupportedVersion { found: 12 }
    );
}

type Corruption = Box<dyn Fn(&mut NavMeshRecord)>;

#[test]
fn every_blob_length_is_checked_against_its_count() {
    let corruptions: [(&str, Corruption); 4] = [
        ("vertices", Box::new(|r| r.vertices.truncate(35))),
        ("triangles", Box::new(|r| r.triangles.push(0))),
        ("edge_links", Box::new(|r| r.edge_link_count = 1)),
        ("door_links", Box::new(|r| r.door_link_count = 1)),
    ];
    for (column, corrupt) in corruptions {
        let mut record = quad();
        corrupt(&mut record);
        assert!(
            matches!(decode(&record), Err(DecodeError::BlobLength { column: c, .. }) if c == column),
            "{column} corruption was not reported"
        );
    }
}

#[test]
fn empty_blobs_decode_to_an_empty_source_that_the_mesh_rejects() {
    let record = record(&[], &[], &[], &[]);
    let source = decode(&record).unwrap();
    assert_eq!(source.triangles, [] as [[u16; 3]; 0]);
    assert!(NavMesh::new(source).is_err());
}

#[test]
fn decoded_record_builds_a_mesh_that_loads_into_a_world() {
    let mesh = NavMesh::new(decode(&quad()).unwrap()).unwrap();
    assert_eq!(mesh.triangle_count(), 2);
    let mut world = NavWorld::new();
    world.add(mesh).unwrap();
    assert_eq!(world.resident_count(), 1);
}

#[test]
fn build_meshes_skips_bad_records_and_names_them() {
    let mut bad_blob = quad();
    bad_blob.form_key = "BAD_BLOB".to_owned();
    bad_blob.vertices.pop();
    let mut bad_index = quad();
    bad_index.form_key = "BAD_INDEX".to_owned();
    bad_index.vertex_count = 2;
    bad_index.vertices.truncate(24);
    let outcome = build_meshes(&[bad_blob, quad(), bad_index]);

    assert_eq!(outcome.meshes.len(), 1);
    assert_eq!(
        outcome
            .failures
            .iter()
            .map(|f| f.form_key.as_str())
            .collect::<Vec<_>>(),
        ["BAD_BLOB", "BAD_INDEX"]
    );
    assert!(matches!(outcome.failures[0].error, BuildError::Decode(_)));
    assert!(matches!(outcome.failures[1].error, BuildError::Mesh(_)));
}
