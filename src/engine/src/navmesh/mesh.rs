use std::collections::HashMap;

use glam::{Vec2, Vec3};

use super::error::NavMeshError;
use super::geometry::{is_degenerate, signed_area2, xz};
use super::grid::TriangleGrid;
use super::link::EdgeLink;
use super::source::NavMeshSource;
use super::validate::validate;

pub const NO_NEIGHBOUR: u16 = u16::MAX;
const MIN_PLANE_NORMAL_Y: f32 = 1e-4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn overlaps(&self, min: Vec3, max: Vec3) -> bool {
        self.min.cmple(max).all() && self.max.cmpge(min).all()
    }
}

/// Triangle plane anchored at one corner, so heights are computed from small offsets rather than
/// from a plane offset that grows with the distance from the origin.
#[derive(Debug, Clone, Copy)]
struct Plane {
    anchor: Vec3,
    normal: Vec3,
}

impl Plane {
    fn through(a: Vec3, b: Vec3, c: Vec3) -> Self {
        Self {
            anchor: a,
            normal: (b - a).cross(c - a).normalize_or_zero(),
        }
    }

    fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        (self.normal.y.abs() >= MIN_PLANE_NORMAL_Y).then(|| {
            self.anchor.y
                - (self.normal.x * (x - self.anchor.x) + self.normal.z * (z - self.anchor.z))
                    / self.normal.y
        })
    }
}

/// One triangle with its index, corners and flags, as yielded by [`NavMesh::triangles`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TriangleView {
    pub index: u16,
    pub corners: [Vec3; 3],
    pub flags: u16,
}

/// One navmesh in engine space, stored as parallel arrays indexed by triangle.
#[derive(Debug, Clone)]
pub struct NavMesh {
    form_id: u32,
    vertices: Vec<Vec3>,
    triangles: Vec<[u16; 3]>,
    neighbours: Vec<[u16; 3]>,
    flags: Vec<u16>,
    planes: Vec<Plane>,
    bounds: Aabb,
    grid: TriangleGrid,
    edge_links: Vec<EdgeLink>,
    door_refs: HashMap<u16, Vec<u32>>,
    walkable: Vec<bool>,
    zero_area_triangles: usize,
    asymmetric_neighbours: usize,
}

impl NavMesh {
    pub fn new(source: NavMeshSource) -> Result<Self, NavMeshError> {
        let validated = validate(&source)?;
        let NavMeshSource {
            form_id,
            vertices,
            triangles,
            neighbours,
            flags,
            door_links,
            ..
        } = source;
        let neighbours = neighbours
            .into_iter()
            .map(|edges| edges.map(|n| u16::try_from(n).unwrap_or(NO_NEIGHBOUR)))
            .collect();
        let corners = |tri: &[u16; 3]| tri.map(|i| vertices[usize::from(i)]);
        let planes = triangles
            .iter()
            .map(|tri| {
                let [a, b, c] = corners(tri);
                Plane::through(a, b, c)
            })
            .collect();
        let triangle_bounds: Vec<(Vec2, Vec2)> = triangles
            .iter()
            .map(|tri| {
                let [a, b, c] = corners(tri).map(xz);
                (a.min(b).min(c), a.max(b).max(c))
            })
            .collect();
        let bounds = triangles
            .iter()
            .flat_map(corners)
            .fold(None, |acc: Option<Aabb>, v| {
                Some(acc.map_or(Aabb { min: v, max: v }, |b| Aabb {
                    min: b.min.min(v),
                    max: b.max.max(v),
                }))
            })
            .ok_or(NavMeshError::Empty)?;
        let grid = TriangleGrid::build(&triangle_bounds, xz(bounds.min), xz(bounds.max));
        let mut door_refs: HashMap<u16, Vec<u32>> = HashMap::new();
        for door in &door_links {
            door_refs
                .entry(door.triangle)
                .or_default()
                .push(door.door_ref_form_id);
        }
        let walkable: Vec<bool> = triangles
            .iter()
            .map(|tri| {
                let [a, b, c] = corners(tri).map(xz);
                !is_degenerate(signed_area2(a, b, c))
            })
            .collect();
        let zero_area_triangles = walkable.iter().filter(|w| !**w).count();
        Ok(Self {
            form_id,
            vertices,
            triangles,
            neighbours,
            flags,
            planes,
            bounds,
            grid,
            edge_links: validated.edge_links,
            door_refs,
            walkable,
            zero_area_triangles,
            asymmetric_neighbours: validated.asymmetric_neighbours,
        })
    }

    pub fn form_id(&self) -> u32 {
        self.form_id
    }

    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    pub fn bounds(&self) -> Aabb {
        self.bounds
    }

    pub fn grid(&self) -> &TriangleGrid {
        &self.grid
    }

    pub fn edge_links(&self) -> &[EdgeLink] {
        &self.edge_links
    }

    pub fn contains_triangle(&self, tri: u16) -> bool {
        usize::from(tri) < self.triangles.len()
    }

    /// Triangles with no area on the XZ plane (vertical faces). They hold no point, so queries
    /// treat them as impassable.
    pub fn zero_area_triangle_count(&self) -> usize {
        self.zero_area_triangles
    }

    /// Edges whose neighbour does not list the triangle back. Such an edge is traversable only
    /// from the side that lists the neighbour.
    pub fn asymmetric_neighbour_count(&self) -> usize {
        self.asymmetric_neighbours
    }

    /// Whether the triangle has area on the XZ plane; `false` for an out-of-range index.
    pub fn is_walkable(&self, tri: u16) -> bool {
        self.walkable
            .get(usize::from(tri))
            .copied()
            .unwrap_or(false)
    }

    pub fn flags(&self, tri: u16) -> Option<u16> {
        self.flags.get(usize::from(tri)).copied()
    }

    /// Door reference form ids attached to a triangle; several doors can share one triangle.
    pub fn door_refs(&self, tri: u16) -> &[u32] {
        self.door_refs.get(&tri).map_or(&[], Vec::as_slice)
    }

    pub fn neighbour(&self, tri: u16, edge: u8) -> Option<u16> {
        let n = *self
            .neighbours
            .get(usize::from(tri))?
            .get(usize::from(edge))?;
        (n != NO_NEIGHBOUR).then_some(n)
    }

    /// Corner positions of a triangle in authored winding order.
    pub fn triangle_vertices(&self, tri: u16) -> Option<[Vec3; 3]> {
        let indices = self.triangles.get(usize::from(tri))?;
        Some(indices.map(|i| self.vertices[usize::from(i)]))
    }

    /// Endpoints of an edge: edge `e` runs from corner `e` to corner `(e + 1) % 3`.
    pub fn edge_vertices(&self, tri: u16, edge: u8) -> Option<(Vec3, Vec3)> {
        if edge > 2 {
            return None;
        }
        let corners = self.triangle_vertices(tri)?;
        Some((
            corners[usize::from(edge)],
            corners[(usize::from(edge) + 1) % 3],
        ))
    }

    pub fn height_at(&self, tri: u16, x: f32, z: f32) -> Option<f32> {
        self.planes.get(usize::from(tri))?.height_at(x, z)
    }

    pub fn triangles(&self) -> impl Iterator<Item = TriangleView> + '_ {
        self.triangles
            .iter()
            .zip(&self.flags)
            .enumerate()
            .map(|(index, (indices, &flags))| TriangleView {
                index: index as u16,
                corners: indices.map(|i| self.vertices[usize::from(i)]),
                flags,
            })
    }
}
