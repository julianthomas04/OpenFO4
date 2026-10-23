//! Synthetic navmesh fixtures built in code; no game data.

use std::collections::HashMap;

use glam::Vec3;

use crate::navmesh::query::NavPoint;
use crate::navmesh::query::nearest::{NearestParams, NearestResult, find_nearest};
use crate::navmesh::{
    DoorLinkSource, EdgeLinkSource, NavMesh, NavMeshSource, NavRef, NavWorld, QueryFilter,
};

/// Builds a mesh from quad grids. In game-space mode the plane coordinates `(a, b)` are game
/// X/Y with counter-clockwise winding and the height is game Z, converted with
/// `(x, y, z) -> (x, z, -y)` exactly like the real import; otherwise the plane is engine XZ.
pub struct MeshBuilder {
    form_id: u32,
    game_space: bool,
    vertices: Vec<Vec3>,
    lookup: HashMap<[u32; 3], u16>,
    triangles: Vec<[u16; 3]>,
    flags: Vec<u16>,
    edge_links: Vec<EdgeLinkSource>,
    door_links: Vec<DoorLinkSource>,
}

impl MeshBuilder {
    pub fn new(form_id: u32) -> Self {
        Self {
            form_id,
            game_space: false,
            vertices: Vec::new(),
            lookup: HashMap::new(),
            triangles: Vec::new(),
            flags: Vec::new(),
            edge_links: Vec::new(),
            door_links: Vec::new(),
        }
    }

    pub fn game_space(mut self) -> Self {
        self.game_space = true;
        self
    }

    fn place(&self, a: f32, b: f32, height: f32) -> Vec3 {
        if self.game_space {
            Vec3::new(a, height, -b)
        } else {
            Vec3::new(a, height, b)
        }
    }

    fn vertex(&mut self, position: Vec3) -> u16 {
        let key = position.to_array().map(f32::to_bits);
        *self.lookup.entry(key).or_insert_with(|| {
            self.vertices.push(position);
            (self.vertices.len() - 1) as u16
        })
    }

    pub fn triangle(&mut self, a: Vec3, b: Vec3, c: Vec3) -> u16 {
        let indices = [self.vertex(a), self.vertex(b), self.vertex(c)];
        self.triangles.push(indices);
        self.flags.push(0);
        (self.triangles.len() - 1) as u16
    }

    /// Adds `cols x rows` unit-`cell` quads (two triangles each, in cell order) starting at
    /// `origin`; `keep` selects cells, `height` gives the surface height at a plane point.
    pub fn grid(
        mut self,
        origin: (f32, f32),
        cols: usize,
        rows: usize,
        cell: f32,
        height: impl Fn(f32, f32) -> f32,
        keep: impl Fn(usize, usize) -> bool,
    ) -> Self {
        for row in 0..rows {
            for col in 0..cols {
                if !keep(col, row) {
                    continue;
                }
                let (x0, b0) = (origin.0 + col as f32 * cell, origin.1 + row as f32 * cell);
                let (x1, b1) = (x0 + cell, b0 + cell);
                let corner = |a, b| self.place(a, b, height(a, b));
                let (p00, p10, p11, p01) = (
                    corner(x0, b0),
                    corner(x1, b0),
                    corner(x1, b1),
                    corner(x0, b1),
                );
                self.triangle(p00, p10, p11);
                self.triangle(p00, p11, p01);
            }
        }
        self
    }

    pub fn flat_grid(self, origin: (f32, f32), cols: usize, rows: usize) -> Self {
        self.grid(origin, cols, rows, 1.0, |_, _| 0.0, |_, _| true)
    }

    pub fn flag(mut self, tri: u16, bits: u16) -> Self {
        self.flags[usize::from(tri)] |= bits;
        self
    }

    pub fn door(mut self, tri: u16, door_ref_form_id: u32) -> Self {
        self.door_links.push(DoorLinkSource {
            triangle: tri,
            door_ref_form_id,
        });
        self
    }

    pub fn link(
        mut self,
        (triangle, edge): (u16, u8),
        link_type: u32,
        target_form_id: u32,
        (target_triangle, target_edge): (u16, u8),
    ) -> Self {
        self.edge_links.push(EdgeLinkSource {
            triangle,
            edge,
            link_type,
            target_form_id,
            target_triangle,
            target_edge,
        });
        self
    }

    pub fn source(self) -> NavMeshSource {
        let neighbours = adjacency(&self.triangles);
        NavMeshSource {
            form_id: self.form_id,
            vertices: self.vertices,
            triangles: self.triangles,
            neighbours,
            flags: self.flags,
            edge_links: self.edge_links,
            door_links: self.door_links,
        }
    }

    pub fn mesh(self) -> NavMesh {
        NavMesh::new(self.source()).expect("fixture mesh is valid")
    }
}

fn adjacency(triangles: &[[u16; 3]]) -> Vec<[i16; 3]> {
    let mut neighbours = vec![[-1i16; 3]; triangles.len()];
    let mut open_edges: HashMap<(u16, u16), (usize, usize)> = HashMap::new();
    for (tri, corners) in triangles.iter().enumerate() {
        for edge in 0..3 {
            let (a, b) = (corners[edge], corners[(edge + 1) % 3]);
            let key = (a.min(b), a.max(b));
            match open_edges.remove(&key) {
                Some((other, other_edge)) => {
                    neighbours[tri][edge] = other as i16;
                    neighbours[other][other_edge] = tri as i16;
                }
                None => {
                    open_edges.insert(key, (tri, edge));
                }
            }
        }
    }
    neighbours
}

