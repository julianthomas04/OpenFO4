#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NavMeshError {
    #[error("navmesh has no triangles")]
    Empty,
    #[error("{field} has {actual} entries, expected {expected}")]
    LengthMismatch {
        field: &'static str,
        expected: usize,
        actual: usize,
    },
    #[error("{0} triangles exceed the 16-bit triangle index range")]
    TooManyTriangles(usize),
    #[error("vertex {0} has a non-finite coordinate")]
    NonFiniteVertex(usize),
    #[error("triangle {triangle} references vertex {index}, but only {count} exist")]
    VertexIndexOutOfRange {
        triangle: usize,
        index: u16,
        count: usize,
    },
    #[error("triangle {triangle} edge {edge} has invalid neighbour {value}")]
    NeighbourOutOfRange {
        triangle: usize,
        edge: usize,
        value: i16,
    },
    #[error("edge link {link} references triangle {triangle}, but only {count} exist")]
    EdgeLinkTriangleOutOfRange {
        link: usize,
        triangle: u16,
        count: usize,
    },
    #[error("edge link {link} uses edge {edge}; only 0..=2 exist")]
    EdgeLinkEdgeOutOfRange { link: usize, edge: u8 },
    #[error("edge link {link} has unknown link type {link_type}")]
    UnknownLinkType { link: usize, link_type: u32 },
    #[error("door link {link} references triangle {triangle}, but only {count} exist")]
    DoorLinkTriangleOutOfRange {
        link: usize,
        triangle: u16,
        count: usize,
    },
    #[error("triangle {triangle} edge {edge} lists the triangle itself as its neighbour")]
    SelfNeighbour { triangle: usize, edge: usize },
    #[error("triangle {triangle} edge {edge} has both an internal neighbour and an edge link")]
    EdgeHasNeighbourAndLink { triangle: usize, edge: usize },
    #[error("edge link {link} targets edge {edge}; only 0..=2 exist")]
    EdgeLinkTargetEdgeOutOfRange { link: usize, edge: u8 },
}
