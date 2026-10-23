use glam::{Vec2, Vec3};

use crate::rendering::vertex::Vertex;

/// Triangles with a smaller doubled UV area carry no texture detail worth
/// streaming (the UVs are collapsed) and would divide by ~zero.
const MIN_UV_AREA_DOUBLED: f32 = 1e-12;
/// The stretchiest triangles covering this share of a submesh's world area are
/// ignored, so stretched slivers do not force the whole submesh to full
/// resolution. This is an estimate of the typical surface; where it falls
/// short the GPU feedback raises the detail.
const IGNORED_AREA_FRACTION: f32 = 0.1;

/// World units per UV unit along the sparsest texture axis of a triangle (the
/// largest singular value of the UV to surface mapping), with the triangle's
/// world area as weight.
struct TriangleStretch {
    world_area: f32,
    world_per_uv: f32,
}

fn triangle_stretch(positions: [Vec3; 3], uvs: [Vec2; 3]) -> Option<TriangleStretch> {
    let edge1 = positions[1] - positions[0];
    let edge2 = positions[2] - positions[0];
    let uv1 = uvs[1] - uvs[0];
    let uv2 = uvs[2] - uvs[0];
    let det = uv1.x * uv2.y - uv2.x * uv1.y;
    let world_area = edge1.cross(edge2).length() * 0.5;
    if det.abs() < MIN_UV_AREA_DOUBLED || !(world_area > 0.0) {
        return None;
    }
    let per_u = (edge1 * uv2.y - edge2 * uv1.y) / det;
    let per_v = (edge2 * uv1.x - edge1 * uv2.x) / det;
    let uu = per_u.length_squared();
    let vv = per_v.length_squared();
    let uv = per_u.dot(per_v);
    let largest_eigenvalue = f32::midpoint(uu, vv) + (((uu - vv) * 0.5).powi(2) + uv * uv).sqrt();
    let world_per_uv = largest_eigenvalue.sqrt();
    (world_per_uv.is_finite() && world_per_uv > 0.0).then_some(TriangleStretch {
        world_area,
        world_per_uv,
    })
}

/// How many world units one UV unit spans on a mesh, in the mesh's local
/// space: the high end of the per-triangle values (see `triangle_stretch`),
/// ignoring the stretchiest `IGNORED_AREA_FRACTION` of the surface. The high
/// end is the demanding one: a surface whose texture is stretched over more
/// world needs a finer mip at the same distance. `None` when no triangle has
/// usable UVs.
pub fn world_per_uv(vertices: &[Vertex], indices: &[u32]) -> Option<f32> {
    let vertex = |index: u32| vertices.get(index as usize);
    let mut triangles: Vec<TriangleStretch> = indices
        .as_chunks::<3>()
        .0
        .iter()
        .filter_map(|tri| {
            let corners = [vertex(tri[0])?, vertex(tri[1])?, vertex(tri[2])?];
            triangle_stretch(
                corners.map(|v| Vec3::from(v.position)),
                corners.map(|v| Vec2::from(v.tex_coords)),
            )
        })
        .collect();
    let total_area: f32 = triangles.iter().map(|t| t.world_area).sum();
    if triangles.is_empty() || !(total_area > 0.0) {
        return None;
    }

    triangles.sort_unstable_by(|a, b| b.world_per_uv.total_cmp(&a.world_per_uv));
    let ignored_area = total_area * IGNORED_AREA_FRACTION;
    let mut skipped = 0.0;
    for triangle in &triangles {
        skipped += triangle.world_area;
        if skipped > ignored_area {
            return Some(triangle.world_per_uv);
        }
    }
    triangles.last().map(|t| t.world_per_uv)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex(position: [f32; 3], tex_coords: [f32; 2]) -> Vertex {
        Vertex {
            position,
            tex_coords,
            normal: [0.0, 0.0, 1.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
        }
    }

    /// Right triangle spanning `width` x `height` world units over the unit UV triangle.
    fn stretched_triangle(width: f32, height: f32) -> Vec<Vertex> {
        vec![
            vertex([0.0, 0.0, 0.0], [0.0, 0.0]),
            vertex([width, 0.0, 0.0], [1.0, 0.0]),
            vertex([0.0, height, 0.0], [0.0, 1.0]),
        ]
    }

    #[test]
    fn world_per_uv_of_a_uniformly_scaled_triangle_is_the_scale() {
        let vertices = stretched_triangle(2.0, 2.0);
        let value = world_per_uv(&vertices, &[0, 1, 2]).unwrap();
        assert!((value - 2.0).abs() < 1e-5, "{value}");
    }

    #[test]
    fn world_per_uv_takes_the_sparser_axis() {
        let vertices = stretched_triangle(2.0, 8.0);
        let value = world_per_uv(&vertices, &[0, 1, 2]).unwrap();
        assert!((value - 8.0).abs() < 1e-5, "{value}");
    }

    #[test]
    fn world_per_uv_is_none_without_triangles() {
        assert_eq!(world_per_uv(&[], &[]), None);
    }

    #[test]
    fn world_per_uv_is_none_when_the_uvs_are_collapsed() {
        let vertices = vec![
            vertex([0.0, 0.0, 0.0], [0.5, 0.5]),
            vertex([1.0, 0.0, 0.0], [0.5, 0.5]),
            vertex([0.0, 1.0, 0.0], [0.5, 0.5]),
        ];
        assert_eq!(world_per_uv(&vertices, &[0, 1, 2]), None);
    }

    #[test]
    fn world_per_uv_skips_triangles_with_out_of_range_indices() {
        let vertices = stretched_triangle(3.0, 3.0);
        let value = world_per_uv(&vertices, &[0, 1, 7, 0, 1, 2]).unwrap();
        assert!((value - 3.0).abs() < 1e-5, "{value}");
    }

    #[test]
    fn world_per_uv_ignores_trailing_indices_that_do_not_form_a_triangle() {
        let vertices = stretched_triangle(3.0, 3.0);
        assert_eq!(world_per_uv(&vertices, &[0, 1]), None);
    }

    #[test]
    fn world_per_uv_ignores_a_stretched_sliver_below_the_ignored_area_share() {
        // A big triangle at 10 world per UV (area 50) and a thin sliver at 1000
        // world per UV (area 0.5, about 1% of the surface, under the 10% cut).
        let mut vertices = stretched_triangle(10.0, 10.0);
        vertices.extend([
            vertex([20.0, 0.0, 0.0], [0.0, 0.0]),
            vertex([1020.0, 0.0, 0.0], [1.0, 0.0]),
            vertex([20.0, 0.001, 0.0], [0.0, 1.0]),
        ]);
        let value = world_per_uv(&vertices, &[0, 1, 2, 3, 4, 5]).unwrap();
        assert!((value - 10.0).abs() < 1e-3, "{value}");
    }

    #[test]
    fn world_per_uv_keeps_a_stretched_region_above_the_ignored_area_share() {
        // The stretched triangle (40 world per UV) has half the surface area.
        let mut vertices = stretched_triangle(10.0, 10.0);
        vertices.extend([
            vertex([20.0, 0.0, 0.0], [0.0, 0.0]),
            vertex([60.0, 0.0, 0.0], [1.0, 0.0]),
            vertex([20.0, 2.5, 0.0], [0.0, 1.0]),
        ]);
        let value = world_per_uv(&vertices, &[0, 1, 2, 3, 4, 5]).unwrap();
        assert!((value - 40.0).abs() < 1e-3, "{value}");
    }
}