pub fn world_of(sources: Vec<NavMeshSource>) -> NavWorld {
    let mut world = NavWorld::new();
    for source in sources {
        world
            .add(NavMesh::new(source).expect("fixture mesh is valid"))
            .expect("fixture forms are unique");
    }
    world
}

pub fn single_world(builder: MeshBuilder) -> NavWorld {
    world_of(vec![builder.source()])
}

pub const WIDE_WINDOW: NearestParams = NearestParams {
    below: 100.0,
    above: 100.0,
    step_tolerance: 0.1,
    fallback_radius: 0.0,
};

/// `find_nearest` for the finite inputs tests use.
pub fn nearest(
    world: &NavWorld,
    pos: Vec3,
    params: NearestParams,
    filter: &QueryFilter,
) -> Option<NearestResult> {
    find_nearest(world, pos, params, filter).expect("test inputs are finite")
}

/// Plane point `(a, b)` at height `height` as a navigable point, in the builder's space.
pub fn locate(world: &NavWorld, position: Vec3) -> NavPoint {
    let found = nearest(world, position, WIDE_WINDOW, &QueryFilter::default())
        .expect("position is on the mesh");
    NavPoint {
        nav_ref: found.nav_ref,
        pos: found.point,
    }
}

pub fn nav_ref(world: &NavWorld, form_id: u32, tri: u16) -> NavRef {
    world.nav_ref(form_id, tri).expect("triangle is resident")
}

pub fn path_length(points: &[Vec3]) -> f32 {
    points.windows(2).map(|w| w[0].distance(w[1])).sum()
}

/// Two unit-cell strips side by side, joined by reciprocal links across `x == 2`.
pub fn linked_pair(a_form: u32, b_form: u32) -> (MeshBuilder, MeshBuilder) {
    let a = MeshBuilder::new(a_form)
        .flat_grid((0.0, 0.0), 2, 1)
        .link((2, 1), 0, b_form, (1, 2));
    let b = MeshBuilder::new(b_form)
        .flat_grid((2.0, 0.0), 2, 1)
        .link((1, 2), 0, a_form, (2, 1));
    (a, b)
}

/// Lower floor (y = 0, x in 0..8), upper floor (y = 4, x in 0..4) and a ramp (x in 4..8, from
/// y = 4 down to y = 0) that meets the upper floor at x = 4 and the lower floor at x = 8.
pub fn stacked_floors(form_id: u32) -> MeshBuilder {
    MeshBuilder::new(form_id)
        .grid((0.0, 0.0), 8, 2, 1.0, |_, _| 0.0, |_, _| true)
        .grid((0.0, 0.0), 4, 2, 1.0, |_, _| 4.0, |_, _| true)
        .grid((4.0, 0.0), 4, 2, 1.0, |x, _| 8.0 - x, |_, _| true)
}

/// A one-cell-wide S-shaped corridor, 7 cells wide and 5 tall, on a surface of the given height.
pub fn serpentine_on(builder: MeshBuilder, height: impl Fn(f32, f32) -> f32) -> MeshBuilder {
    const ROWS: [&str; 5] = [".......", "######.", ".......", ".######", "......."];
    builder.grid((0.0, 0.0), 7, 5, 1.0, height, |col, row| {
        ROWS[row].as_bytes()[col] == b'.'
    })
}

pub fn serpentine(builder: MeshBuilder) -> MeshBuilder {
    serpentine_on(builder, |_, _| 0.0)
}

/// A 5x5 grid with the centre cell removed.
pub fn grid_with_hole(builder: MeshBuilder) -> MeshBuilder {
    builder.grid(
        (0.0, 0.0),
        5,
        5,
        1.0,
        |_, _| 0.0,
        |col, row| (col, row) != (2, 2),
    )
}

/// A 4 x 4 cell `long` mesh whose right edge (x = 4, z in 0..4) is linked to the left edge of the
/// first cell of a three-cell column `short` (x = 4, z in 0..1), and back: only the lowest unit of
/// the long edge faces the short mesh.
pub fn t_junction_pair(long_form: u32, short_form: u32) -> (MeshBuilder, MeshBuilder) {
    let long = MeshBuilder::new(long_form)
        .grid((0.0, 0.0), 1, 1, 4.0, |_, _| 0.0, |_, _| true)
        .link((0, 1), 0, short_form, (1, 2));
    let short = MeshBuilder::new(short_form)
        .grid((4.0, 0.0), 1, 3, 1.0, |_, _| 0.0, |_, _| true)
        .link((1, 2), 0, long_form, (0, 1));
    (long, short)
}

/// Two one-cell platforms with a row of detour cells above them. The only direct connection from
/// the left platform (triangles 0 and 1) to the right one (triangles 2 and 3) is a ledge link from
/// the left platform's right edge to the right platform's left edge; the detour (triangles 4..12)
/// joins both platforms by ordinary adjacency.
pub fn ledge_shortcut(form_id: u32, link_type: u32) -> MeshBuilder {
    MeshBuilder::new(form_id)
        .flat_grid((0.0, 0.0), 1, 1)
        .flat_grid((3.0, 0.0), 1, 1)
        .flat_grid((0.0, 1.0), 4, 1)
        .link((0, 1), link_type, form_id, (3, 2))
}
