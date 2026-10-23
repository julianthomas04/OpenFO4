//! Decoding of the installer's `navigation_meshes` blobs into a [`NavMeshSource`].

use glam::Vec3;

use super::flags::{EDGE_0_LINK, EDGE_1_LINK, EDGE_2_LINK};
use super::source::{DoorLinkSource, EdgeLinkSource, NavMeshSource};
use crate::bethesda_space::position_to_engine;

pub const SUPPORTED_FORMAT_VERSION: u32 = 15;

const VERTEX_STRIDE: usize = 12;
const TRIANGLE_STRIDE: usize = 14;
const EDGE_LINK_STRIDE: usize = 8;
const DOOR_LINK_STRIDE: usize = 6;
const EDGE_FLAGS: [u16; 3] = [EDGE_0_LINK, EDGE_1_LINK, EDGE_2_LINK];

/// An authored link as stored: where it leads, before it is attached to a triangle edge.
#[derive(Debug, Clone, Copy)]
struct RawEdgeLink {
    link_type: u32,
    target_form_id: u32,
    target_triangle: u16,
    target_edge: u8,
}

/// One `navigation_meshes` row as stored: counts plus the little-endian blob columns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NavMeshRecord {
    pub form_key: String,
    pub form_id: u32,
    pub format_version: u32,
    pub vertex_count: usize,
    pub triangle_count: usize,
    pub edge_link_count: usize,
    pub door_link_count: usize,
    pub vertices: Vec<u8>,
    pub triangles: Vec<u8>,
    pub edge_links: Vec<u8>,
    pub door_links: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("unsupported navmesh format version {found}; expected {SUPPORTED_FORMAT_VERSION}")]
    UnsupportedVersion { found: u32 },
    #[error("{column} blob is {actual} bytes, expected {expected} for {count} entries")]
    BlobLength {
        column: &'static str,
        count: usize,
        expected: usize,
        actual: usize,
    },
    #[error("triangle {triangle} edge {edge} refers to edge link {index}, but only {count} exist")]
    EdgeLinkIndexOutOfRange {
        triangle: usize,
        edge: usize,
        index: i16,
        count: usize,
    },
}

pub fn decode(record: &NavMeshRecord) -> Result<NavMeshSource, DecodeError> {
    if record.format_version != SUPPORTED_FORMAT_VERSION {
        return Err(DecodeError::UnsupportedVersion {
            found: record.format_version,
        });
    }
    let vertices = decode_vertices(record)?;
    let raw_links = decode_raw_edge_links(record)?;
    let (triangles, neighbours, flags, edge_links) = decode_triangles(record, &raw_links)?;
    Ok(NavMeshSource {
        form_id: record.form_id,
        vertices,
        triangles,
        neighbours,
        flags,
        edge_links,
        door_links: decode_door_links(record)?,
    })
}

fn check_blob(
    column: &'static str,
    blob: &[u8],
    count: usize,
    stride: usize,
) -> Result<(), DecodeError> {
    let expected = count.saturating_mul(stride);
    if blob.len() == expected {
        Ok(())
    } else {
        Err(DecodeError::BlobLength {
            column,
            count,
            expected,
            actual: blob.len(),
        })
    }
}

fn le_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

fn le_i16(bytes: &[u8]) -> i16 {
    i16::from_le_bytes([bytes[0], bytes[1]])
}

fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn le_f32(bytes: &[u8]) -> f32 {
    f32::from_bits(le_u32(bytes))
}

fn decode_vertices(record: &NavMeshRecord) -> Result<Vec<Vec3>, DecodeError> {
    check_blob(
        "vertices",
        &record.vertices,
        record.vertex_count,
        VERTEX_STRIDE,
    )?;
    Ok(record
        .vertices
        .chunks_exact(VERTEX_STRIDE)
        .map(|v| position_to_engine(le_f32(&v[0..4]), le_f32(&v[4..8]), le_f32(&v[8..12])))
        .collect())
}

fn decode_raw_edge_links(record: &NavMeshRecord) -> Result<Vec<RawEdgeLink>, DecodeError> {
    check_blob(
        "edge_links",
        &record.edge_links,
        record.edge_link_count,
        EDGE_LINK_STRIDE,
    )?;
    Ok(record
        .edge_links
        .chunks_exact(EDGE_LINK_STRIDE)
        .map(|link| RawEdgeLink {
            link_type: u32::from(link[0]),
            target_form_id: le_u32(&link[4..8]),
            target_triangle: le_u16(&link[2..4]),
            target_edge: link[1],
        })
        .collect())
}

type DecodedTriangles = (Vec<[u16; 3]>, Vec<[i16; 3]>, Vec<u16>, Vec<EdgeLinkSource>);

/// Triangle edges flagged as external carry an index into the link array instead of a neighbour:
/// those become `-1` neighbours plus one [`EdgeLinkSource`] positioned on that triangle edge.
fn decode_triangles(
    record: &NavMeshRecord,
    raw_links: &[RawEdgeLink],
) -> Result<DecodedTriangles, DecodeError> {
    check_blob(
        "triangles",
        &record.triangles,
        record.triangle_count,
        TRIANGLE_STRIDE,
    )?;
    let mut triangles = Vec::with_capacity(record.triangle_count);
    let mut neighbours = Vec::with_capacity(record.triangle_count);
    let mut flags = Vec::with_capacity(record.triangle_count);
    let mut edge_links = Vec::new();
    for (index, bytes) in record.triangles.chunks_exact(TRIANGLE_STRIDE).enumerate() {
        let triangle_flags = le_u16(&bytes[12..14]);
        let mut edges = [
            le_i16(&bytes[6..8]),
            le_i16(&bytes[8..10]),
            le_i16(&bytes[10..12]),
        ];
        for (edge, value) in edges.iter_mut().enumerate() {
            if triangle_flags & EDGE_FLAGS[edge] == 0 {
                continue;
            }
            let authored = usize::try_from(*value)
                .ok()
                .and_then(|slot| raw_links.get(slot))
                .ok_or(DecodeError::EdgeLinkIndexOutOfRange {
                    triangle: index,
                    edge,
                    index: *value,
                    count: raw_links.len(),
                })?;
            edge_links.push(EdgeLinkSource {
                triangle: index as u16,
                edge: edge as u8,
                link_type: authored.link_type,
                target_form_id: authored.target_form_id,
                target_triangle: authored.target_triangle,
                target_edge: authored.target_edge,
            });
            *value = -1;
        }
        triangles.push([
            le_u16(&bytes[0..2]),
            le_u16(&bytes[2..4]),
            le_u16(&bytes[4..6]),
        ]);
        neighbours.push(edges);
        flags.push(triangle_flags);
    }
    Ok((triangles, neighbours, flags, edge_links))
}

fn decode_door_links(record: &NavMeshRecord) -> Result<Vec<DoorLinkSource>, DecodeError> {
    check_blob(
        "door_links",
        &record.door_links,
        record.door_link_count,
        DOOR_LINK_STRIDE,
    )?;
    Ok(record
        .door_links
        .chunks_exact(DOOR_LINK_STRIDE)
        .map(|door| DoorLinkSource {
            triangle: le_u16(&door[0..2]),
            door_ref_form_id: le_u32(&door[2..6]),
        })
        .collect())
}
