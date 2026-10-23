//! Strokes the panels of the chart share: a rectangle outline whose edges have their own
//! thickness, and the corner brackets.

use egui::{Color32, Painter, Pos2, Rect, Vec2};

use crate::rendering::pipboy::paint::fill;

/// The outline of `path`, a stroke centred on its edges: vertical edges `thickness.x` thick,
/// horizontal edges `thickness.y`, square corners.
pub fn outline_rect(painter: &Painter, path: Rect, thickness: Vec2, colour: Color32) {
    for edge in outline_edges(path, thickness) {
        fill(painter, edge, colour);
    }
}

fn outline_edges(path: Rect, thickness: Vec2) -> [Rect; 4] {
    let horizontal = |y: f32| {
        Rect::from_center_size(
            Pos2::new(path.center().x, y),
            Vec2::new(path.width() + thickness.x, thickness.y),
        )
    };
    let vertical = |x: f32| {
        Rect::from_center_size(
            Pos2::new(x, path.center().y),
            Vec2::new(thickness.x, path.height() + thickness.y),
        )
    };
    [
        horizontal(path.min.y),
        horizontal(path.max.y),
        vertical(path.min.x),
        vertical(path.max.x),
    ]
}

/// The horizontal bracket style: a vertical stroke centred on each side of `rect` with a hook of
/// `corner` length at its top and bottom pointing inward, square caps.
pub fn brackets(painter: &Painter, rect: Rect, line_width: f32, corner: f32, colour: Color32) {
    for rect in bracket_rects(rect, line_width, corner) {
        fill(painter, rect, colour);
    }
}

fn bracket_rects(rect: Rect, line: f32, corner: f32) -> [Rect; 6] {
    let half = line / 2.0;
    let bar = |x: f32| {
        Rect::from_min_max(
            Pos2::new(x - half, rect.min.y - half),
            Pos2::new(x + half, rect.max.y + half),
        )
    };
    let hook = |x: f32, y: f32, towards: f32| {
        let far = x + towards * corner;
        Rect::from_min_max(
            Pos2::new(x.min(far) - half, y - half),
            Pos2::new(x.max(far) + half, y + half),
        )
    };
    [
        bar(rect.min.x),
        bar(rect.max.x),
        hook(rect.min.x, rect.min.y, 1.0),
        hook(rect.min.x, rect.max.y, 1.0),
        hook(rect.max.x, rect.min.y, -1.0),
        hook(rect.max.x, rect.max.y, -1.0),
    ]
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "exact literals")]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::from_min_max(Pos2::new(50.0, 645.0), Pos2::new(290.0, 691.0))
    }

    #[test]
    fn the_brackets_are_centred_on_the_sides_and_hook_inward() {
        let rects = bracket_rects(rect(), 1.5, 6.0);
        assert_eq!((rects[0].min.x, rects[0].max.x), (49.25, 50.75));
        assert_eq!((rects[1].min.x, rects[1].max.x), (289.25, 290.75));
        assert_eq!((rects[2].min.x, rects[2].max.x), (49.25, 56.75));
        assert_eq!((rects[5].min.x, rects[5].max.x), (283.25, 290.75));
        assert_eq!((rects[2].min.y, rects[2].max.y), (644.25, 645.75));
    }

    #[test]
    fn the_outline_edges_use_their_own_thickness() {
        let edges = outline_edges(
            Rect::from_min_max(Pos2::ZERO, Pos2::new(100.0, 20.0)),
            Vec2::new(2.0, 1.0),
        );
        assert_eq!(edges[0].height(), 1.0);
        assert_eq!(edges[2].width(), 2.0);
        assert_eq!(edges[0].width(), 102.0, "the corners are filled");
    }
}
