//! The engine's version and build in the bottom-left corner, over every other screen: main menu, loading
//! overlay, reveal, HUD and Pip-Boy. It is pure white, outside the HUD and Pip-Boy colours and
//! the HUD opacity, in the HUD's bold face at a size and corner inset the user chose.

use crate::rendering::hud::field_text::{FieldText, ScreenMap, TextItem};
use crate::rendering::hud::layout::Layout;
use crate::rendering::ui_fonts::{UiFontError, UiFonts};
use crate::ui_layout::faces::BOLD_FACE;
use crate::ui_layout::scaleform_text::{FieldGeometry, FontMetrics};
use crate::ui_layout::values::Align;
use crate::ui_layout::{UiLayout, UiLayoutError};
use crate::version;

const WHITE: [u8; 4] = [255, 255, 255, 255];

/// Chosen by the user, not taken from the game; in stage pixels.
pub(crate) const TEXT_SIZE: f32 = 12.8;
/// Chosen by the user, not taken from the game; in stage pixels.
const CORNER_INSET: f32 = 16.0;
/// The lines shown, the version above the build.
const ROWS: usize = 2;

#[derive(Debug, thiserror::Error)]
pub enum VersionOverlayError {
    #[error(transparent)]
    Fonts(#[from] UiFontError),
    #[error(transparent)]
    Layout(#[from] UiLayoutError),
}

pub struct VersionOverlay {
    text: FieldText,
    stage: (f32, f32),
    metrics: FontMetrics,
    labels: [String; ROWS],
    cached: Option<((u32, u32), [TextItem; ROWS])>,
}

impl VersionOverlay {
    /// An overlay for a surface of `format`, scaled like the HUD movie's `stage`. Fails when the
    /// bold face is missing from `fonts` or the layout.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        fonts: &UiFonts,
        layout: &UiLayout,
        stage: (f32, f32),
    ) -> Result<Self, VersionOverlayError> {
        fonts.require_family(BOLD_FACE)?;
        Ok(Self {
            text: FieldText::new(device, queue, format, fonts, glyphon::ColorMode::Accurate),
            stage,
            metrics: layout.family_metrics(BOLD_FACE)?,
            labels: version::overlay_lines(),
            cached: None,
        })
    }

    /// Draws the version over `view`, a surface view of `fb` pixels, in a pass of its own.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        fb: (u32, u32),
    ) {
        let items = match &self.cached {
            Some((cached_fb, items)) if *cached_fb == fb => items,
            _ => &self.cached.insert((fb, self.items(fb))).1,
        };
        self.text.prepare(device, queue, fb, items);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Version Overlay Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        self.text.render_into(&mut pass);
    }

    fn items(&self, fb: (u32, u32)) -> [TextItem; ROWS] {
        std::array::from_fn(|index| self.item(fb, ROWS - 1 - index, &self.labels[index]))
    }

    fn item(&self, fb: (u32, u32), row: usize, text: &str) -> TextItem {
        let line = CornerLine::new(self.stage, fb, self.metrics, row);
        TextItem {
            text: text.to_owned(),
            family: BOLD_FACE.to_owned(),
            geometry: line.geometry(self.metrics),
            map: ScreenMap {
                origin: (0.0, 0.0),
                scale: line.scale,
            },
            shrink: false,
            colour: WHITE,
            shadow: None,
        }
    }
}

fn scale_for(stage: (f32, f32), fb: (u32, u32)) -> f32 {
    Layout::for_stage(stage.0, stage.1, fb.0 as f32, fb.1 as f32).scale()
}

/// The line box of a row counted from the bottom (0 is the bottom line): left-aligned,
/// `CORNER_INSET` from the left and bottom screen edges.
struct CornerLine {
    scale: f32,
    /// Stage pixels.
    left: f32,
    top: f32,
    height: f32,
    screen_width: f32,
}

