use glam::{Vec2, Vec3};

use crate::navmesh::geometry::{closest_point_on_triangle, point_in_triangle, xz};
use crate::navmesh::{NavRef, NavWorld, QueryFilter, ResidentMesh};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NearestParams {
    /// How far below the query point a surface may lie.
    pub below: f32,
    /// How far above the query point a surface may lie.
    pub above: f32,
    /// Surfaces up to this far above the point still count as "at or below" it.
    pub step_tolerance: f32,
    /// When no surface lies under the point, search for the closest boundary point within this
    /// 3D distance; zero disables the fallback.
    pub fallback_radius: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NearestError {
    #[error("query position or window is not finite")]
    NonFiniteInput,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NearestResult {
    pub nav_ref: NavRef,
    pub point: Vec3,
    /// True when the point is directly above or below the triangle, false for a boundary snap.
    pub inside: bool,
}

/// Finds the triangle under `pos`.
///
/// Among surfaces inside the vertical window, the highest one at or below `pos + step_tolerance`
/// wins; failing that, the lowest one above. With no surface under the point the closest
/// boundary point within the fallback radius is returned. `Ok(None)` means nothing qualifies.
pub fn find_nearest(
    world: &NavWorld,
    pos: Vec3,
    params: NearestParams,
    filter: &QueryFilter,
) -> Result<Option<NearestResult>, NearestError> {
    let finite = [
        params.below,
        params.above,
        params.step_tolerance,
        params.fallback_radius,
    ]
    .iter()
    .all(|v| v.is_finite());
    if !pos.is_finite() || !finite {
        return Err(NearestError::NonFiniteInput);
    }
    Ok(find_under(world, pos, params, filter).or_else(|| find_closest(world, pos, params, filter)))
}

fn find_under(
    world: &NavWorld,
    pos: Vec3,
    params: NearestParams,
    filter: &QueryFilter,
) -> Option<NearestResult> {
    let point = xz(pos);
    let (floor, ceiling) = (pos.y - params.below, pos.y + params.above);
    let step_limit = pos.y + params.step_tolerance;
    let mut best_at_or_below: Option<(f32, NavRef)> = None;
    let mut best_above: Option<(f32, NavRef)> = None;
    for resident in world.resident() {
        let bounds = resident.mesh.bounds();
        let outside_footprint = point.x < bounds.min.x
            || point.x > bounds.max.x
            || point.y < bounds.min.z
            || point.y > bounds.max.z;
        if outside_footprint || bounds.min.y > ceiling || bounds.max.y < floor {
            continue;
        }
        for tri in resident.mesh.grid().candidates(point, point) {
            let Some(height) = surface_height(&resident, tri, point, filter) else {
                continue;
            };
            if !(floor..=ceiling).contains(&height) {
                continue;
            }
            let candidate = (height, resident.nav_ref(tri));
            if height <= step_limit {
                if best_at_or_below.is_none_or(|best| higher(candidate, best)) {
                    best_at_or_below = Some(candidate);
                }
            } else if best_above.is_none_or(|best| lower(candidate, best)) {
                best_above = Some(candidate);
            }
        }
    }
    let (height, nav_ref) = best_at_or_below.or(best_above)?;
    Some(NearestResult {
        nav_ref,
        point: Vec3::new(pos.x, height, pos.z),
        inside: true,
    })
}

fn higher(a: (f32, NavRef), b: (f32, NavRef)) -> bool {
    a.0 > b.0 || (a.0.total_cmp(&b.0).is_eq() && a.1 < b.1)
}

fn lower(a: (f32, NavRef), b: (f32, NavRef)) -> bool {
    a.0 < b.0 || (a.0.total_cmp(&b.0).is_eq() && a.1 < b.1)
}

fn surface_height(
    resident: &ResidentMesh,
    tri: u16,
    point: Vec2,
    filter: &QueryFilter,
) -> Option<f32> {
    if !filter.is_passable(resident.mesh, tri) {
        return None;
    }
    let corners = resident.mesh.triangle_vertices(tri)?.map(xz);
    if !point_in_triangle(point, corners) {
        return None;
    }
    resident.mesh.height_at(tri, point.x, point.y)
}

fn find_closest(
    world: &NavWorld,
    pos: Vec3,
    params: NearestParams,
    filter: &QueryFilter,
) -> Option<NearestResult> {
    if params.fallback_radius <= 0.0 {
        return None;
    }
    let reach = Vec3::splat(params.fallback_radius);
    let radius_sq = params.fallback_radius * params.fallback_radius;
    let mut best: Option<(f32, NavRef, Vec3)> = None;
    for resident in world.resident() {
        if !resident.mesh.bounds().overlaps(pos - reach, pos + reach) {
            continue;
        }
        let candidates = resident
            .mesh
            .grid()
            .candidates(xz(pos - reach), xz(pos + reach));
        for tri in candidates {
            if !filter.is_passable(resident.mesh, tri) {
                continue;
            }
            let Some([a, b, c]) = resident.mesh.triangle_vertices(tri) else {
                continue;
            };
            let closest = closest_point_on_triangle(pos, a, b, c);
            let distance_sq = closest.distance_squared(pos);
            let candidate = (distance_sq, resident.nav_ref(tri), closest);
            let closer = best.is_none_or(|(best_sq, best_ref, _)| {
                distance_sq < best_sq
                    || (distance_sq.total_cmp(&best_sq).is_eq() && candidate.1 < best_ref)
            });
            if distance_sq <= radius_sq && closer {
                best = Some(candidate);
            }
        }
    }
    best.map(|(_, nav_ref, point)| NearestResult {
        nav_ref,
        point,
        inside: false,
    })
}
