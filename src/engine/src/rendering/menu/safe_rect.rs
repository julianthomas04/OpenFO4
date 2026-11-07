//! The loading screen's script locks clips to the visible rectangle's safe zone with
//! `GlobalFunc.LockToSafeRect`: it moves a clip's registration point to the anchor, and the clip's
//! contents follow. The anchors the screen uses are the left edge, which moves a clip sideways
//! only, and the bottom-right corner.

use crate::rendering::game_settings::SafeZone;
use crate::rendering::hud::layout::Layout;
use crate::ui_layout::{PlacementKey, UiLayout, UiLayoutError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    Left,
    BottomRight,
}

impl Anchor {
    /// The stage offset that puts a clip's registration point, authored at `origin`, on this
    /// anchor of the safe zone inside the visible rectangle of the stage fitted by `fit`.
    pub fn shift(self, fit: &Layout, safe_zone: SafeZone, origin: (f32, f32)) -> (f32, f32) {
        let [left, _, right, bottom] = fit.visible_stage_rect();
        match self {
            Self::Left => (left + safe_zone.x - origin.0, 0.0),
            Self::BottomRight => (
                right - safe_zone.x - origin.0,
                bottom - safe_zone.y - origin.1,
            ),
        }
    }
}

/// The registration point of the clip at `key` on the stage as authored. The clip must be placed
/// by a plain offset, so that moving its registration point moves its contents alone.
pub fn plain_origin(layout: &UiLayout, key: &PlacementKey) -> Result<(f32, f32), UiLayoutError> {
    let matrix = layout.get(key)?.world.matrix;
    if (matrix.a, matrix.b, matrix.c, matrix.d) != (1.0, 0.0, 0.0, 1.0) {
        return Err(UiLayoutError::unresolved(
            key.movie,
            key.path,
            "the clip is scaled, rotated or skewed",
        ));
    }
    Ok((matrix.tx, matrix.ty))
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the layout arithmetic below stays on exactly representable values"
)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::ui_layout::test_support::{font_config_json, instance, movie_json, translate_json};

    const SAFE: SafeZone = SafeZone { x: 50.0, y: 36.0 };
    const ORIGIN: (f32, f32) = (139.0, 506.0);
    static CLIP: PlacementKey = PlacementKey::new("Screen", "Menu_mc/Clip");

    fn layout_with_clip(matrix: [f64; 6]) -> UiLayout {
        let mut clip = instance(1, "sprite", 0.0, 0.0);
        clip["world"]["matrix"] = json!(matrix);
        let parts = json!({ "instances": { "Menu_mc/Clip": clip } });
        UiLayout::from_documents(
            &translate_json(),
            &font_config_json(),
            &[("Screen", &movie_json("Screen", &parts))],
        )
        .unwrap()
    }

    #[test]
    fn the_origin_of_a_clip_placed_by_a_plain_offset_is_that_offset() {
        let layout = layout_with_clip([1.0, 0.0, 0.0, 1.0, 139.0, 506.65]);
        assert_eq!(plain_origin(&layout, &CLIP).unwrap(), (139.0, 506.65));
    }

    #[test]
    fn a_scaled_clip_has_no_plain_origin() {
        let layout = layout_with_clip([2.0, 0.0, 0.0, 1.0, 139.0, 506.65]);
        let error = plain_origin(&layout, &CLIP).unwrap_err();
        assert!(error.to_string().contains("Menu_mc/Clip"), "{error}");
    }

    #[test]
    fn the_left_anchor_moves_a_clip_sideways_to_the_safe_zones_left_edge() {
        let fit = Layout::for_stage(1280.0, 720.0, 1280.0, 720.0);
        assert_eq!(Anchor::Left.shift(&fit, SAFE, ORIGIN), (50.0 - 139.0, 0.0));
    }

    #[test]
    fn the_left_anchor_follows_the_visible_rectangle_of_a_wider_screen() {
        let fit = Layout::for_stage(1280.0, 720.0, 2560.0, 1080.0);
        let [left, ..] = fit.visible_stage_rect();
        assert!(left < 0.0);
        assert_eq!(
            Anchor::Left.shift(&fit, SAFE, ORIGIN).0,
            left + 50.0 - 139.0
        );
    }

    #[test]
    fn the_bottom_right_anchor_puts_the_registration_point_inside_the_safe_zone() {
        let fit = Layout::for_stage(1280.0, 720.0, 1280.0, 720.0);
        let (x, y) = Anchor::BottomRight.shift(&fit, SAFE, ORIGIN);
        assert_eq!((ORIGIN.0 + x, ORIGIN.1 + y), (1280.0 - 50.0, 720.0 - 36.0));
    }
}
