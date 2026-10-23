use super::error::NavMeshError;
use super::link::{EdgeLink, LinkType};
use super::source::NavMeshSource;

pub struct Validated {
    pub edge_links: Vec<EdgeLink>,
    /// Edges whose neighbour does not list the triangle back; tolerated, counted for the log.
    pub asymmetric_neighbours: usize,
}

pub fn validate(source: &NavMeshSource) -> Result<Validated, NavMeshError> {
    let count = source.triangles.len();
    if count == 0 {
        return Err(NavMeshError::Empty);
    }
    if count > usize::from(u16::MAX) {
        return Err(NavMeshError::TooManyTriangles(count));
    }
    check_length("neighbours", count, source.neighbours.len())?;
    check_length("flags", count, source.flags.len())?;
    check_vertices(source)?;
    check_triangles(source)?;
    check_neighbours(source)?;
    check_door_links(source)?;
    let edge_links = convert_edge_links(source)?;
    check_edges_have_one_connection(source, &edge_links)?;
    Ok(Validated {
        edge_links,
        asymmetric_neighbours: count_asymmetric_neighbours(source),
    })
}

fn check_length(field: &'static str, expected: usize, actual: usize) -> Result<(), NavMeshError> {
    if expected == actual {
        Ok(())
    } else {
        Err(NavMeshError::LengthMismatch {
            field,
            expected,
            actual,
        })
    }
}

fn check_vertices(source: &NavMeshSource) -> Result<(), NavMeshError> {
    match source.vertices.iter().position(|v| !v.is_finite()) {
        Some(index) => Err(NavMeshError::NonFiniteVertex(index)),
        None => Ok(()),
    }
}

fn check_triangles(source: &NavMeshSource) -> Result<(), NavMeshError> {
    let count = source.vertices.len();
    for (triangle, indices) in source.triangles.iter().enumerate() {
        if let Some(&index) = indices.iter().find(|&&i| usize::from(i) >= count) {
            return Err(NavMeshError::VertexIndexOutOfRange {
                triangle,
                index,
                count,
            });
        }
    }
    Ok(())
}

fn check_neighbours(source: &NavMeshSource) -> Result<(), NavMeshError> {
    let count = source.triangles.len();
    for (triangle, edges) in source.neighbours.iter().enumerate() {
        for (edge, &value) in edges.iter().enumerate() {
            let in_range = value == -1 || (value >= 0 && (value as usize) < count);
            if !in_range {
                return Err(NavMeshError::NeighbourOutOfRange {
                    triangle,
                    edge,
                    value,
                });
            }
            if usize::try_from(value).is_ok_and(|n| n == triangle) {
                return Err(NavMeshError::SelfNeighbour { triangle, edge });
            }
        }
    }
    Ok(())
}

fn check_door_links(source: &NavMeshSource) -> Result<(), NavMeshError> {
    let count = source.triangles.len();
    for (link, door) in source.door_links.iter().enumerate() {
        if usize::from(door.triangle) >= count {
            return Err(NavMeshError::DoorLinkTriangleOutOfRange {
                link,
                triangle: door.triangle,
                count,
            });
        }
    }
    Ok(())
}

fn convert_edge_links(source: &NavMeshSource) -> Result<Vec<EdgeLink>, NavMeshError> {
    let count = source.triangles.len();
    source
        .edge_links
        .iter()
        .enumerate()
        .map(|(link, authored)| {
            if usize::from(authored.triangle) >= count {
                return Err(NavMeshError::EdgeLinkTriangleOutOfRange {
                    link,
                    triangle: authored.triangle,
                    count,
                });
            }
            if authored.edge > 2 {
                return Err(NavMeshError::EdgeLinkEdgeOutOfRange {
                    link,
                    edge: authored.edge,
                });
            }
            if authored.target_edge > 2 {
                return Err(NavMeshError::EdgeLinkTargetEdgeOutOfRange {
                    link,
                    edge: authored.target_edge,
                });
            }
            let link_type =
                LinkType::from_raw(authored.link_type).ok_or(NavMeshError::UnknownLinkType {
                    link,
                    link_type: authored.link_type,
                })?;
            Ok(EdgeLink {
                triangle: authored.triangle,
                edge: authored.edge,
                link_type,
                target_form_id: authored.target_form_id,
                target_triangle: authored.target_triangle,
                target_edge: authored.target_edge,
            })
        })
        .collect()
}

/// An edge is either shared with a neighbour in this mesh or linked out to another mesh.
fn check_edges_have_one_connection(
    source: &NavMeshSource,
    edge_links: &[EdgeLink],
) -> Result<(), NavMeshError> {
    for link in edge_links {
        let (triangle, edge) = (usize::from(link.triangle), usize::from(link.edge));
        if source.neighbours[triangle][edge] != -1 {
            return Err(NavMeshError::EdgeHasNeighbourAndLink { triangle, edge });
        }
    }
    Ok(())
}

fn count_asymmetric_neighbours(source: &NavMeshSource) -> usize {
    source
        .neighbours
        .iter()
        .enumerate()
        .flat_map(|(triangle, edges)| {
            edges
                .iter()
                .filter_map(|&n| usize::try_from(n).ok())
                .filter(move |&n| {
                    !source.neighbours[n]
                        .iter()
                        .any(|&back| usize::try_from(back).is_ok_and(|b| b == triangle))
                })
        })
        .count()
}
