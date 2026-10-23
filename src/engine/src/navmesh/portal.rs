//! Where an edge link can actually be crossed, on the ground plane (engine space is Y-up).
//!
//! Authored links join an edge of one triangle to an edge of another, but the two edges need not
//! coincide: a long edge can face several short ones (a T-junction), and ledge links join edges
//! at different places altogether. The crossable part of a portal link is the overlap of the two
//! edges, measured along the source edge.

use glam::{Vec2, Vec3};

use super::geometry::xz;

/// Shortest overlap, in engine units, that still counts as a shared stretch of edge.
const MIN_OVERLAP: f32 = 1e-4;

/// A stretch of a source edge, ordered along the edge direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub start: Vec3,
    pub end: Vec3,
}

impl Span {
    pub fn midpoint(&self) -> Vec3 {
        self.start.midpoint(self.end)
    }

    pub fn length(&self) -> f32 {
        self.start.distance(self.end)
    }

    /// Whether `point` lies within `tolerance` of the span, measured on the ground plane.
    pub fn contains_xz(&self, point: Vec2, tolerance: f32) -> bool {
        let (a, b) = (xz(self.start), xz(self.end));
        let edge = b - a;
        let length_sq = edge.length_squared();
        let along = if length_sq <= f32::MIN_POSITIVE {
            0.0
        } else {
            ((point - a).dot(edge) / length_sq).clamp(0.0, 1.0)
        };
        (a + edge * along).distance(point) <= tolerance
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClippedPortal {
    pub span: Span,
    /// The edges did not overlap, so the span is the shorter edge's full extent instead.
    pub fallback: bool,
}

/// The part of `source` that faces `target`: both target endpoints are projected onto the source
/// edge line and the overlap of the two intervals is kept. Without an overlap the shorter edge's
/// endpoints are used, ordered along the source edge.
pub fn clip_portal(source: (Vec3, Vec3), target: (Vec3, Vec3)) -> ClippedPortal {
    let (s0, s1) = source;
    let direction = xz(s1) - xz(s0);
    let length_sq = direction.length_squared();
    if length_sq > f32::MIN_POSITIVE {
        let along = |p: Vec3| (xz(p) - xz(s0)).dot(direction) / length_sq;
        let (t0, t1) = (along(target.0), along(target.1));
        let (lo, hi) = (t0.min(t1).max(0.0), t0.max(t1).min(1.0));
        if (hi - lo) * length_sq.sqrt() > MIN_OVERLAP {
            return ClippedPortal {
                span: Span {
                    start: s0.lerp(s1, lo),
                    end: s0.lerp(s1, hi),
                },
                fallback: false,
            };
        }
    }
    let shorter = if source.0.distance(source.1) <= target.0.distance(target.1) {
        source
    } else {
        target
    };
    ClippedPortal {
        span: ordered_along(shorter, direction),
        fallback: true,
    }
}

fn ordered_along((a, b): (Vec3, Vec3), direction: Vec2) -> Span {
    if direction.dot(xz(b) - xz(a)) >= 0.0 {
        Span { start: a, end: b }
    } else {
        Span { start: b, end: a }
    }
}

/// The whole edge as a span, for links whose two sides share the edge exactly.
pub fn full_edge((start, end): (Vec3, Vec3)) -> Span {
    Span { start, end }
}
