//! Text of the HUD and the menus: single-line text fields laid out by the Scaleform rules of
//! [`crate::ui_layout::scaleform_text`] (field box, gutter, baseline, alignment, shrink-to-fit) and
//! drawn with glyphon. The field geometry is in stage pixels; a [`ScreenMap`] puts it on the screen.
//!
//! Every Scaleform geometry call of the HUD and the menus is in this file.

use std::cell::RefCell;

use glyphon::cosmic_text::Align as LineAlign;
use glyphon::{
    Attrs, Buffer, Cache, Color, ColorMode, Family, FontSystem, Metrics, Resolution, Shaping,
    SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};

use crate::rendering::ui_fonts::UiFonts;
use crate::ui_layout::scaleform_text::{FieldGeometry, VerticalAlign, fit_size};
use crate::ui_layout::values::Align;

/// Stage pixels to framebuffer pixels: `screen = origin + stage * scale`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenMap {
    pub origin: (f32, f32),
    pub scale: f32,
}

impl ScreenMap {
    pub fn point(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.origin.0 + x * self.scale,
            self.origin.1 + y * self.scale,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    /// Offset down and to the right, in framebuffer pixels.
    pub offset: f32,
    pub colour: [u8; 4],
}

/// One text field to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct TextItem {
    pub text: String,
    pub family: String,
    /// The field in stage pixels; its text is a single line.
    pub geometry: FieldGeometry,
    pub map: ScreenMap,
    /// The text shrinks until it fits the field's width (shrink auto-size).
    pub shrink: bool,
    pub colour: [u8; 4],
    pub shadow: Option<Shadow>,
    /// The width in framebuffer pixels the text wraps at, or `None` for a single line. Centred
    /// text is centred line by line within it.
    pub wrap_width: Option<f32>,
    /// The height in framebuffer pixels below which no line is drawn, or `None` for no limit.
    pub max_height: Option<f32>,
}

/// The framebuffer rectangle a text item's lines cover: the left edge of its text area and the
/// top of its first line, the width of its longest line and the height of all its lines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextExtent {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

struct Placed {
    index: usize,
    left: f32,
    top: f32,
    colour: [u8; 4],
}

pub struct FieldText {
    font_system: FontSystem,
    swash: SwashCache,
    atlas: TextAtlas,
    viewport: Viewport,
    renderer: TextRenderer,
    buffers: Vec<Buffer>,
    prepared: Vec<TextItem>,
    prepared_for: (u32, u32),
    stale: bool,
    measured: Option<(TextItem, TextExtent)>,
}

impl FieldText {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        fonts: &UiFonts,
        colour_mode: ColorMode,
    ) -> Self {
        let cache = Cache::new(device);
        let viewport = Viewport::new(device, &cache);
        let mut atlas = TextAtlas::with_color_mode(device, queue, &cache, format, colour_mode);
        let renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        Self {
            font_system: fonts.font_system(),
            swash: SwashCache::new(),
            atlas,
            viewport,
            renderer,
            buffers: Vec::new(),
            prepared: Vec::new(),
            prepared_for: (0, 0),
            stale: true,
            measured: None,
        }
    }

    /// Where the lines of `item` land, as [`Self::prepare`] would place them. The last
    /// measurement is kept while the item stays the same.
    pub fn measure(&mut self, item: &TextItem) -> TextExtent {
        if let Some((measured, extent)) = &self.measured
            && measured == item
        {
            return *extent;
        }
        let extent = extent_of(&mut self.font_system, item);
        self.measured = Some((item.clone(), extent));
        extent
    }

    /// Shapes `items` and uploads their glyphs for a framebuffer of `fb` pixels. Nothing is redone
    /// when the items and the framebuffer are those of the last successful call. Call before the
    /// render pass that [`Self::render_into`] draws in.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        fb: (u32, u32),
        items: &[TextItem],
    ) {
        if !self.stale && self.prepared_for == fb && self.prepared == items {
            return;
        }
        self.viewport.update(
            queue,
            Resolution {
                width: fb.0,
                height: fb.1,
            },
        );
        while self.buffers.len() < items.len() {
            let buffer = Buffer::new(&mut self.font_system, Metrics::new(1.0, 1.0));
            self.buffers.push(buffer);
        }
        let mut placed = Vec::with_capacity(items.len() * 2);
        for (index, item) in items.iter().enumerate() {
            let (left, top) = shape(&mut self.font_system, &mut self.buffers[index], item);
            if let Some(shadow) = item.shadow {
                placed.push(Placed {
                    index,
                    left: left + shadow.offset,
                    top: top + shadow.offset,
                    colour: shadow.colour,
                });
            }
            placed.push(Placed {
                index,
                left,
                top,
                colour: item.colour,
            });
        }

        let bounds = TextBounds {
            left: 0,
            top: 0,
            right: fb.0 as i32,
            bottom: fb.1 as i32,
        };
        let areas = placed.iter().map(|place| {
            let [r, g, b, a] = place.colour;
            TextArea {
                buffer: &self.buffers[place.index],
                left: place.left,
                top: place.top,
                scale: 1.0,
                bounds,
                default_color: Color::rgba(r, g, b, a),
                custom_glyphs: &[],
            }
        });
        match self.renderer.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            areas,
            &mut self.swash,
        ) {
            Ok(()) => {
                self.stale = false;
                self.prepared_for = fb;
                self.prepared.clear();
                self.prepared.extend_from_slice(items);
            }
            Err(error) => {
                self.stale = true;
                tracing::warn!("UI text prepare failed (will retry): {error}");
            }
        }
    }

    /// Draws what [`Self::prepare`] uploaded.
    pub fn render_into(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Err(error) = self.renderer.render(&self.atlas, &self.viewport, pass) {
            tracing::warn!("UI text render failed: {error}");
        }
    }
}

