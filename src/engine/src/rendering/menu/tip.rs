//! Where a loading screen's tip text goes: left-aligned in the lower part of the screen, wrapped
//! to a column, in the list's font and the HUD colour.

use super::layout::MenuLayout;
use crate::rendering::game_settings::SafeZone;
use crate::rendering::hud::field_text::{ScreenMap, TextItem};
use crate::rendering::ui_colour::UiTint;
use crate::ui_layout::scaleform_text::FieldGeometry;
use crate::ui_layout::values::Align;

/// Inferred: where the column starts, as a fraction of the visible height.
const TOP_FRACTION: f32 = 0.78;
/// Inferred: the column's width as a fraction of the visible width; it leaves the logo's corner
/// free.
const WIDTH_FRACTION: f32 = 0.6;

/// The tip `text` for a framebuffer of `fb` pixels, faded by `alpha` (0 to 1).
pub(super) fn item(
    layout: &MenuLayout,
    safe_zone: SafeZone,
    tint: &UiTint,
    fb: (u32, u32),
    text: &str,
    alpha: f32,
) -> TextItem {
    let fit = layout.fit(fb.0, fb.1);
    let [left, top, right, bottom] = fit.visible_stage_rect();
    let column_left = left + safe_zone.x;
    let column_top = top + (bottom - top) * TOP_FRACTION;
    let column_width = ((right - left) * WIDTH_FRACTION).max(1.0);
    let column_height = (bottom - safe_zone.y - column_top).max(0.0);
    let entry = &layout.list.entry;
    let geometry = FieldGeometry::content_box(
        (column_left, column_top),
        column_width,
        column_height,
        entry.text.size,
        Align::Left,
        entry.text.metrics,
    )
    .wrapping();
    let mut colour = tint.text_rgba();
    colour[3] = (f32::from(colour[3]) * alpha.clamp(0.0, 1.0)).round() as u8;
    let (origin_x, origin_y) = fit.stage_point(0.0, 0.0);
    TextItem {
        text: text.to_owned(),
        family: entry.font_family.clone(),
        geometry,
        map: ScreenMap {
            origin: (origin_x, origin_y),
            scale: fit.scale(),
        },
        shrink: false,
        colour,
        shadow: None,
        wrap_width: Some(fit.s(column_width)),
    }
}
