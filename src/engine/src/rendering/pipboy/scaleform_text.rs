//! Paints text the way the game's text fields lay it out: baseline at the font's ascent below the
//! line top, line pitch from the font's ascent and descent plus leading, a gutter, alignment
//! inside the content box, letter spacing, auto-size and shrink-to-fit. The arithmetic is in
//! [`crate::ui_layout::scaleform_text`]; this adapts it to egui.

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Color32, Galley, Painter, Pos2, Rect, Vec2};

use super::fonts::{Weight, font_id, weight_for_class};
use super::shapes::Shade;
use crate::ui_layout::scaleform_text::{AutoSize, FieldGeometry, FontMetrics, fit_size};
use crate::ui_layout::values::Align as FieldAlign;
use crate::ui_layout::{Declaration, KeyCheck, TextKey, UiLayout, UiLayoutError};

/// How a field sizes its text: unchanged, or shrunk to fit. The mode that grows a field to fit
/// its text is not drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAutoSize {
    None,
    Shrink,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldStyle {
    pub weight: Weight,
    pub colour: Color32,
    /// The field shrinks its text to fit (the game's shrink-to-fit text auto-size).
    pub shrink: bool,
    pub auto_size: Option<AutoSize>,
}

impl FieldStyle {
    pub fn new(weight: Weight, colour: Color32) -> Self {
        Self {
            weight,
            colour,
            shrink: false,
            auto_size: None,
        }
    }

    #[must_use]
    pub fn shrinking(self) -> Self {
        Self {
            shrink: true,
            ..self
        }
    }

    #[must_use]
    pub fn auto_sized(self, mode: AutoSize) -> Self {
        Self {
            auto_size: Some(mode),
            ..self
        }
    }
}

/// An authored text field resolved once: its geometry at the world placement, its face and its
/// text colour after the placement's colour transform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldSpec {
    pub geometry: FieldGeometry,
    pub weight: Weight,
    pub shade: Shade,
    pub auto_size: TextAutoSize,
    /// The Flash `autoSize` the menu's code sets on the field.
    pub field_auto_size: Option<AutoSize>,
}

impl FieldSpec {
    /// The field `key` with the text auto-size its menu's code sets on it.
    pub fn resolve(
        layout: &UiLayout,
        key: &TextKey,
        auto_size: TextAutoSize,
    ) -> Result<Self, UiLayoutError> {
        let font = layout.get(key)?.font;
        Self::resolve_in_font(layout, key, auto_size, &font)
    }

    /// The field `key` after its code sets the font class `font` on it.
    pub fn resolve_in_font(
        layout: &UiLayout,
        key: &TextKey,
        auto_size: TextAutoSize,
        font: &str,
    ) -> Result<Self, UiLayoutError> {
        let field = layout.get(key)?;
        let colour = field.color.ok_or_else(|| {
            UiLayoutError::unresolved(key.movie, key.path, "the text field names no text colour")
        })?;
        Ok(Self {
            geometry: FieldGeometry::of(&field, layout.font_metrics(font)?),
            weight: weight_for_class(layout, font)?,
            shade: Shade::of(colour, &field.placement.world.color),
            auto_size,
            field_auto_size: None,
        })
    }

    /// The field as the code sets it to grow with its text: `autoSize`, multi-line and word wrap.
    #[must_use]
    pub fn growing(&self, mode: AutoSize) -> Self {
        Self {
            geometry: self.geometry.wrapping(),
            field_auto_size: Some(mode),
            ..*self
        }
    }

    pub fn style(&self, colour: Color32) -> FieldStyle {
        let mut style = FieldStyle::new(self.weight, colour);
        if let Some(mode) = self.field_auto_size {
            style = style.auto_sized(mode);
        }
        match self.auto_size {
            TextAutoSize::Shrink => style.shrinking(),
            TextAutoSize::None => style,
        }
    }

    /// The same field moved by `offset` (a list row, a clip the menu repositions).
    #[must_use]
    pub fn translated(&self, offset: Vec2) -> Self {
        Self {
            geometry: self.geometry.translated(offset.x, offset.y),
            ..*self
        }
    }

    /// Draws `text` in the field's own colour under the Pip-Boy `tint`.
    pub fn paint(&self, painter: &Painter, text: &str, tint: Color32) -> PaintedField {
        self.paint_coloured(painter, text, self.shade.colour(tint))
    }