/// The width of the first line in `buffer` after shaping `item.text` at `size` stage pixels,
/// the first baseline's offset below the buffer's top (both in framebuffer pixels) and the number
/// of lines.
fn shape_at(
    font_system: &mut FontSystem,
    buffer: &mut Buffer,
    item: &TextItem,
    geometry: &FieldGeometry,
) -> (f32, f32, usize) {
    let scale = item.map.scale;
    let size = geometry.size * scale;
    buffer.set_metrics(Metrics::new(size, geometry.line_pitch() * scale));
    buffer.set_size(item.wrap_width, item.max_height);
    // Stand-in S2: the face is chosen by family name only. The font configuration's style
    // (Normal / Bold) is not applied as a weight; the bold classes map to their own family.
    let attrs = Attrs::new()
        .family(Family::Name(&item.family))
        .letter_spacing(geometry.letter_spacing / geometry.size.max(f32::MIN_POSITIVE));
    buffer.set_text(
        &item.text,
        &attrs,
        Shaping::Advanced,
        line_alignment(item, geometry),
    );
    buffer.shape_until_scroll(font_system, false);
    let first = buffer
        .layout_runs()
        .next()
        .map_or((0.0, 0.0), |run| (run.line_w, run.line_y));
    (first.0, first.1, buffer.layout_runs().count())
}

/// Wrapped centred text is centred line by line inside the wrap width; every other text is placed
/// by its first line alone.
fn is_centred_block(item: &TextItem, geometry: &FieldGeometry) -> bool {
    item.wrap_width.is_some() && geometry.align == Align::Center
}

fn line_alignment(item: &TextItem, geometry: &FieldGeometry) -> Option<LineAlign> {
    is_centred_block(item, geometry).then_some(LineAlign::Center)
}

/// How far down, in stage pixels, a text block of `lines` lines sits below the top of the field's
/// content box. Text taller than the field keeps its top at the field's top.
fn block_offset(geometry: &FieldGeometry, lines: usize) -> f32 {
    match geometry.valign {
        VerticalAlign::Top => 0.0,
        VerticalAlign::Bottom => {
            (geometry.content_height() - lines as f32 * geometry.line_pitch()).max(0.0)
        }
    }
}

