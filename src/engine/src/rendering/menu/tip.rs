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
    let whole_lines = (column_height / geometry.line_pitch()).floor().max(1.0);
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
        // Half a line short of the last whole line, so no rounding shows a line past the safe zone.
        max_height: Some(fit.s((whole_lines - 0.5) * geometry.line_pitch())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::menu::layout::test_support::standard;

    const SAFE_ZONE: SafeZone = SafeZone { x: 64.0, y: 36.0 };
    const TINT: UiTint = UiTint {
        color: [0.0, 1.0, 0.0],
        opacity: 1.0,
    };

    fn tip(fb: (u32, u32)) -> (TextItem, f32) {
        let layout = standard();
        let item = item(&layout, SAFE_ZONE, &TINT, fb, "A tip", 1.0);
        let bottom_limit = layout.fit(fb.0, fb.1).visible_stage_rect()[3] - SAFE_ZONE.y;
        (item, bottom_limit)
    }

    #[test]
    fn the_tip_column_ends_above_the_bottom_safe_zone_on_every_screen_shape() {
        for fb in [(1280, 720), (640, 360), (1000, 1000), (3440, 1440)] {
            let (item, bottom_limit) = tip(fb);
            let scale = item.map.scale;
            let top = item.geometry.top + item.geometry.vertical_inset;
            let height = item.max_height.expect("the tip is bounded") / scale;
            assert!(
                top + height <= bottom_limit,
                "{fb:?}: the column ends at {} below the limit {bottom_limit}",
                top + height
            );
            assert!(height > 0.0, "{fb:?}: at least one line fits");
        }
    }

    #[test]
    fn the_tip_fades_by_scaling_the_hud_colour_alpha() {
        let layout = standard();
        let half = item(&layout, SAFE_ZONE, &TINT, (640, 360), "A tip", 0.5);
        assert_eq!(half.colour, [0, 255, 0, 128]);
    }
}
