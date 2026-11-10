//! The button hint component (`BSButtonHint`) every menu movie that shows hints embeds: how a hint's
//! key caption is spelled, the face and offset of its key field, where its label goes and how
//! hints are spaced and centred. The literals live in each movie under the same names; only the
//! movie differs, so a consumer asks for the keys of its own movie.

use super::error::UiLayoutError;
use super::layout::UiLayout;
use super::set_text::SetTextRules;
use super::{LiteralKey, PlacementKey, TextKey, key_check_fields};

/// The keys of the hint component in one movie.
pub struct ButtonHintKeys {
    pub key_field: TextKey,
    pub label: TextKey,
    icon_holder: PlacementKey,
    key_y: LiteralKey,
    key_font: LiteralKey,
    key_suffix: LiteralKey,
    icon_holder_x: LiteralKey,
    gap: LiteralKey,
    centre_divisor: LiteralKey,
    label_upper_case: LiteralKey,
}

impl ButtonHintKeys {
    pub const fn of(movie: &'static str) -> Self {
        Self {
            key_field: TextKey::new(
                movie,
                "[Shared.AS3.BSButtonHint]/IconHolderInstance/IconAnimInstance/Icon_tf",
            ),
            label: TextKey::new(movie, "[Shared.AS3.BSButtonHint]/textField_tf"),
            icon_holder: PlacementKey::new(movie, "[Shared.AS3.BSButtonHint]/IconHolderInstance"),
            key_y: LiteralKey::new(movie, "Shared.AS3.BSButtonHint.UpdateIconTextField#$5=", 2),
            key_font: LiteralKey::texts(movie, "Shared.AS3.BSButtonHint.GetExpectedFont#$1=", 3),
            key_suffix: LiteralKey::texts(
                movie,
                "Shared.AS3.BSButtonHint.get:PCKey#_buttonHintData.PCKey+",
                1,
            ),
            icon_holder_x: LiteralKey::new(
                movie,
                "Shared.AS3.BSButtonHint.redrawPrimaryButton#IconHolderInstance.x=",
                1,
            ),
            gap: LiteralKey::new(
                movie,
                "Shared.AS3.BSButtonHintBar.redrawUIComponent#$6.width+",
                1,
            ),
            centre_divisor: LiteralKey::new(
                movie,
                "Shared.AS3.BSButtonHintBar.redrawUIComponent#-(ButtonHintBarInternal_mc.width)/",
                1,
            ),
            label_upper_case: LiteralKey::flags(
                movie,
                "Shared.AS3.BSButtonHint.redrawTextField#GlobalFunc.SetText(3)",
                1,
            ),
        }
    }

    pub fn resolve(&self, layout: &UiLayout) -> Result<ButtonHintRules, UiLayoutError> {
        let key_placement = layout.get(&self.key_field)?.placement;
        let holder = layout.get(&self.icon_holder)?;
        let key_y = layout.lit_number(&self.key_y, 0)? as f32;
        Ok(ButtonHintRules {
            key_suffix: layout.lit_string(&self.key_suffix, 0)?,
            key_font: layout.lit_string(&self.key_font, 0)?,
            key_y_shift: key_y - key_placement.local.matrix.ty,
            icon_holder_x: layout.lit_number(&self.icon_holder_x, 0)? as f32,
            key_x_in_holder: key_placement.world.matrix.tx - holder.world.matrix.tx,
            label_x: layout.get(&self.label)?.placement.local.matrix.tx,
            gap: layout.lit_number(&self.gap, 0)? as f32,
            centre_divisor: layout.lit_number(&self.centre_divisor, 0)? as f32,
            label_upper_case: layout.lit_flag(&self.label_upper_case, 0)?,
        })
    }
}

key_check_fields!(ButtonHintKeys {
    key_field,
    label,
    icon_holder,
    key_y,
    key_font,
    key_suffix,
    icon_holder_x,
    gap,
    centre_divisor,
    label_upper_case,
});

/// The hint component's rules of one movie, resolved from the layout.
#[derive(Debug, Clone, PartialEq)]
pub struct ButtonHintRules {
    /// Follows the key's name in the caption: "E" becomes "E)".
    pub key_suffix: String,
    /// The font class the code sets on the key field.
    pub key_font: String,
    /// How far the code moves the key field down from where it is authored.
    pub key_y_shift: f32,
    /// The key holder's x as the code sets it, where the key field's box starts inside the
    /// holder, and the label field's authored x, which the code replaces by the holder's width.
    pub icon_holder_x: f32,
    pub key_x_in_holder: f32,
    pub label_x: f32,
    /// The space between two hints.
    pub gap: f32,
    /// A row of hints is centred by its width divided by this.
    pub centre_divisor: f32,
    pub label_upper_case: bool,
}

