//! Scaleform text-field geometry: where a text field's content box, first baseline and following
//! lines sit, how auto-size changes the box, and the largest size that fits. Pure arithmetic over
//! a [`TextField`] from the layout; the egui painter lives with the renderer.

use super::values::{Align, TextField};

/// Inset between a field's bounds and its content box, on every side, in field pixels. Exists
/// only in the native engine.
pub const FIELD_GUTTER_PX: f32 = 2.0;
/// Resolution of shrink-to-fit: candidate sizes are the authored size minus whole steps.
/// Unverified (A3): Scaleform's step is native and has not been measured.
pub const SHRINK_STEP_PX: f32 = 0.5;

/// A font's vertical metrics in font sizes: the baseline sits `ascent` below a line's top and a
/// line is `ascent + descent` tall before the field's leading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontMetrics {
    pub ascent: f32,
    pub descent: f32,
}

impl FontMetrics {
    pub fn baseline_offset(&self, size: f32) -> f32 {
        self.ascent * size
    }

    pub fn line_pitch(&self, size: f32, leading: f32) -> f32 {
        (self.ascent + self.descent) * size + leading
    }
}

/// How a field resizes itself to its text (`TextField.autoSize`), chosen by the menu's own code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoSize {
    Left,
    Center,
    Right,
}

/// A text field in stage pixels: its box, insets and the text format scaled by its placement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldGeometry {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
    pub left_inset: f32,
    pub right_inset: f32,
    pub vertical_inset: f32,
    pub size: f32,
    pub leading: f32,
    pub letter_spacing: f32,
    pub indent: f32,
    pub align: Align,
    pub wraps: bool,
    pub metrics: FontMetrics,
}

impl FieldGeometry {
    /// The geometry of `field` at its world placement. The layout accessor has already rejected
    /// rotated and skewed fields; the horizontal scale applies to x extents and the vertical scale
    /// to y extents and the font size.
    pub fn of(field: &TextField, metrics: FontMetrics) -> Self {
        let matrix = field.placement.world.matrix;
        let [xmin, ymin, xmax, ymax] = field.bounds;
        let (sx, sy) = (matrix.a, matrix.d);
        Self {
            left: matrix.tx + sx * xmin,
            right: matrix.tx + sx * xmax,
            top: matrix.ty + sy * ymin,
            bottom: matrix.ty + sy * ymax,
            left_inset: sx * (FIELD_GUTTER_PX + field.left_margin),
            right_inset: sx * (FIELD_GUTTER_PX + field.right_margin),
            vertical_inset: sy * FIELD_GUTTER_PX,
            size: field.size * sy,
            leading: field.leading * sy,
            letter_spacing: field.letter_spacing * sx,
            indent: field.indent * sx,
            align: field.align,
            wraps: field.word_wrap,
            metrics,
        }
    }

    /// A field whose content box starts at `content_top_left` and is `content_width` x
    /// `content_height`, with no margins or leading: for text the game lays out in code rather
    /// than in an authored field.
    pub fn content_box(
        content_top_left: (f32, f32),
        content_width: f32,
        content_height: f32,
        size: f32,
        align: Align,
        metrics: FontMetrics,
    ) -> Self {
        let (x, y) = content_top_left;
        Self {
            left: x - FIELD_GUTTER_PX,
            right: x + content_width + FIELD_GUTTER_PX,
            top: y - FIELD_GUTTER_PX,
            bottom: y + content_height + FIELD_GUTTER_PX,
            left_inset: FIELD_GUTTER_PX,
            right_inset: FIELD_GUTTER_PX,
            vertical_inset: FIELD_GUTTER_PX,
            size,
            leading: 0.0,
            letter_spacing: 0.0,
            indent: 0.0,
            align,
            wraps: false,
            metrics,
        }
    }

    /// The same field moved by `(dx, dy)`.
    #[must_use]
    pub fn translated(&self, dx: f32, dy: f32) -> Self {
        Self {
            left: self.left + dx,
            right: self.right + dx,
            top: self.top + dy,
            bottom: self.bottom + dy,
            ..*self
        }
    }

    /// The same field wrapping its text at the content width.
    #[must_use]
    pub fn wrapping(&self) -> Self {
        Self {
            wraps: true,
            ..*self
        }
    }

    pub fn content_left(&self) -> f32 {
        self.left + self.left_inset
    }