/// Shapes `item` into `buffer` at its final size and returns the framebuffer position of the
/// buffer's top-left corner that puts the text where the field's rules want it.
fn shape(font_system: &mut FontSystem, buffer: &mut Buffer, item: &TextItem) -> (f32, f32) {
    let scale = item.map.scale;
    let mut geometry = item.geometry;
    if item.shrink {
        let content_width = geometry.content_width();
        let shaping = RefCell::new((&mut *font_system, &mut *buffer));
        let size = fit_size(geometry.size, |candidate| {
            let (font_system, buffer) = &mut *shaping.borrow_mut();
            let trial = geometry.with_size(candidate);
            shape_at(font_system, buffer, item, &trial).0 / scale <= content_width
        });
        geometry = geometry.with_size(size);
    }
    let (width, baseline, lines) = shape_at(font_system, buffer, item, &geometry);
    let (anchor_x, _) = item.map.point(geometry.anchor_x(), 0.0);
    let first_baseline = geometry.first_baseline() + block_offset(&geometry, lines);
    let (_, baseline_y) = item.map.point(0.0, first_baseline);
    let text_width = match item.wrap_width {
        Some(wrap_width) if is_centred_block(item, &geometry) => wrap_width,
        _ => width,
    };
    let left = match geometry.align {
        Align::Left | Align::Justify => anchor_x,
        Align::Center => anchor_x - text_width / 2.0,
        Align::Right => anchor_x - width,
    };
    (left, baseline_y - baseline)
}

