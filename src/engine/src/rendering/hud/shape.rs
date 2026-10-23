//! Reading solid shapes of a movie as stage rectangles and flat colours.

use crate::ui_layout::UiLayoutError;
use crate::ui_layout::values::Shape;

const OPAQUE: f32 = 255.0;

/// A rectangle in stage pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StageRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl StageRect {
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }
}

/// The stage rectangle the fill of the shape at `key` covers, without any stroke; it must not be
/// rotated or skewed.
pub fn stage_rect(movie: &str, key: &str, shape: &Shape) -> Result<StageRect, UiLayoutError> {
    let matrix = shape.placement.world.matrix;
    if !matrix.is_axis_aligned() {
        return Err(UiLayoutError::unresolved(
            movie,
            key,
            "the shape is rotated or skewed",
        ));
    }
    let [xmin, ymin, xmax, ymax] = shape.edge_bounds;
    let (x0, y0) = matrix.apply(xmin, ymin);
    let (x1, y1) = matrix.apply(xmax, ymax);
    Ok(StageRect {
        x: x0.min(x1),
        y: y0.min(y1),
        w: (x1 - x0).abs(),
        h: (y1 - y0).abs(),
    })
}

/// The colour `[r, g, b, a]` (0 to 1) of a shape filled with one solid colour.
pub fn solid_fill(movie: &str, key: &str, shape: &Shape) -> Result<[f32; 4], UiLayoutError> {
    match shape.fills.as_slice() {
        [fill] if fill.kind == "solid" => fill
            .color
            .map(|rgba| rgba.map(|channel| f32::from(channel) / OPAQUE))
            .ok_or_else(|| UiLayoutError::unresolved(movie, key, "the fill has no colour")),
        _ => Err(UiLayoutError::unresolved(
            movie,
            key,
            "the shape is not one solid fill",
        )),
    }
}

/// [`solid_fill`] after the colour transform the shape is shown with.
pub fn shown_fill(movie: &str, key: &str, shape: &Shape) -> Result<[f32; 4], UiLayoutError> {
    let fill = solid_fill(movie, key, shape)?;
    let transform = shape.placement.world.color;
    let mut shown = [0.0; 4];
    for channel in 0..4 {
        let added = transform.add[channel] / OPAQUE;
        shown[channel] = (fill[channel] * transform.mult[channel] + added).clamp(0.0, 1.0);
    }
    Ok(shown)
}