    /// Draws `text` in `colour`, for fields whose colour the menu's code sets.
    pub fn paint_coloured(&self, painter: &Painter, text: &str, colour: Color32) -> PaintedField {
        paint_field(painter, &self.geometry, text, self.style(colour))
    }
}

/// Boot check that a text field and the face of its font class resolve.
pub struct FieldCheck(pub TextKey);

impl KeyCheck for FieldCheck {
    fn failures(&self, layout: &UiLayout) -> Vec<UiLayoutError> {
        FieldSpec::resolve(layout, &self.0, TextAutoSize::None)
            .err()
            .into_iter()
            .collect()
    }

    fn declare(&self, out: &mut Vec<Declaration>) {
        self.0.declare(out);
    }
}

/// A one-line field the game lays out in code: content box from `origin`, `width` wide, in a
/// face with `metrics` (from [`UiLayout::font_metrics`]).
pub fn line_field(
    origin: Pos2,
    width: f32,
    size: f32,
    align: FieldAlign,
    metrics: FontMetrics,
) -> FieldGeometry {
    let height = metrics.line_pitch(size, 0.0);
    FieldGeometry::content_box((origin.x, origin.y), width, height, size, align, metrics)
}

/// A wrapping block the game lays out in code: content box from `origin`, `width` x `height`,
/// in a face with `metrics`.
pub fn block_field(
    origin: Pos2,
    width: f32,
    height: f32,
    size: f32,
    align: FieldAlign,
    metrics: FontMetrics,
) -> FieldGeometry {
    FieldGeometry::content_box((origin.x, origin.y), width, height, size, align, metrics).wrapping()
}

/// What was drawn: the field after auto-size, the font size after shrinking, the first baseline,
/// the text's bounding rect, where its first line starts and its glyph run from the first glyph's
/// left to the last glyph's advance end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintedField {
    pub geometry: FieldGeometry,
    pub size: f32,
    pub baseline: f32,
    pub rect: Rect,
    pub line_x: f32,
    pub run: (f32, f32),
}

fn horizontal(align: FieldAlign) -> Align {
    match align {
        FieldAlign::Left | FieldAlign::Justify => Align::Min,
        FieldAlign::Center => Align::Center,
        FieldAlign::Right => Align::Max,
    }
}

fn job(text: &str, geometry: &FieldGeometry, style: FieldStyle, wrap_width: f32) -> LayoutJob {
    let format = TextFormat {
        line_height: Some(geometry.line_pitch()),
        extra_letter_spacing: geometry.letter_spacing,
        ..TextFormat::simple(font_id(style.weight, geometry.size), style.colour)
    };
    let mut job = LayoutJob::single_section(text.to_owned(), format);
    job.sections[0].leading_space = geometry.indent;
    job.halign = horizontal(geometry.align);
    job.wrap.max_width = wrap_width;
    job
}

pub fn lay_out(
    painter: &Painter,
    text: &str,
    geometry: &FieldGeometry,
    style: FieldStyle,
) -> Arc<Galley> {
    let wrap_width = if geometry.wraps {
        geometry.content_width()
    } else {
        f32::INFINITY
    };
    painter.layout_job(job(text, geometry, style, wrap_width))
}

/// The width of the widest line of `text` at the geometry's size, unwrapped.
pub fn natural_width(
    painter: &Painter,
    text: &str,
    geometry: &FieldGeometry,
    style: FieldStyle,
) -> f32 {
    let single_line = FieldGeometry {
        wraps: false,
        align: FieldAlign::Left,
        ..*geometry
    };
    lay_out(painter, text, &single_line, style).rect.width()
}

/// The height `text` takes in `geometry`, wrapped when the field wraps.
pub fn text_height(
    painter: &Painter,
    text: &str,
    geometry: &FieldGeometry,
    style: FieldStyle,
) -> f32 {
    lay_out(painter, text, geometry, style).rect.height()
}

/// The number of lines `text` takes in `geometry`, wrapped when the field wraps.
pub fn line_count(
    painter: &Painter,
    text: &str,
    geometry: &FieldGeometry,
    style: FieldStyle,
) -> usize {
    lay_out(painter, text, geometry, style).rows.len()
}

fn fits(painter: &Painter, text: &str, geometry: &FieldGeometry, style: FieldStyle) -> bool {
    let galley = lay_out(painter, text, geometry, style);
    let height_fits = galley.rows.len() <= 1 || galley.rect.height() <= geometry.content_height();
    let width_fits =
        geometry.wraps || natural_width(painter, text, geometry, style) <= geometry.content_width();
    height_fits && width_fits
}

