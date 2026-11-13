//! The bracket a HUD component draws around its content: one vertical bar on each side with a hook
//! at each end pointing inward (the "horizontal" bracket style, the only one the HUD draws). Its
//! size comes from the literals the movie sets on the component; the colour is the HUD white of the
//! movie's `lineStyle` (the tint turns it into the HUD colour).

use super::layout::Layout;
use super::quad::{PxRect, QuadBuilder};
use crate::ui_layout::{LiteralKey, UiLayout, UiLayoutError, key_check_fields};

const HORIZONTAL_STYLE: &str = "horizontal";
const LINE_WHITE: [f32; 3] = [1.0; 3];

pub(super) struct BracketKeys {
    pub(super) style: LiteralKey,
    pub(super) shows: LiteralKey,
    pub(super) corner: LiteralKey,
    pub(super) line: LiteralKey,
    pub(super) pad_x: LiteralKey,
    pub(super) pad_y: LiteralKey,
}

key_check_fields!(BracketKeys {
    style,
    shows,
    corner,
    line,
    pad_x,
    pad_y
});

/// The bracket keys of the component whose setting literals start with `$prefix`.
macro_rules! bracket_keys {
    ($movie:expr, $prefix:expr) => {
        $crate::rendering::hud::bracket::BracketKeys {
            style: $crate::ui_layout::LiteralKey::texts(
                $movie,
                concat!($prefix, ".BracketStyle="),
                1,
            ),
            shows: $crate::ui_layout::LiteralKey::flags(
                $movie,
                concat!($prefix, ".bShowBrackets="),
                1,
            ),
            corner: $crate::ui_layout::LiteralKey::new(
                $movie,
                concat!($prefix, ".bracketCornerLength="),
                1,
            ),
            line: $crate::ui_layout::LiteralKey::new(
                $movie,
                concat!($prefix, ".bracketLineWidth="),
                1,
            ),
            pad_x: $crate::ui_layout::LiteralKey::new(
                $movie,
                concat!($prefix, ".bracketPaddingX="),
                1,
            ),
            pad_y: $crate::ui_layout::LiteralKey::new(
                $movie,
                concat!($prefix, ".bracketPaddingY="),
                1,
            ),
        }
    };
}
pub(super) use bracket_keys;

/// A bracket's measures, in stage pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BracketSpec {
    corner: f32,
    line: f32,
    pad_x: f32,
    pad_y: f32,
}

impl BracketSpec {
    /// `None` when the component turns brackets off; an error naming the literal when the style is
    /// not the horizontal one or a measure is not a usable length.
    pub(super) fn resolve(
        layout: &UiLayout,
        keys: &BracketKeys,
    ) -> Result<Option<Self>, UiLayoutError> {
        if !layout.lit_flag(&keys.shows, 0)? {
            return Ok(None);
        }
        let style = layout.lit_string(&keys.style, 0)?;
        if style != HORIZONTAL_STYLE {
            return Err(UiLayoutError::unresolved(
                keys.style.movie,
                keys.style.path,
                format!("the bracket style '{style}' is not one the HUD draws"),
            ));
        }
        let spec = Self {
            corner: length(layout, &keys.corner)?,
            line: length(layout, &keys.line)?,
            pad_x: length(layout, &keys.pad_x)?,
            pad_y: length(layout, &keys.pad_y)?,
        };
        if spec.line == 0.0 {
            return Err(UiLayoutError::unresolved(
                keys.line.movie,
                keys.line.path,
                "the bracket line has no width",
            ));
        }
        Ok(Some(spec))
    }

    /// `content` grown by the padding.
    pub(super) fn frame(&self, content: PxRect, layout: &Layout) -> PxRect {
        let (pad_x, pad_y) = (layout.s(self.pad_x), layout.s(self.pad_y));
        PxRect {
            x: content.x - pad_x,
            y: content.y - pad_y,
            w: content.w + 2.0 * pad_x,
            h: content.h + 2.0 * pad_y,
        }
    }