    pub fn content_right(&self) -> f32 {
        self.right - self.right_inset
    }

    pub fn content_width(&self) -> f32 {
        self.content_right() - self.content_left()
    }

    /// Top of the first line.
    pub fn line_top(&self) -> f32 {
        self.top + self.vertical_inset
    }

    pub fn content_height(&self) -> f32 {
        self.bottom - self.vertical_inset - self.line_top()
    }

    pub fn first_baseline(&self) -> f32 {
        self.line_top() + self.metrics.baseline_offset(self.size)
    }

    pub fn line_pitch(&self) -> f32 {
        self.metrics.line_pitch(self.size, self.leading)
    }

    /// The x text is anchored at inside the content box.
    pub fn anchor_x(&self) -> f32 {
        match self.align {
            Align::Left | Align::Justify => self.content_left(),
            Align::Center => f32::midpoint(self.content_left(), self.content_right()),
            Align::Right => self.content_right(),
        }
    }

    /// The same field at font size `size`.
    #[must_use]
    pub fn with_size(&self, size: f32) -> Self {
        Self { size, ..*self }
    }

    /// The field after auto-size to a text `text_width` wide: its edge on the anchored side stays,
    /// the width becomes the text plus the insets.
    #[must_use]
    pub fn auto_sized(&self, mode: AutoSize, text_width: f32) -> Self {
        let width = text_width + self.left_inset + self.right_inset;
        let (left, right) = match mode {
            AutoSize::Left => (self.left, self.left + width),
            AutoSize::Right => (self.right - width, self.right),
            AutoSize::Center => {
                let middle = f32::midpoint(self.left, self.right);
                (middle - width / 2.0, middle + width / 2.0)
            }
        };
        Self {
            left,
            right,
            ..*self
        }
    }

    /// The field after auto-size to wrapped text `text_height` tall: the top stays.
    #[must_use]
    pub fn auto_height(&self, text_height: f32) -> Self {
        Self {
            bottom: self.line_top() + text_height + self.vertical_inset,
            ..*self
        }
    }
}