/// Draws `text` in the field `geometry` and returns what was drawn.
pub fn paint_field(
    painter: &Painter,
    geometry: &FieldGeometry,
    text: &str,
    style: FieldStyle,
) -> PaintedField {
    let mut geometry = *geometry;
    if let Some(mode) = style.auto_size {
        if geometry.wraps {
            let height = text_height(painter, text, &geometry, style);
            geometry = geometry.auto_height(height);
        } else {
            geometry = geometry.auto_sized(mode, natural_width(painter, text, &geometry, style));
        }
    }
    if style.shrink {
        let size = fit_size(geometry.size, |size| {
            fits(painter, text, &geometry.with_size(size), style)
        });
        geometry = geometry.with_size(size);
    }

    let galley = lay_out(painter, text, &geometry, style);
    let baseline = geometry.first_baseline();
    let anchor_x = geometry.anchor_x();
    let Some(first_row) = galley.rows.first() else {
        return empty(geometry, anchor_x, baseline);
    };
    let (Some(first), Some(last)) = (first_row.row.glyphs.first(), first_row.row.glyphs.last())
    else {
        return empty(geometry, anchor_x, baseline);
    };
    let shift = geometry.metrics.baseline_offset(geometry.size) - first.pos.y;
    let pos = Pos2::new(anchor_x, geometry.line_top() + shift);
    let rect = galley.rect.translate(pos.to_vec2());
    let row_x = pos.x + first_row.pos.x;
    let run = (row_x + first.pos.x, row_x + last.pos.x + last.advance_width);
    painter.galley(pos, galley, style.colour);
    PaintedField {
        geometry,
        size: geometry.size,
        baseline,
        rect,
        line_x: row_x,
        run,
    }
}

