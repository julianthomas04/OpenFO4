//! Text of the HUD and the menus: single-line text fields laid out by the Scaleform rules of
//! [`crate::ui_layout::scaleform_text`] (field box, gutter, baseline, alignment, shrink-to-fit) and
//! drawn with glyphon. The field geometry is in stage pixels; a [`ScreenMap`] puts it on the screen.
//!
//! Every Scaleform geometry call of the HUD and the menus is in this file.

use std::cell::RefCell;

use glyphon::{
    Attrs, Buffer, Cache, Color, ColorMode, Family, FontSystem, Metrics, Resolution, Shaping,
    SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};

use crate::rendering::ui_fonts::UiFonts;
use crate::ui_layout::scaleform_text::{FieldGeometry, fit_size};
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
        }
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

/// The width of the single line in `buffer` after shaping `item.text` at `size` stage pixels,
/// and the baseline's offset below the buffer's top, both in framebuffer pixels.
fn shape_at(
    font_system: &mut FontSystem,
    buffer: &mut Buffer,
    item: &TextItem,
    geometry: &FieldGeometry,
) -> (f32, f32) {
    let scale = item.map.scale;
    let size = geometry.size * scale;
    buffer.set_metrics(Metrics::new(size, geometry.line_pitch() * scale));
    buffer.set_size(None, None);
    // Stand-in S2: the face is chosen by family name only. The font configuration's style
    // (Normal / Bold) is not applied as a weight; the bold classes map to their own family.
    let attrs = Attrs::new()
        .family(Family::Name(&item.family))
        .letter_spacing(geometry.letter_spacing / geometry.size.max(f32::MIN_POSITIVE));
    buffer.set_text(&item.text, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);
    buffer
        .layout_runs()
        .next()
        .map_or((0.0, 0.0), |run| (run.line_w, run.line_y))
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
    let (width, baseline) = shape_at(font_system, buffer, item, &geometry);
    let (anchor_x, _) = item.map.point(geometry.anchor_x(), 0.0);
    let (_, baseline_y) = item.map.point(0.0, geometry.first_baseline());
    let left = match geometry.align {
        Align::Left | Align::Justify => anchor_x,
        Align::Center => anchor_x - width / 2.0,
        Align::Right => anchor_x - width,
    };
    (left, baseline_y - baseline)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_screen_map_scales_then_shifts() {
        let map = ScreenMap {
            origin: (10.0, 20.0),
            scale: 2.0,
        };
        assert_eq!(map.point(3.0, 4.0), (16.0, 28.0));
    }
}