impl ButtonHintRules {
    /// The key as the hint shows it.
    pub fn key_caption(&self, key: &str) -> String {
        format!("{key}{}", self.key_suffix)
    }

    /// A hint's label as the hint shows it: upper-cased when the component asks for it, never
    /// truncated.
    pub fn label_text(&self, rules: &SetTextRules, layout: &UiLayout, label: &str) -> String {
        rules.apply(layout, label, self.label_upper_case, false)
    }

    /// How far the code moves a hint's label when the key field's box is `key_box_width` wide.
    pub fn label_shift(&self, key_box_width: f32) -> f32 {
        label_shift(
            self.icon_holder_x,
            self.key_x_in_holder + key_box_width,
            self.label_x,
        )
    }
}

/// How far the code moves a hint's label: it sets the label's x to the key holder's width, which
/// reaches from the holder's origin to the right edge of the key field, and the holder sits at
/// `holder_x`.
fn label_shift(holder_x: f32, holder_width: f32, authored_label_x: f32) -> f32 {
    holder_x + holder_width - authored_label_x
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_layout::test_support::{assert_close, synthesized_layout};
    use serde_json::json;

    const MOVIE_A: &str = "HintMovieA";
    const MOVIE_B: &str = "HintMovieB";
    const KEYS_A: ButtonHintKeys = ButtonHintKeys::of(MOVIE_A);
    const KEYS_B: ButtonHintKeys = ButtonHintKeys::of(MOVIE_B);

    fn rules_with(overlay: serde_json::Value) -> ButtonHintRules {
        let layout = synthesized_layout(&[&KEYS_A, &KEYS_B], &[(MOVIE_A, overlay)]);
        KEYS_A.resolve(&layout).expect("the fixture resolves")
    }

    #[test]
    fn the_label_follows_the_right_edge_of_the_key_field_inside_its_holder() {
        let key_x_in_holder = 2.0;
        let key_box = 30.0;
        assert_close(label_shift(0.0, key_x_in_holder + key_box, 2.0), 30.0);
        assert_close(label_shift(5.0, key_x_in_holder + key_box, 2.0), 35.0);
    }

    #[test]
    fn the_caption_is_the_key_followed_by_the_suffix() {
        let rules = rules_with(json!({"literals": {
            "Shared.AS3.BSButtonHint.get:PCKey#_buttonHintData.PCKey+": {"values": [")"]},
        }}));
        assert_eq!(rules.key_caption("E"), "E)");
        assert_eq!(rules.key_caption("Ctrl"), "Ctrl)");
    }

    #[test]
    fn the_rules_of_each_movie_come_from_that_movie() {
        let layout = synthesized_layout(
            &[&KEYS_A, &KEYS_B],
            &[
                (
                    MOVIE_A,
                    json!({"literals": {
                        "Shared.AS3.BSButtonHintBar.redrawUIComponent#$6.width+": {"values": [20]},
                    }}),
                ),
                (
                    MOVIE_B,
                    json!({"literals": {
                        "Shared.AS3.BSButtonHintBar.redrawUIComponent#$6.width+": {"values": [7]},
                    }}),
                ),
            ],
        );
        assert_close(KEYS_A.resolve(&layout).unwrap().gap, 20.0);
        assert_close(KEYS_B.resolve(&layout).unwrap().gap, 7.0);
    }

    #[test]
    fn the_key_field_is_moved_to_the_height_the_code_sets() {
        let rules = rules_with(json!({
            "literals": {
                "Shared.AS3.BSButtonHint.UpdateIconTextField#$5=": {"values": [-1.5, 0]},
            },
            "instances": {
                "[Shared.AS3.BSButtonHint]/IconHolderInstance/IconAnimInstance/Icon_tf":
                    crate::ui_layout::test_support::instance(3, "text", 4.0, 3.0),
            },
        }));
        assert_close(rules.key_y_shift, -4.5);
    }

    #[test]
    fn labels_are_upper_cased_only_when_the_component_asks_for_it() {
        use crate::ui_layout::set_text::SetTextKeys;
        let flag_path = "Shared.AS3.BSButtonHint.redrawTextField#GlobalFunc.SetText(3)";
        let set_text = SetTextKeys::new(MOVIE_A);
        let text_rules = |upper: bool| {
            let layout = synthesized_layout(
                &[&KEYS_A, &set_text],
                &[(
                    MOVIE_A,
                    json!({"literals": {flag_path: {"values": [upper]}}}),
                )],
            );
            let rules = KEYS_A.resolve(&layout).unwrap();
            let shown = rules.label_text(&set_text.resolve(&layout).unwrap(), &layout, "open");
            (rules.label_upper_case, shown)
        };
        assert_eq!(text_rules(true), (true, "OPEN".to_owned()));
        assert_eq!(text_rules(false), (false, "open".to_owned()));
    }
}
