//! The shaded background a HUD component draws behind its content: the game renders it with a
//! native shader the movie only names (`ShadedBackgroundMethod` "Shader", type "normal"), so the
//! engine draws one flat translucent rectangle in the colour and alpha the original was measured
//! to produce.

use super::quad::{PxRect, QuadBuilder};
use crate::rendering::lighting::cell_lighting::srgb_to_linear;
use crate::ui_layout::{LiteralKey, UiLayout, UiLayoutError, key_check_fields};

const SHADER_METHOD: &str = "Shader";
const NORMAL_TYPE: &str = "normal";

pub(super) struct BackdropKeys {
    pub(super) shows: LiteralKey,
    pub(super) method: LiteralKey,
    pub(super) kind: LiteralKey,
}

key_check_fields!(BackdropKeys {
    shows,
    method,
    kind
});

/// The backdrop keys of the component whose setting literals start with `$prefix`.
macro_rules! backdrop_keys {
    ($movie:expr, $prefix:expr) => {
        $crate::rendering::hud::backdrop::BackdropKeys {
            shows: $crate::ui_layout::LiteralKey::flags(
                $movie,
                concat!($prefix, ".bUseShadedBackground="),
                1,
            ),
            method: $crate::ui_layout::LiteralKey::texts(
                $movie,
                concat!($prefix, ".ShadedBackgroundMethod="),
                1,
            ),
            kind: $crate::ui_layout::LiteralKey::texts(
                $movie,
                concat!($prefix, ".ShadedBackgroundType="),
                1,
            ),
        }
    };
}
pub(super) use backdrop_keys;

/// What the backdrop looks like: not in the movie, measured in the original.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BackdropLook {
    /// The colour the rectangle is blended in, 8-bit sRGB.
    pub colour: [u8; 3],
    pub alpha: f32,
}

/// A component's backdrop: the one kind the HUD draws, so nothing to hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BackdropSpec;

impl BackdropSpec {
    /// `None` when the component draws no backdrop; an error naming the literal when the method or
    /// the type is not the one the HUD draws.
    pub(super) fn resolve(
        layout: &UiLayout,
        keys: &BackdropKeys,
    ) -> Result<Option<Self>, UiLayoutError> {
        if !layout.lit_flag(&keys.shows, 0)? {
            return Ok(None);
        }
        for (key, drawn) in [(&keys.method, SHADER_METHOD), (&keys.kind, NORMAL_TYPE)] {
            let value = layout.lit_string(key, 0)?;
            if value != drawn {
                return Err(UiLayoutError::unresolved(
                    key.movie,
                    key.path,
                    format!("the shaded background '{value}' is not the '{drawn}' the HUD draws"),
                ));
            }
        }
        Ok(Some(Self))
    }

    /// One flat rectangle over `frame`: the look's colour as linear light (the HUD tint multiplies
    /// it like any art) and its alpha times `alpha`.
    pub(super) fn emit(qb: &mut QuadBuilder, frame: PxRect, look: BackdropLook, alpha: f32) {
        let [r, g, b] = look.colour.map(srgb_to_linear);
        qb.rect_flat(frame, [r, g, b, look.alpha * alpha]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::hud::quad::recorded_rects;
    use crate::ui_layout::test_support::{literal, synthesized_layout};
    use serde_json::json;

    const PREFIX: &str = "Fixture.setProp#Comp";

    fn keys() -> BackdropKeys {
        backdrop_keys!("Fixture", "Fixture.setProp#Comp")
    }

    fn layout_of(shows: bool, method: &str, kind: &str) -> UiLayout {
        let overlay = json!({"literals": {
            format!("{PREFIX}.bUseShadedBackground="): literal(&json!(shows)),
            format!("{PREFIX}.ShadedBackgroundMethod="): literal(&json!(method)),
            format!("{PREFIX}.ShadedBackgroundType="): literal(&json!(kind)),
        }});
        synthesized_layout(&[&keys()], &[("Fixture", overlay)])
    }

    #[test]
    fn a_component_without_a_shaded_background_has_none() {
        let layout = layout_of(false, "Shader", "normal");
        assert_eq!(BackdropSpec::resolve(&layout, &keys()).unwrap(), None);
    }

    #[test]
    fn the_shader_method_of_the_normal_type_is_the_one_the_hud_draws() {
        let layout = layout_of(true, "Shader", "normal");
        assert_eq!(
            BackdropSpec::resolve(&layout, &keys()).unwrap(),
            Some(BackdropSpec)
        );
    }

    #[test]
    fn another_method_or_type_is_refused_naming_its_literal() {
        let method = BackdropSpec::resolve(&layout_of(true, "Blur", "normal"), &keys());
        let message = method.unwrap_err().to_string();
        assert!(message.contains("ShadedBackgroundMethod"), "{message}");
        assert!(message.contains("Blur"), "{message}");
        let kind = BackdropSpec::resolve(&layout_of(true, "Shader", "inverted"), &keys());
        let message = kind.unwrap_err().to_string();
        assert!(message.contains("ShadedBackgroundType"), "{message}");
    }

    #[test]
    fn the_rectangle_is_flat_linear_light_with_the_alpha_scaled() {
        let frame = PxRect {
            x: 10.0,
            y: 20.0,
            w: 300.0,
            h: 40.0,
        };
        let look = BackdropLook {
            colour: [10, 255, 0],
            alpha: 0.5,
        };
        let mut verts = Vec::new();
        let mut qb = QuadBuilder::new(&mut verts, 640.0, 360.0, 2.0, [0.0, 0.0, 0.0, 0.6]);
        BackdropSpec::emit(&mut qb, frame, look, 0.5);
        qb.finish();
        let rects = recorded_rects(&verts, (640.0, 360.0));
        assert_eq!(rects.len(), 1, "a backdrop casts no shadow");
        let (rect, colour) = rects[0];
        assert!((rect.x - 10.0).abs() < 1e-3 && (rect.h - 40.0).abs() < 1e-3);
        let expected = [10.0 / 255.0 / 12.92, 1.0, 0.0, 0.25];
        for (actual, expected) in colour.into_iter().zip(expected) {
            assert!((actual - expected).abs() < 1e-5, "{colour:?}");
        }
    }
}