/// The rectangle the lines of `item` cover once shaped.
pub(super) fn extent_of(font_system: &mut FontSystem, item: &TextItem) -> TextExtent {
    let mut buffer = Buffer::new(font_system, Metrics::new(1.0, 1.0));
    let (left, top) = shape(font_system, &mut buffer, item);
    let (width, height) = buffer
        .layout_runs()
        .fold((0.0_f32, 0.0_f32), |(width, height), run| {
            (
                width.max(run.line_w),
                height.max(run.line_top + run.line_height),
            )
        });
    TextExtent {
        left,
        top,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::ui_fonts::test_support as fonts;
    use crate::ui_layout::scaleform_text::FontMetrics;

    #[test]
    fn the_screen_map_scales_then_shifts() {
        let map = ScreenMap {
            origin: (10.0, 20.0),
            scale: 2.0,
        };
        assert_eq!(map.point(3.0, 4.0), (16.0, 28.0));
    }

    const TIP: &str = "Wrapped text breaks into lines at the width of its column.";
    const COLUMN: f32 = 120.0;

    fn item(wrap_width: Option<f32>) -> TextItem {
        let metrics = FontMetrics {
            ascent: 0.9,
            descent: 0.2,
        };
        TextItem {
            text: TIP.to_owned(),
            family: fonts::family(),
            geometry: FieldGeometry::content_box(
                (0.0, 0.0),
                COLUMN,
                100.0,
                16.0,
                Align::Left,
                metrics,
            ),
            map: ScreenMap {
                origin: (0.0, 0.0),
                scale: 1.0,
            },
            shrink: false,
            colour: [255; 4],
            shadow: None,
            wrap_width,
            max_height: None,
        }
    }

    fn line_widths(item: &TextItem) -> Vec<f32> {
        let mut font_system = fonts::ui_fonts().font_system();
        let mut buffer = Buffer::new(&mut font_system, Metrics::new(1.0, 1.0));
        shape(&mut font_system, &mut buffer, item);
        buffer.layout_runs().map(|run| run.line_w).collect()
    }

    #[test]
    fn text_with_a_wrap_width_breaks_into_lines_that_fit_it() {
        let widths = line_widths(&item(Some(COLUMN)));
        assert!(widths.len() > 1, "{widths:?}");
        assert!(widths.iter().all(|&width| width <= COLUMN), "{widths:?}");
    }

    fn extent(item: &TextItem) -> TextExtent {
        extent_of(&mut fonts::ui_fonts().font_system(), item)
    }

    #[test]
    fn the_extent_of_wrapped_text_is_its_column_wide_at_most_and_all_its_lines_tall() {
        let wrapped = extent(&item(Some(COLUMN)));
        let single = extent(&item(None));

        assert!(
            wrapped.width <= COLUMN && wrapped.width > COLUMN / 2.0,
            "{wrapped:?}"
        );
        assert!(single.width > COLUMN, "{single:?}");
        let line = single.height;
        assert!(line > 0.0);
        assert!(
            wrapped.height >= line * 2.0 - 0.01,
            "several lines of {line} px: {wrapped:?}"
        );
        assert!(
            ((wrapped.height / line).round() * line - wrapped.height).abs() < 0.01,
            "whole lines: {wrapped:?}"
        );
    }

    #[test]
    fn the_extent_starts_where_the_field_puts_the_text() {
        let item = item(Some(COLUMN));
        let extent = extent(&item);
        assert!((extent.left - item.geometry.content_left()).abs() < 0.01);
        let baseline = item.geometry.first_baseline();
        assert!(
            extent.top < baseline && baseline < extent.top + extent.height,
            "the first baseline {baseline} lies in the first line of {extent:?}"
        );
    }

    fn laid_out(item: &TextItem) -> (Vec<f32>, Vec<(f32, f32)>) {
        let mut font_system = fonts::ui_fonts().font_system();
        let mut buffer = Buffer::new(&mut font_system, Metrics::new(1.0, 1.0));
        let (left, top) = shape(&mut font_system, &mut buffer, item);
        let runs: Vec<_> = buffer.layout_runs().collect();
        let baselines = runs.iter().map(|run| top + run.line_y).collect();
        let spans = runs
            .iter()
            .map(|run| {
                let first = run.glyphs.first().map_or(0.0, |glyph| glyph.x);
                (left + first, left + first + run.line_w)
            })
            .collect();
        (baselines, spans)
    }

    fn centred(mut item: TextItem) -> TextItem {
        item.geometry.align = Align::Center;
        item
    }

    #[test]
    fn wrapped_centred_text_is_centred_line_by_line_in_its_width() {
        let item = centred(item(Some(COLUMN)));
        let (_, spans) = laid_out(&item);
        assert!(spans.len() > 1, "{spans:?}");
        let widths: Vec<f32> = spans.iter().map(|(left, right)| right - left).collect();
        assert!(
            widths.iter().any(|width| (width - widths[0]).abs() > 1.0),
            "lines of different widths: {widths:?}"
        );
        for (left, right) in spans {
            let middle = f32::midpoint(left, right);
            assert!(
                (middle - item.geometry.anchor_x()).abs() < 0.5,
                "{middle} is not the field's middle {}",
                item.geometry.anchor_x()
            );
        }
    }

    fn bottom_anchored(text: &str, height: f32) -> TextItem {
        let mut item = item(Some(COLUMN));
        item.text = text.to_owned();
        item.geometry = FieldGeometry::content_box(
            (0.0, 0.0),
            COLUMN,
            height,
            16.0,
            Align::Left,
            item.geometry.metrics,
        )
        .with_valign(VerticalAlign::Bottom);
        item
    }

    fn expected_first_baseline(item: &TextItem, lines: usize) -> f32 {
        let geometry = &item.geometry;
        geometry.line_top() + geometry.content_height() - lines as f32 * geometry.line_pitch()
            + geometry.metrics.baseline_offset(geometry.size)
    }

    #[test]
    fn text_anchored_at_the_bottom_ends_one_line_pitch_per_line_above_the_field_bottom() {
        for (text, lines) in [("Short", 1_usize), ("First\nSecond", 2)] {
            let item = bottom_anchored(text, 100.0);
            let (baselines, _) = laid_out(&item);
            assert_eq!(baselines.len(), lines);
            let first = expected_first_baseline(&item, lines);
            for (index, baseline) in baselines.iter().enumerate() {
                let expected = first + index as f32 * item.geometry.line_pitch();
                assert!((baseline - expected).abs() < 0.01, "{baselines:?}");
            }
        }
    }

    #[test]
    fn text_taller_than_its_bottom_anchored_field_keeps_its_top_at_the_field_top() {
        let item = bottom_anchored("One\nTwo\nThree", 20.0);
        let (baselines, _) = laid_out(&item);
        assert!((baselines[0] - item.geometry.first_baseline()).abs() < 0.01);
    }

    #[test]
    fn top_anchored_text_starts_at_the_first_baseline_of_the_field() {
        let item = item(Some(COLUMN));
        let (baselines, _) = laid_out(&item);
        assert!((baselines[0] - item.geometry.first_baseline()).abs() < 0.01);
    }

    #[test]
    fn text_without_a_wrap_width_stays_on_one_line() {
        let widths = line_widths(&item(None));
        assert_eq!(widths.len(), 1);
        assert!(widths[0] > COLUMN, "{widths:?}");
    }
}
