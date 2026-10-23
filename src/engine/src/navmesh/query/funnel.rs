//! Simple stupid funnel string pulling over a triangle corridor.
//!
//! The funnel runs on the XZ plane; every output point keeps the height of the portal endpoint
//! (or of the start/goal) it came from, so heights follow the walked surface.
//!
//! Portal links cross along the overlap of the two linked edges, so a T-junction yields the
//! stretch the short edge shares with the long one. A ledge link is not a shared stretch: the
//! corridor is split there. The path is pulled to the middle of the ledge's source edge, which
//! the output contains as a point, then continues from the landing point on the target edge, the
//! next point of the output. Two consecutive output points are therefore either connected by a
//! walk or, at a ledge, by a jump.
//!
//! Adjacency authored on one side only is one-way: a corridor that uses it against its direction
//! fails with `NotConnected`.

use glam::{Vec2, Vec3};

use crate::navmesh::geometry::{cross2, is_degenerate, signed_area2, xz};
use crate::navmesh::portal::Span;
use crate::navmesh::{NavRef, NavWorld};

const SAME_POINT_DISTANCE: f32 = 1e-6;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FunnelError {
    #[error("corridor is empty")]
    EmptyCorridor,
    #[error("corridor reference is stale")]
    StaleRef(NavRef),
    #[error("corridor triangles are not connected")]
    NotConnected { from: NavRef, to: NavRef },
    #[error("corridor triangle has no area")]
    DegenerateTriangle(NavRef),
    #[error("start, goal or agent radius is not finite")]
    NonFiniteInput,
}

#[derive(Debug, Clone, Copy)]
struct Portal {
    left: Vec3,
    right: Vec3,
}

impl Portal {
    fn point(point: Vec3) -> Self {
        Self {
            left: point,
            right: point,
        }
    }
}

/// Shortest path from `start` to `goal` inside `corridor`, as 3D corner points.
///
/// `agent_radius` pulls each portal inward from both ends (up to its midpoint) so corners keep
/// clear of portal endpoints.
pub fn string_pull(
    world: &NavWorld,
    corridor: &[NavRef],
    start: Vec3,
    goal: Vec3,
    agent_radius: f32,
) -> Result<Vec<Vec3>, FunnelError> {
    if !start.is_finite() || !goal.is_finite() || !agent_radius.is_finite() {
        return Err(FunnelError::NonFiniteInput);
    }
    if corridor.is_empty() {
        return Err(FunnelError::EmptyCorridor);
    }
    if let Some(&stale) = corridor.iter().find(|r| !world.is_valid(**r)) {
        return Err(FunnelError::StaleRef(stale));
    }
    let mut points: Vec<Vec3> = Vec::new();
    let mut portals = vec![Portal::point(start)];
    for pair in corridor.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let link = world
            .link_between(from, to)
            .ok_or(FunnelError::NotConnected { from, to })?;
        if let Some(landing) = link.landing {
            portals.push(Portal::point(link.crossing()));
            extend_distinct(&mut points, &pull_string(&portals));
            portals = vec![Portal::point(landing)];
        } else {
            portals.push(crossing_portal(world, from, link.span, agent_radius)?);
        }
    }
    portals.push(Portal::point(goal));
    extend_distinct(&mut points, &pull_string(&portals));
    Ok(points)
}

fn crossing_portal(
    world: &NavWorld,
    from: NavRef,
    span: Span,
    agent_radius: f32,
) -> Result<Portal, FunnelError> {
    let mesh = world.mesh(from).ok_or(FunnelError::StaleRef(from))?;
    let corners = mesh
        .triangle_vertices(from.tri())
        .ok_or(FunnelError::StaleRef(from))?;
    let orientation = signed_area2(xz(corners[0]), xz(corners[1]), xz(corners[2]));
    if is_degenerate(orientation) {
        return Err(FunnelError::DegenerateTriangle(from));
    }
    // Leaving a counter-clockwise triangle through an edge, the edge's end corner is on the left.
    let (left, right) = if orientation > 0.0 {
        (span.end, span.start)
    } else {
        (span.start, span.end)
    };
    Ok(shrink(Portal { left, right }, agent_radius))
}

fn shrink(portal: Portal, radius: f32) -> Portal {
    let length = portal.left.distance(portal.right);
    if length <= SAME_POINT_DISTANCE || radius <= 0.0 {
        return portal;
    }
    let inset = (radius / length).min(0.5);
    Portal {
        left: portal.left.lerp(portal.right, inset),
        right: portal.right.lerp(portal.left, inset),
    }
}

/// `cross(a - apex, b - apex)`: positive when `b` is counter-clockwise of `a` around `apex`.
fn turn(apex: Vec2, a: Vec2, b: Vec2) -> f32 {
    cross2(a - apex, b - apex)
}

fn pull_string(portals: &[Portal]) -> Vec<Vec3> {
    let mut apex = portals[0].left;
    let mut points = vec![apex];
    let (mut left, mut right) = (apex, apex);
    let (mut left_index, mut right_index) = (0, 0);
    let mut index = 1;
    while index < portals.len() {
        let portal = portals[index];
        let (apex_2d, left_2d, right_2d) = (xz(apex), xz(left), xz(right));

        if turn(apex_2d, right_2d, xz(portal.right)) >= 0.0 {
            if apex_2d == right_2d || turn(apex_2d, left_2d, xz(portal.right)) < 0.0 {
                (right, right_index) = (portal.right, index);
            } else {
                apex = left;
                push_distinct(&mut points, apex);
                index = left_index + 1;
                (left, right, left_index, right_index) = (apex, apex, index - 1, index - 1);
                continue;
            }
        }
        let right_2d = xz(right);
        if turn(apex_2d, left_2d, xz(portal.left)) <= 0.0 {
            if apex_2d == left_2d || turn(apex_2d, right_2d, xz(portal.left)) > 0.0 {
                (left, left_index) = (portal.left, index);
            } else {
                apex = right;
                push_distinct(&mut points, apex);
                index = right_index + 1;
                (left, right, left_index, right_index) = (apex, apex, index - 1, index - 1);
                continue;
            }
        }
        index += 1;
    }
    push_distinct(&mut points, portals[portals.len() - 1].left);
    points
}

fn push_distinct(points: &mut Vec<Vec3>, point: Vec3) {
    let duplicate = points
        .last()
        .is_some_and(|last| last.distance(point) <= SAME_POINT_DISTANCE);
    if !duplicate {
        points.push(point);
    }
}

fn extend_distinct(points: &mut Vec<Vec3>, more: &[Vec3]) {
    for &point in more {
        push_distinct(points, point);
    }
}
