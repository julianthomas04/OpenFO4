use glam::{Vec2, Vec3};

use crate::navmesh::portal::{Span, clip_portal, full_edge};

fn edge(x0: f32, z0: f32, x1: f32, z1: f32) -> (Vec3, Vec3) {
    (Vec3::new(x0, 0.0, z0), Vec3::new(x1, 0.0, z1))
}

fn assert_span(span: Span, expected: (Vec3, Vec3)) {
    assert!(span.start.distance(expected.0) < 1e-5, "{span:?}");
    assert!(span.end.distance(expected.1) < 1e-5, "{span:?}");
}

#[test]
fn coinciding_edges_keep_the_whole_edge() {
    let clipped = clip_portal(edge(0.0, 0.0, 0.0, 2.0), edge(0.0, 2.0, 0.0, 0.0));
    assert!(!clipped.fallback);
    assert_span(clipped.span, edge(0.0, 0.0, 0.0, 2.0));
}

#[test]
fn a_short_target_edge_clips_a_long_source_edge() {
    let clipped = clip_portal(edge(0.0, 0.0, 0.0, 4.0), edge(0.5, 1.0, 0.5, 2.0));
    assert!(!clipped.fallback);
    assert_span(clipped.span, edge(0.0, 1.0, 0.0, 2.0));
}

#[test]
fn a_long_target_edge_leaves_a_short_source_edge_whole() {
    let clipped = clip_portal(edge(0.0, 1.0, 0.0, 2.0), edge(0.0, 0.0, 0.0, 4.0));
    assert!(!clipped.fallback);
    assert_span(clipped.span, edge(0.0, 1.0, 0.0, 2.0));
}

#[test]
fn partial_overlap_is_cut_on_the_side_that_sticks_out() {
    let clipped = clip_portal(edge(0.0, 0.0, 0.0, 2.0), edge(0.0, 1.0, 0.0, 5.0));
    assert_span(clipped.span, edge(0.0, 1.0, 0.0, 2.0));
    let reversed = clip_portal(edge(0.0, 0.0, 0.0, 2.0), edge(0.0, 5.0, 0.0, 1.0));
    assert_span(reversed.span, edge(0.0, 1.0, 0.0, 2.0));
}

#[test]
fn the_span_is_ordered_along_the_source_edge_whatever_the_target_direction() {
    let clipped = clip_portal(edge(0.0, 4.0, 0.0, 0.0), edge(0.0, 1.0, 0.0, 2.0));
    assert_span(clipped.span, edge(0.0, 2.0, 0.0, 1.0));
}

#[test]
fn a_target_edge_beside_the_source_line_still_overlaps_by_projection() {
    let clipped = clip_portal(edge(0.0, 0.0, 0.0, 4.0), edge(1.5, 1.0, 1.5, 3.0));
    assert!(!clipped.fallback);
    assert_span(clipped.span, edge(0.0, 1.0, 0.0, 3.0));
}

#[test]
fn heights_are_interpolated_on_the_source_edge() {
    let source = (Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 4.0, 4.0));
    let clipped = clip_portal(source, edge(0.0, 1.0, 0.0, 2.0));
    assert_span(
        clipped.span,
        (Vec3::new(0.0, 1.0, 1.0), Vec3::new(0.0, 2.0, 2.0)),
    );
}

#[test]
fn disjoint_edges_fall_back_to_the_shorter_edge_in_source_direction() {
    let clipped = clip_portal(edge(0.0, 0.0, 0.0, 3.0), edge(0.0, 8.0, 0.0, 6.0));
    assert!(clipped.fallback);
    assert_span(clipped.span, edge(0.0, 6.0, 0.0, 8.0));

    let source_shorter = clip_portal(edge(0.0, 1.0, 0.0, 0.0), edge(0.0, 8.0, 0.0, 4.0));
    assert!(source_shorter.fallback);
    assert_span(source_shorter.span, edge(0.0, 1.0, 0.0, 0.0));
}

#[test]
fn touching_edges_count_as_no_overlap() {
    let clipped = clip_portal(edge(0.0, 0.0, 0.0, 1.0), edge(0.0, 1.0, 0.0, 2.0));
    assert!(clipped.fallback);
}

#[test]
fn a_zero_length_source_edge_falls_back_without_dividing_by_zero() {
    let clipped = clip_portal(edge(1.0, 1.0, 1.0, 1.0), edge(0.0, 0.0, 0.0, 2.0));
    assert!(clipped.fallback);
    assert!(clipped.span.start.is_finite() && clipped.span.end.is_finite());
    assert_span(clipped.span, edge(1.0, 1.0, 1.0, 1.0));
}

#[test]
fn span_contains_points_within_the_tolerance_only() {
    let span = full_edge(edge(0.0, 0.0, 0.0, 2.0));
    assert!(span.contains_xz(Vec2::new(0.0, 1.0), 1e-3));
    assert!(span.contains_xz(Vec2::new(0.0005, 1.0), 1e-3));
    assert!(!span.contains_xz(Vec2::new(0.01, 1.0), 1e-3));
    assert!(!span.contains_xz(Vec2::new(0.0, 2.1), 1e-3));
    assert!(span.contains_xz(Vec2::new(0.0, 2.0005), 1e-3));
}

#[test]
fn midpoint_and_length_describe_the_span() {
    let span = full_edge((Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 2.0, 4.0)));
    assert_eq!(span.midpoint(), Vec3::new(0.0, 1.0, 2.0));
    assert!((span.length() - 20.0f32.sqrt()).abs() < 1e-6);
}
