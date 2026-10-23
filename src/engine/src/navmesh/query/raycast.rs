use glam::{Vec2, Vec3};

use super::NavPoint;
use crate::navmesh::geometry::{exit_through_triangle, xz};
use crate::navmesh::{NavRef, NavWorld, QueryFilter};

pub const DEFAULT_MAX_STEPS: usize = 256;
/// How far, in engine units, a crossing may stray from the crossable stretch of a link.
const CROSSING_TOLERANCE: f32 = 1e-3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WallHit {
    /// Fraction of the 2D segment travelled before the wall.
    pub t: f32,
    /// Unit outward normal of the wall edge, in the ground plane (`y == 0`).
    pub normal: Vec3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RaycastResult {
    /// `None` when the segment reached its end without leaving the walkable surface.
    pub hit: Option<WallHit>,
    pub visited: Vec<NavRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RaycastError {
    #[error("raycast start reference is stale")]
    StaleStart,
    #[error("raycast start triangle has no area on the ground plane")]
    DegenerateStart,
    #[error("raycast start or end is not finite")]
    NonFiniteInput,
    #[error("raycast exceeded {0} triangle steps")]
    StepLimit(usize),
}

/// Walks the surface from `start` toward the XZ position of `end`, stopping at the first wall.
///
/// A wall is an edge without a usable connection: no neighbour, a filtered-out target, a
/// filtered-out link type, or a ledge link (a ledge is a jump, not a walkable crossing). A link
/// is followed only where the ray actually crosses its crossable stretch of the edge, so where an
/// edge faces several triangles the one under the crossing point is taken, and a crossing outside
/// every link's stretch is a wall.
pub fn raycast(
    world: &NavWorld,
    start: NavPoint,
    end: Vec3,
    filter: &QueryFilter,
    max_steps: usize,
) -> Result<RaycastResult, RaycastError> {
    if !start.pos.is_finite() || !end.is_finite() {
        return Err(RaycastError::NonFiniteInput);
    }
    let start_mesh = world.mesh(start.nav_ref).ok_or(RaycastError::StaleStart)?;
    if !start_mesh.contains_triangle(start.nav_ref.tri()) {
        return Err(RaycastError::StaleStart);
    }
    if !start_mesh.is_walkable(start.nav_ref.tri()) {
        return Err(RaycastError::DegenerateStart);
    }
    let origin = xz(start.pos);
    let dir = xz(end) - origin;
    let mut current = start.nav_ref;
    let mut visited = vec![current];
    for _ in 0..max_steps {
        let mesh = world.mesh(current).ok_or(RaycastError::StaleStart)?;
        let corners = mesh
            .triangle_vertices(current.tri())
            .ok_or(RaycastError::StaleStart)?
            .map(xz);
        let Some(exit) = exit_through_triangle(corners, origin, dir) else {
            return Ok(RaycastResult { hit: None, visited });
        };
        let crossing = origin + dir * exit.t;
        if let Some(next) = usable_link_on_edge(world, current, exit.edge as u8, crossing, filter) {
            current = next;
            visited.push(next);
            continue;
        }
        let hit = WallHit {
            t: exit.t,
            normal: Vec3::new(exit.normal.x, 0.0, exit.normal.y),
        };
        return Ok(RaycastResult {
            hit: Some(hit),
            visited,
        });
    }
    Err(RaycastError::StepLimit(max_steps))
}

fn usable_link_on_edge(
    world: &NavWorld,
    from: NavRef,
    edge: u8,
    crossing: Vec2,
    filter: &QueryFilter,
) -> Option<NavRef> {
    world
        .links(from)
        .filter(|link| {
            link.edge == edge
                && !link.is_ledge()
                && filter.allows_link(link.link_type)
                && link.span.contains_xz(crossing, CROSSING_TOLERANCE)
        })
        .map(|link| link.target)
        .find(|&target| {
            world
                .mesh(target)
                .is_some_and(|mesh| filter.is_passable(mesh, target.tri()))
        })
}
