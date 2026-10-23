//! The button hint bar: its place and backdrop, what each hint says and which key it shows.

use super::{Words, local_origin, translate_key_of, unresolved};
use crate::rendering::pipboy::hint_bar::{HintBarLayout, MoviePlacement};
use crate::rendering::pipboy::shapes::{Shade, code_colour};
use crate::rendering::pipboy::tab::HintKey;
use crate::ui_layout::values::Scalar;
use crate::ui_layout::{KeyGroup, LiteralKey, PlacementKey, UiLayout, UiLayoutError};

pub(crate) static HINT_BAR: PlacementKey =
    PlacementKey::new(super::MOVIE, "[LevelUpMenu]/ButtonHintBar_mc");
literal!(
    BACKGROUND_COLOUR,
    "LevelUpMenu.__setProp_ButtonHintBar_mc_MenuObj_ButtonHintBar_mc_0#ButtonHintBar_mc.BackgroundColor=",
    1
);
literal!(
    BACKGROUND_ALPHA,
    "LevelUpMenu.__setProp_ButtonHintBar_mc_MenuObj_ButtonHintBar_mc_0#ButtonHintBar_mc.BackgroundAlpha=",
    1
);
text_literal!(
    CHOOSE_PREFIX,
    "LevelUpMenu.SetButtons#+uiPerkCount.toString()",
    1
);
text_literal!(
    CHOOSE_SUFFIX,
    "LevelUpMenu.SetButtons#(\"$$Choose (\"+uiPerkCount.toString())+",
    1
);
text_literal!(HINT_TEXTS, "LevelUpMenu.<init>#new BSButtonHintData(0)", 4);
text_literal!(HINT_KEYS, "LevelUpMenu.<init>#new BSButtonHintData(1)", 4);

pub(crate) static KEYS: KeyGroup = KeyGroup(&[
    &crate::rendering::pipboy::hint_bar::LAYOUT_KEYS,
    &HINT_BAR,
    &BACKGROUND_COLOUR,
    &BACKGROUND_ALPHA,
    &CHOOSE_PREFIX,
    &CHOOSE_SUFFIX,
    &HINT_TEXTS,
    &HINT_KEYS,
]);

/// What a hint is for, in the menu's own order.
pub struct HintLabels {
    pub accept: String,
    pub close: String,
    pub previous: String,
    pub next: String,
    pub keys: [HintKey; 4],
    pub choose_template: String,
}

pub struct HintsLayout {
    pub bar: HintBarLayout,
    pub placement: MoviePlacement,
    pub labels: HintLabels,
}

fn text_at(values: &[Scalar], index: usize, key: &LiteralKey) -> Result<String, UiLayoutError> {
    match &values[index] {
        Scalar::Text(text) => Ok(text.clone()),
        other => Err(unresolved(
            key.path,
            format!("entry {index} is {}", other.type_name()),
        )),
    }
}

fn key_at(values: &[Scalar], index: usize) -> Result<HintKey, UiLayoutError> {
    let name = text_at(values, index, &HINT_KEYS)?;
    HintKey::from_label(&name)
        .ok_or_else(|| unresolved(HINT_KEYS.path, format!("'{name}' is not a known key")))
}

/// The backdrop of the bar as the menu sets it: a packed colour and an opacity.
fn backdrop(layout: &UiLayout) -> Result<Shade, UiLayoutError> {
    Ok(Shade {
        alpha: layout.lit_number(&BACKGROUND_ALPHA, 0)? as f32,
        ..code_colour(layout, &BACKGROUND_COLOUR, 0)?
    })
}

pub fn load(layout: &UiLayout, words: &Words<'_>) -> Result<HintsLayout, UiLayoutError> {
    let texts = layout.literal_list(&HINT_TEXTS)?;
    let keys = layout.literal_list(&HINT_KEYS)?;
    let text =
        |index: usize| Ok::<_, UiLayoutError>(words.text(&text_at(&texts, index, &HINT_TEXTS)?));
    let choose_prefix = layout.lit_string(&CHOOSE_PREFIX, 0)?;
    let choose_suffix = layout.lit_string(&CHOOSE_SUFFIX, 0)?;
    let choose_key = format!("{}#{choose_suffix}", translate_key_of(&choose_prefix));
    Ok(HintsLayout {
        bar: HintBarLayout::resolve(layout)?,
        placement: MoviePlacement {
            origin: local_origin(layout, &HINT_BAR)?.to_pos2(),
            backdrop: backdrop(layout)?,
        },
        labels: HintLabels {
            accept: text(0)?,
            close: text(1)?,
            previous: text(2)?,
            next: text(3)?,
            keys: [
                key_at(&keys, 0)?,
                key_at(&keys, 1)?,
                key_at(&keys, 2)?,
                key_at(&keys, 3)?,
            ],
            choose_template: layout.translate_text(&choose_key)?,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::pipboy::test_layout::pipboy_layout;
    use crate::ui_layout::test_support::assert_close;

    #[test]
    fn the_bar_sits_at_the_menus_clip_over_a_backdrop_of_the_menus_colour_and_opacity() {
        let layout = pipboy_layout();
        let placement = load(&layout, &Words::new(&layout).unwrap())
            .unwrap()
            .placement;
        assert_eq!(placement.origin, egui::Pos2::new(111.0, 222.0));
        assert_close(placement.backdrop.grey, f32::from(0x30_u8) / 255.0);
        assert_close(placement.backdrop.alpha, 0.5);
    }

    #[test]
    fn the_hint_keys_are_resolved_by_label() {
        let layout = pipboy_layout();
        let hints = load(&layout, &Words::new(&layout).unwrap()).unwrap();
        for key in hints.labels.keys {
            assert_eq!(HintKey::from_label(key.label()), Some(key));
        }
    }

    #[test]
    fn an_entry_that_is_not_text_is_rejected() {
        let error = text_at(&[Scalar::Number(1.0)], 0, &HINT_KEYS).unwrap_err();
        assert!(error.to_string().contains("entry 0"), "{error}");
    }

    #[test]
    fn an_unknown_key_name_is_rejected() {
        let error = key_at(&[Scalar::Text("Nowhere".into())], 0).unwrap_err();
        assert!(error.to_string().contains("not a known key"), "{error}");
    }
}