    /// The brackets of `frame` at `alpha`: per side a bar over the full height and, inside it, a
    /// hook of the corner length at the top and the bottom.
    pub(super) fn emit(&self, qb: &mut QuadBuilder, layout: &Layout, frame: PxRect, alpha: f32) {
        let [r, g, b] = LINE_WHITE;
        let colour = [r, g, b, alpha];
        let (line, corner) = (layout.s(self.line), layout.s(self.corner));
        let bottom = frame.y + frame.h - line;
        for (bar_x, hook_x) in [
            (frame.x, frame.x + line),
            (frame.x + frame.w - line, frame.x + frame.w - line - corner),
        ] {
            qb.rect(
                PxRect {
                    x: bar_x,
                    y: frame.y,
                    w: line,
                    h: frame.h,
                },
                colour,
            );
            for hook_y in [frame.y, bottom] {
                qb.rect(
                    PxRect {
                        x: hook_x,
                        y: hook_y,
                        w: corner,
                        h: line,
                    },
                    colour,
                );
            }
        }
    }
}

fn length(layout: &UiLayout, key: &LiteralKey) -> Result<f32, UiLayoutError> {
    let value = layout.lit_number(key, 0)?;
    if value.is_finite() && value >= 0.0 {
        Ok(value as f32)
    } else {
        Err(UiLayoutError::unresolved(
            key.movie,
            key.path,
            format!("{value} is not a length"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::hud::quad::recorded_rects;
    use crate::ui_layout::test_support::{assert_close, literal, synthesized_layout};
    use serde_json::json;

    const MOVIE: &str = "Fixture";
    const PREFIX: &str = "Fixture.setProp#Comp";

    fn keys() -> BracketKeys {
        bracket_keys!("Fixture", "Fixture.setProp#Comp")
    }

    fn layout_of(shows: bool, style: &str, line: f64) -> UiLayout {
        let overlay = json!({"literals": {
            format!("{PREFIX}.bShowBrackets="): literal(&json!(shows)),
            format!("{PREFIX}.BracketStyle="): literal(&json!(style)),
            format!("{PREFIX}.bracketCornerLength="): literal(&json!(5)),
            format!("{PREFIX}.bracketLineWidth="): literal(&json!(line)),
            format!("{PREFIX}.bracketPaddingX="): literal(&json!(7)),
            format!("{PREFIX}.bracketPaddingY="): literal(&json!(3)),
        }});
        synthesized_layout(&[&keys()], &[(MOVIE, overlay)])
    }

    fn spec() -> BracketSpec {
        BracketSpec::resolve(&layout_of(true, "horizontal", 2.0), &keys())
            .unwrap()
            .expect("brackets are on")
    }

    fn assert_rect(actual: PxRect, expected: [f32; 4]) {
        for (a, e) in [actual.x, actual.y, actual.w, actual.h]
            .into_iter()
            .zip(expected)
        {
            assert!((a - e).abs() < 1e-3, "{actual:?} is not {expected:?}");
        }
    }

    #[test]
    fn the_spec_reads_the_literals_of_the_component() {
        assert_eq!(
            spec(),
            BracketSpec {
                corner: 5.0,
                line: 2.0,
                pad_x: 7.0,
                pad_y: 3.0
            }
        );
    }

    #[test]
    fn a_component_without_brackets_has_no_spec() {
        let layout = layout_of(false, "horizontal", 2.0);
        assert_eq!(BracketSpec::resolve(&layout, &keys()).unwrap(), None);
    }

    #[test]
    fn a_style_the_hud_does_not_draw_is_refused_naming_the_literal() {
        let error = BracketSpec::resolve(&layout_of(true, "vertical", 2.0), &keys()).unwrap_err();
        assert!(error.to_string().contains("BracketStyle"), "{error}");
        assert!(error.to_string().contains("vertical"), "{error}");
    }

    #[test]
    fn a_line_without_width_is_refused_naming_the_literal() {
        let error = BracketSpec::resolve(&layout_of(true, "horizontal", 0.0), &keys()).unwrap_err();
        assert!(error.to_string().contains("bracketLineWidth"), "{error}");
    }

    #[test]
    fn a_negative_measure_is_refused_naming_the_literal() {
        let error =
            BracketSpec::resolve(&layout_of(true, "horizontal", -1.0), &keys()).unwrap_err();
        assert!(error.to_string().contains("bracketLineWidth"), "{error}");
    }

    #[test]
    fn the_frame_is_the_content_grown_by_the_padding_in_screen_pixels() {
        let content = PxRect {
            x: 100.0,
            y: 50.0,
            w: 200.0,
            h: 40.0,
        };
        for fb in [(1280_u32, 720_u32), (2560, 1440)] {
            let layout = Layout::hud_stage(fb);
            let scale = layout.scale();
            assert_rect(
                spec().frame(content, &layout),
                [
                    100.0 - 7.0 * scale,
                    50.0 - 3.0 * scale,
                    200.0 + 14.0 * scale,
                    40.0 + 6.0 * scale,
                ],
            );
        }
    }

    #[test]
    fn each_side_is_a_full_height_bar_with_two_inward_hooks() {
        let layout = Layout::hud_stage((2560, 1440));
        let scale = layout.scale();
        let frame = PxRect {
            x: 100.0,
            y: 200.0,
            w: 600.0,
            h: 90.0,
        };
        let mut verts = Vec::new();
        let mut qb = QuadBuilder::new(&mut verts, 2560.0, 1440.0, 0.0, [0.0; 4]);
        spec().emit(&mut qb, &layout, frame, 0.5);
        qb.finish();
        let rects = recorded_rects(&verts, (2560.0, 1440.0));
        assert_eq!(rects.len(), 6);
        let (line, corner) = (2.0 * scale, 5.0 * scale);
        let expected = [
            [100.0, 200.0, line, 90.0],
            [100.0 + line, 200.0, corner, line],
            [100.0 + line, 290.0 - line, corner, line],
            [700.0 - line, 200.0, line, 90.0],
            [700.0 - line - corner, 200.0, corner, line],
            [700.0 - line - corner, 290.0 - line, corner, line],
        ];
        for ((rect, colour), expected) in rects.into_iter().zip(expected) {
            assert_rect(rect, expected);
            assert_eq!(colour, [1.0, 1.0, 1.0, 0.5]);
        }
    }

    #[test]
    fn hooks_and_bars_do_not_overlap_so_a_translucent_bracket_stays_even() {
        let layout = Layout::hud_stage((1280, 720));
        let frame = PxRect {
            x: 0.0,
            y: 0.0,
            w: 300.0,
            h: 40.0,
        };
        let mut verts = Vec::new();
        let mut qb = QuadBuilder::new(&mut verts, 1280.0, 720.0, 0.0, [0.0; 4]);
        spec().emit(&mut qb, &layout, frame, 1.0);
        qb.finish();
        let rects: Vec<PxRect> = recorded_rects(&verts, (1280.0, 720.0))
            .into_iter()
            .map(|(rect, _)| rect)
            .collect();
        for (index, a) in rects.iter().enumerate() {
            for b in &rects[index + 1..] {
                let apart = a.x + a.w <= b.x + 1e-3
                    || b.x + b.w <= a.x + 1e-3
                    || a.y + a.h <= b.y + 1e-3
                    || b.y + b.h <= a.y + 1e-3;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn the_bracket_scales_with_the_framebuffer() {
        let one = Layout::hud_stage((1280, 720));
        let two = Layout::hud_stage((2560, 1440));
        assert_close(two.s(spec().line), 2.0 * one.s(spec().line));
    }
}