/// The largest size not above `size` for which `fits` holds, trying the authored size and then
/// every [`SHRINK_STEP_PX`] step below it in turn (a wrapped text's fit is not monotonic in the
/// size, so no step is skipped). When nothing fits, the smallest positive candidate; a size that
/// is not a positive number is returned unchanged.
pub fn fit_size(size: f32, fits: impl Fn(f32) -> bool) -> f32 {
    if size.is_nan() || size <= 0.0 || fits(size) {
        return size;
    }
    let steps = (size / SHRINK_STEP_PX).ceil() as u32;
    let candidate = |step: u32| size - step as f32 * SHRINK_STEP_PX;
    (1..steps)
        .map(candidate)
        .find(|size| fits(*size))
        .unwrap_or_else(|| {
            if steps > 1 {
                candidate(steps - 1)
            } else {
                size
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_layout::test_support::{assert_close, text_field};

    use crate::ui_layout::test_support::FIXTURE_METRICS;
    const METRICS: FontMetrics = FIXTURE_METRICS;

    fn geometry(align: Align, bounds: [f32; 4], tx: f32, ty: f32, scale: f32) -> FieldGeometry {
        let mut field = text_field(bounds, 30.0);
        field.align = align;
        field.placement.world.matrix.tx = tx;
        field.placement.world.matrix.ty = ty;
        field.placement.world.matrix.a = scale;
        field.placement.world.matrix.d = scale;
        FieldGeometry::of(&field, METRICS)
    }

    #[test]
    fn the_content_box_is_inset_by_the_gutter_from_the_bounds() {
        let g = geometry(Align::Left, [-2.0, -2.0, 192.0, 40.0], 10.0, 100.0, 1.0);
        assert_close(g.content_left(), 10.0);
        assert_close(g.content_right(), 200.0);
        assert_close(g.line_top(), 100.0);
        assert_close(g.content_height(), 38.0);
    }

    #[test]
    fn the_first_baseline_sits_the_fonts_ascent_below_the_line_top() {
        let g = geometry(Align::Left, [-2.0, -2.0, 192.0, 40.0], 10.0, 100.0, 1.0);
        assert!((g.first_baseline() - (100.0 + METRICS.ascent * 30.0)).abs() < 1e-4);
    }

    #[test]
    fn the_line_pitch_is_ascent_plus_descent_plus_leading() {
        let metrics = FontMetrics {
            ascent: 0.8,
            descent: 0.3,
        };
        assert!((metrics.line_pitch(30.0, 2.0) - 35.0).abs() < 1e-4);
        assert!((metrics.line_pitch(30.0, -2.0) - 31.0).abs() < 1e-4);
    }

    #[test]
    fn alignment_anchors_inside_the_content_box() {
        let bounds = [-2.0, -2.0, 102.0, 40.0];
        assert_close(geometry(Align::Left, bounds, 0.0, 0.0, 1.0).anchor_x(), 0.0);
        assert_close(
            geometry(Align::Center, bounds, 0.0, 0.0, 1.0).anchor_x(),
            50.0,
        );
        assert_close(
            geometry(Align::Right, bounds, 0.0, 0.0, 1.0).anchor_x(),
            100.0,
        );
    }

    #[test]
    fn placement_scale_applies_to_extents_size_and_gutter() {
        let g = geometry(Align::Left, [-2.0, -2.0, 102.0, 40.0], 10.0, 20.0, 2.0);
        assert_close(g.size, 60.0);
        assert_close(g.content_left(), 10.0);
        assert_close(g.content_right(), 10.0 + 200.0);
        assert_close(g.line_top(), 20.0);
    }

    #[test]
    fn margins_narrow_the_content_box() {
        let mut field = text_field([-2.0, -2.0, 102.0, 40.0], 20.0);
        field.left_margin = 3.0;
        field.right_margin = 5.0;
        let g = FieldGeometry::of(&field, METRICS);
        assert_close(g.content_left(), 3.0);
        assert_close(g.content_right(), 95.0);
    }

    #[test]
    fn a_content_box_field_puts_its_content_where_asked() {
        let g = FieldGeometry::content_box((10.0, 20.0), 100.0, 30.0, 18.0, Align::Right, METRICS);
        assert_close(g.content_left(), 10.0);
        assert_close(g.content_right(), 110.0);
        assert_close(g.line_top(), 20.0);
        assert_close(g.content_height(), 30.0);
        assert_close(g.translated(5.0, 1.0).anchor_x(), 115.0);
    }

    #[test]
    fn auto_size_keeps_the_anchored_edge_and_adds_the_gutters() {
        let g = geometry(Align::Left, [-2.0, -2.0, 102.0, 40.0], 0.0, 0.0, 1.0);
        let left = g.auto_sized(AutoSize::Left, 30.0);
        assert_eq!((left.left, left.right), (-2.0, 32.0));
        let right = g.auto_sized(AutoSize::Right, 30.0);
        assert_eq!((right.left, right.right), (68.0, 102.0));
        let centre = g.auto_sized(AutoSize::Center, 30.0);
        assert_eq!((centre.left, centre.right), (33.0, 67.0));
    }

    #[test]
    fn auto_height_keeps_the_top_and_wraps_the_text() {
        let g = geometry(Align::Left, [-2.0, -2.0, 102.0, 40.0], 0.0, 0.0, 1.0);
        assert_close(g.auto_height(50.0).bottom, 52.0);
    }

    #[test]
    fn text_that_fits_keeps_its_size() {
        assert_close(fit_size(28.0, |size| size * 3.0 <= 100.0), 28.0);
    }

    #[test]
    fn the_largest_fitting_size_is_found_in_half_pixel_steps() {
        assert_close(fit_size(30.0, |size| size * 10.0 <= 201.0), 20.0);
        assert_close(fit_size(30.0, |size| size * 10.0 <= 204.0), 20.0);
        assert_close(fit_size(30.0, |size| size * 10.0 <= 205.0), 20.5);
    }

    #[test]
    fn a_non_monotonic_fit_finds_the_largest_fitting_step() {
        let fits = |size: f32| (size - 25.0).abs() < 0.1 || size <= 10.0;
        assert_close(fit_size(30.0, fits), 25.0);
    }

    #[test]
    fn nothing_fitting_returns_the_smallest_candidate() {
        assert_close(fit_size(3.0, |_| false), 0.5);
    }

    #[test]
    fn a_size_that_is_not_a_positive_number_is_returned_unchanged() {
        assert!(fit_size(f32::NAN, |_| false).is_nan());
        assert_close(fit_size(0.0, |_| false), 0.0);
    }
}
