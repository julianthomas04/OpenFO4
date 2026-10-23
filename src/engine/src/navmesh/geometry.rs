//! Planar helpers on the XZ ground plane (engine space is Y-up) and 3D triangle distance.

use glam::{Vec2, Vec3};

pub const INSIDE_EPSILON: f32 = 1e-5;
const DEGENERATE_AREA: f32 = 1e-9;

pub fn xz(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.z)
}

pub fn cross2(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

/// Twice the signed area of `abc`, positive when counter-clockwise with X right and Z up.
pub fn signed_area2(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    cross2(b - a, c - a)
}

pub fn is_degenerate(area2: f32) -> bool {
    area2.abs() <= DEGENERATE_AREA
}

pub fn point_in_triangle(p: Vec2, [a, b, c]: [Vec2; 3]) -> bool {
    let area = signed_area2(a, b, c);
    if is_degenerate(area) {
        return false;
    }
    let orientation = area.signum();
    [
        signed_area2(a, b, p),
        signed_area2(b, c, p),
        signed_area2(c, a, p),
    ]
    .iter()
    .all(|side| side * orientation >= -INSIDE_EPSILON)
}

pub fn closest_point_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return b + (c - b) * w;
    }
    let sum = va + vb + vc;
    if sum.abs() <= f32::MIN_POSITIVE {
        return a;
    }
    a + ab * (vb / sum) + ac * (vc / sum)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TriangleExit {
    pub edge: usize,
    pub t: f32,
    pub normal: Vec2,
}

/// Where the ray `origin + t * dir` leaves the triangle, if before `t == 1`.
///
/// `origin` is assumed to lie inside the triangle. Returns `None` when the ray ends inside
/// or the triangle is degenerate.
pub fn exit_through_triangle(tri: [Vec2; 3], origin: Vec2, dir: Vec2) -> Option<TriangleExit> {
    let area = signed_area2(tri[0], tri[1], tri[2]);
    if is_degenerate(area) {
        return None;
    }
    let orientation = area.signum();
    let mut exit: Option<TriangleExit> = None;
    for edge in 0..3 {
        let a = tri[edge];
        let b = tri[(edge + 1) % 3];
        let edge_dir = b - a;
        let normal = Vec2::new(edge_dir.y, -edge_dir.x) * orientation;
        let denom = normal.dot(dir);
        if denom <= 0.0 {
            continue;
        }
        let t = (normal.dot(a - origin) / denom).max(0.0);
        if exit.is_none_or(|best| t < best.t) {
            exit = Some(TriangleExit {
                edge,
                t,
                normal: normal.normalize_or_zero(),
            });
        }
    }
    exit.filter(|e| e.t < 1.0)
}