impl CornerLine {
    fn new(stage: (f32, f32), fb: (u32, u32), metrics: FontMetrics, row: usize) -> Self {
        let scale = scale_for(stage, fb);
        let height = metrics.line_pitch(TEXT_SIZE, 0.0);
        Self {
            scale,
            left: CORNER_INSET,
            top: fb.1 as f32 / scale - CORNER_INSET - height * (row + 1) as f32,
            height,
            screen_width: fb.0 as f32 / scale,
        }
    }

    fn geometry(&self, metrics: FontMetrics) -> FieldGeometry {
        FieldGeometry::content_box(
            (self.left, self.top),
            self.screen_width - self.left,
            self.height,
            TEXT_SIZE,
            Align::Left,
            metrics,
        )
    }

    /// The line box in framebuffer pixels for a text of `width` pixels.
    #[cfg(test)]
    fn rect(&self, width: f32) -> crate::rendering::hud::quad::PxRect {
        crate::rendering::hud::quad::PxRect {
            x: self.left * self.scale,
            y: self.top * self.scale,
            w: width,
            h: self.height * self.scale,
        }
    }
}

/// The framebuffer rectangle the whole block of lines occupies, its widest line `width` pixels.
#[cfg(test)]
pub(crate) fn corner_rect(
    stage: (f32, f32),
    fb: (u32, u32),
    metrics: FontMetrics,
    width: f32,
) -> crate::rendering::hud::quad::PxRect {
    let bottom = CornerLine::new(stage, fb, metrics, 0).rect(width);
    let top = CornerLine::new(stage, fb, metrics, ROWS - 1).rect(width);
    crate::rendering::hud::quad::PxRect {
        h: bottom.y + bottom.h - top.y,
        ..top
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the arithmetic below stays on exactly representable values"
)]
mod tests {
    use super::*;

    const METRICS: FontMetrics = FontMetrics {
        ascent: 0.75,
        descent: 0.25,
    };
    const STAGE: (f32, f32) = (1280.0, 720.0);

    #[test]
    fn the_line_sits_the_inset_in_from_the_bottom_left_corner() {
        let rect = corner_rect(STAGE, (1920, 1080), METRICS, 100.0);
        assert_eq!(rect.x, 24.0);
        assert_eq!(rect.y + rect.h, 1080.0 - 24.0);
        assert!((rect.h - ROWS as f32 * TEXT_SIZE * 1.5).abs() < 1e-3);
    }

    #[test]
    fn the_version_line_is_one_line_height_above_the_build_line() {
        let build = CornerLine::new(STAGE, (1920, 1080), METRICS, 0);
        let version = CornerLine::new(STAGE, (1920, 1080), METRICS, 1);
        assert_eq!(version.left, build.left);
        assert_eq!(version.top + version.height, build.top);
    }

    #[test]
    fn the_line_scales_with_the_screen() {
        let small = corner_rect(STAGE, (1280, 720), METRICS, 100.0);
        let large = corner_rect(STAGE, (2560, 1440), METRICS, 100.0);
        assert_eq!(large.x, 2.0 * small.x);
        assert_eq!(large.h, 2.0 * small.h);
        assert_eq!(large.y + large.h, 1440.0 - 32.0);
    }

    #[test]
    fn on_a_wider_screen_the_line_stays_at_the_screen_corner() {
        let rect = corner_rect(STAGE, (2560, 1080), METRICS, 100.0);
        assert_eq!(rect.x, 24.0);
        assert_eq!(rect.y + rect.h, 1080.0 - 24.0);
    }

    #[test]
    fn the_geometry_starts_where_the_rectangle_does() {
        let line = CornerLine::new(STAGE, (2560, 1080), METRICS, 0);
        let geometry = line.geometry(METRICS);
        assert_eq!(geometry.content_left(), line.left);
        assert!((geometry.top + geometry.vertical_inset - line.top).abs() < 1e-3);
    }
}