fn empty(geometry: FieldGeometry, anchor_x: f32, baseline: f32) -> PaintedField {
    PaintedField {
        geometry,
        size: geometry.size,
        baseline,
        rect: Rect::from_min_size(Pos2::new(anchor_x, geometry.line_top()), Vec2::ZERO),
        line_x: anchor_x,
        run: (anchor_x, anchor_x),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_layout::test_support::{assert_close, text_field};
    use egui::{Context, Id, LayerId, Order, RawInput};

    use crate::ui_layout::test_support::FIXTURE_METRICS;
    const METRICS: FontMetrics = FIXTURE_METRICS;

    fn context() -> Context {
        let defaults = egui::FontDefinitions::default();
        let bytes = defaults.font_data["Ubuntu-Light"].font.to_vec();
        let ctx = Context::default();
        ctx.set_fonts(super::super::fonts::PipBoyFonts::new(bytes.clone(), bytes).definitions());
        ctx
    }

    fn paint<T>(build: impl Fn(&Painter) -> T) -> (T, Vec<egui::epaint::ClippedShape>) {
        let ctx = context();
        let mut painted = None;
        let mut output = ctx.run_ui(RawInput::default(), |_| {
            let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("t")));
            painted = Some(build(&painter));
        });
        output.textures_delta.clear();
        (painted.unwrap(), output.shapes)
    }

    fn geometry(width: f32, size: f32) -> FieldGeometry {
        FieldGeometry::of(&text_field([-2.0, -2.0, width + 2.0, 60.0], size), METRICS)
    }

    fn style() -> FieldStyle {
        FieldStyle::new(Weight::Regular, Color32::WHITE)
    }

    /// The baseline of the first glyph egui placed, in painter space.
    fn painted_baseline(shapes: &[egui::epaint::ClippedShape]) -> f32 {
        shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => {
                    let row = text.galley.rows.first()?;
                    let glyph = row.row.glyphs.first()?;
                    Some(text.pos.y + row.pos.y + glyph.pos.y)
                }
                _ => None,
            })
            .expect("a text shape was painted")
    }

    #[test]
    fn the_painted_baseline_sits_the_fonts_ascent_below_the_line_top() {
        for size in [20.0, 28.0, 36.0] {
            let (_, shapes) = paint(|p| paint_field(p, &geometry(300.0, size), "Hxg", style()));
            assert!((painted_baseline(&shapes) - METRICS.ascent * size).abs() < 1e-3);
        }
    }

    #[test]
    fn right_alignment_ends_at_the_content_edge() {
        let mut field = text_field([-2.0, -2.0, 202.0, 60.0], 24.0);
        field.align = FieldAlign::Right;
        let g = FieldGeometry::of(&field, METRICS);
        let (painted, _) = paint(|p| paint_field(p, &g, "ab", style()));
        assert!((painted.rect.max.x - 200.0).abs() < 1e-3);
    }

    #[test]
    fn centre_alignment_is_centred_in_the_content_box() {
        let mut field = text_field([-2.0, -2.0, 202.0, 60.0], 24.0);
        field.align = FieldAlign::Center;
        let g = FieldGeometry::of(&field, METRICS);
        let (painted, _) = paint(|p| paint_field(p, &g, "ab", style()));
        assert!((painted.rect.center().x - 100.0).abs() < 1e-3);
    }

    #[test]
    fn the_glyph_run_spans_the_first_glyph_to_the_last_advance() {
        let (painted, _) = paint(|p| paint_field(p, &geometry(300.0, 24.0), "abc", style()));
        let (left, right) = painted.run;
        assert_close(left, painted.rect.min.x);
        assert!(right > left);
    }

    #[test]
    fn text_wider_than_the_field_shrinks_to_fit() {
        let long = "A long line of text that cannot fit";
        let narrow = geometry(100.0, 30.0);
        let (painted, _) = paint(|p| paint_field(p, &narrow, long, style().shrinking()));
        assert!(painted.size < 30.0);
        let (width, _) = paint(|p| natural_width(p, long, &painted.geometry, style()));
        assert!(width <= 100.0);
        let (unshrunk, _) = paint(|p| paint_field(p, &narrow, long, style()));
        assert_close(unshrunk.size, 30.0);
    }

    #[test]
    fn explicit_lines_taller_than_the_field_shrink_to_fit_its_height() {
        let short =
            FieldGeometry::content_box((0.0, 0.0), 300.0, 40.0, 30.0, FieldAlign::Left, METRICS);
        let text = "one\ntwo\nthree";
        let (painted, _) = paint(|p| paint_field(p, &short, text, style().shrinking()));
        let (height, _) = paint(|p| text_height(p, text, &painted.geometry, style()));
        assert!(height <= 40.0);
    }

    #[test]
    fn auto_size_left_makes_the_field_the_text_plus_gutters() {
        let g = geometry(300.0, 24.0);
        let (painted, _) = paint(|p| paint_field(p, &g, "abc", style().auto_sized(AutoSize::Left)));
        let text_width = painted.geometry.content_width();
        assert!(text_width < 300.0);
        assert_close(painted.geometry.left, g.left);
        assert!((painted.geometry.right - painted.geometry.left - text_width - 4.0).abs() < 1e-3);
    }

    #[test]
    fn auto_size_on_a_wrapping_field_keeps_the_width_and_grows_the_height() {
        let g = geometry(60.0, 20.0).wrapping();
        let text = "aaaa bbbb cccc dddd eeee";
        let (painted, _) = paint(|p| paint_field(p, &g, text, style().auto_sized(AutoSize::Left)));
        assert_close(painted.geometry.left, g.left);
        assert_close(painted.geometry.right, g.right);
        assert!(painted.geometry.bottom > g.top + 2.0 * METRICS.line_pitch(20.0, 0.0));
    }

    #[test]
    fn leading_adds_to_the_pitch_of_wrapped_lines() {
        let mut field = text_field([-2.0, -2.0, 62.0, 200.0], 20.0);
        field.word_wrap = true;
        field.multiline = true;
        field.leading = 6.0;
        let g = FieldGeometry::of(&field, METRICS);
        let (pitch, _) = paint(|painter| {
            let galley = lay_out(painter, "aaaa bbbb cccc dddd", &g, style());
            galley.rows[1].pos.y - galley.rows[0].pos.y
        });
        assert!((pitch - METRICS.line_pitch(20.0, 6.0)).abs() <= 1.0);
    }

    #[test]
    fn empty_text_paints_nothing() {
        let (painted, shapes) = paint(|p| paint_field(p, &geometry(100.0, 20.0), "", style()));
        assert_close(painted.rect.width(), 0.0);
        assert_eq!(shapes, []);
    }

    #[test]
    fn a_code_laid_line_puts_its_content_box_at_the_origin() {
        let g = line_field(
            Pos2::new(10.0, 20.0),
            100.0,
            28.0,
            FieldAlign::Right,
            METRICS,
        );
        assert_close(g.content_left(), 10.0);
        assert_close(g.content_right(), 110.0);
        assert_close(g.line_top(), 20.0);
        assert!(block_field(Pos2::ZERO, 10.0, 10.0, 20.0, FieldAlign::Left, METRICS).wraps);
    }
}
