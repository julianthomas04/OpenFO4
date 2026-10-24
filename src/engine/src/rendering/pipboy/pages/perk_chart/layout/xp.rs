//! The XP meter: its background, brackets, level text and the meter with its fill.

use egui::Vec2;

use super::{Text, Words, local_origin, single, translate_key_of, unresolved};
use crate::ui_layout::{KeyGroup, PlacementKey, ShapeKey, TextKey, UiLayout, UiLayoutError};

text_literal!(LEVEL_LABEL, "LevelUpMenu.onCodeObjCreate#+$1.level", 1);
text_literal!(
    BRACKET_STYLE,
    "LevelUpMenu.__setProp_XPMeterHolder_mc_MenuObj_XPMeter_0#XPMeterHolder_mc.BracketStyle=",
    1
);
literal!(
    BRACKET_LINE,
    "LevelUpMenu.__setProp_XPMeterHolder_mc_MenuObj_XPMeter_0#XPMeterHolder_mc.bracketLineWidth=",
    1
);
literal!(
    BRACKET_CORNER,
    "LevelUpMenu.__setProp_XPMeterHolder_mc_MenuObj_XPMeter_0#XPMeterHolder_mc.bracketCornerLength=",
    1
);
literal!(
    BRACKET_PADDING_X,
    "LevelUpMenu.__setProp_XPMeterHolder_mc_MenuObj_XPMeter_0#XPMeterHolder_mc.bracketPaddingX=",
    1
);
literal!(
    BRACKET_PADDING_Y,
    "LevelUpMenu.__setProp_XPMeterHolder_mc_MenuObj_XPMeter_0#XPMeterHolder_mc.bracketPaddingY=",
    1
);

pub(crate) static HOLDER: PlacementKey =
    PlacementKey::new(super::MOVIE, "[LevelUpMenu]/XPMeterHolder_mc");
pub(crate) static BACKGROUND: ShapeKey =
    ShapeKey::new(super::MOVIE, "[LevelUpMenu]/XPMeterHolder_mc/@1");
pub(crate) static TEXT: TextKey =
    TextKey::new(super::MOVIE, "[LevelUpMenu]/XPMeterHolder_mc/textField");
pub(crate) static METER: PlacementKey =
    PlacementKey::new(super::MOVIE, "[LevelUpMenu]/XPMeterHolder_mc/Meter_mc");
pub(crate) static METER_OUTLINE: ShapeKey =
    ShapeKey::new(super::MOVIE, "[LevelUpMenu]/XPMeterHolder_mc/Meter_mc/@1");
pub(crate) static METER_FILL: PlacementKey = PlacementKey::new(
    super::MOVIE,
    "[LevelUpMenu]/XPMeterHolder_mc/Meter_mc/Fill_mc",
);
pub(crate) static METER_FILL_SHAPE: ShapeKey = ShapeKey::new(
    super::MOVIE,
    "[LevelUpMenu]/XPMeterHolder_mc/Meter_mc/Fill_mc/@1",
);

pub(crate) static KEYS: KeyGroup = KeyGroup(&[
    &LEVEL_LABEL,
    &BRACKET_STYLE,
    &BRACKET_LINE,
    &BRACKET_CORNER,
    &BRACKET_PADDING_X,
    &BRACKET_PADDING_Y,
    &HOLDER,
    &BACKGROUND,
    &TEXT,
    &METER,
    &METER_OUTLINE,
    &METER_FILL,
    &METER_FILL_SHAPE,
]);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BracketLayout {
    pub line_width: f32,
    pub corner: f32,
    pub padding: Vec2,
}

pub struct XpLayout {
    pub holder: Vec2,
    pub bounds: [f32; 4],
    pub fill: [u8; 4],
    pub text: Text,
    pub level_word: String,
    pub brackets: BracketLayout,
    pub meter_origin: Vec2,
    pub meter_scale: Vec2,
    pub outline_path: [f32; 4],
    pub outline_width: f32,
    pub meter_fill_origin: Vec2,
    pub meter_fill_scale: Vec2,
    pub meter_fill_bounds: [f32; 4],
}

fn brackets(layout: &UiLayout) -> Result<BracketLayout, UiLayoutError> {
    let style = layout.lit_string(&BRACKET_STYLE, 0)?;
    if style != "horizontal" {
        return Err(unresolved(
            BRACKET_STYLE.path,
            format!("bracket style '{style}' is not supported"),
        ));
    }
    Ok(BracketLayout {
        line_width: single(layout, &BRACKET_LINE)?,
        corner: single(layout, &BRACKET_CORNER)?,
        padding: Vec2::new(
            single(layout, &BRACKET_PADDING_X)?,
            single(layout, &BRACKET_PADDING_Y)?,
        ),
    })
}

pub fn load(layout: &UiLayout, words: &Words<'_>) -> Result<XpLayout, UiLayoutError> {
    let background = layout.get(&BACKGROUND)?;
    let outline = layout.get(&METER_OUTLINE)?;
    let fill_shape = layout.get(&METER_FILL_SHAPE)?;
    let fill_matrix = layout.get(&METER_FILL)?.local.matrix;
    let meter_matrix = layout.get(&METER)?.local.matrix;
    let level_literal = layout.lit_string(&LEVEL_LABEL, 0)?;
    Ok(XpLayout {
        holder: local_origin(layout, &HOLDER)?,
        bounds: background.bounds,
        fill: background
            .fills
            .first()
            .and_then(|fill| fill.color)
            .ok_or_else(|| unresolved(HOLDER.path, "its background has no solid fill"))?,
        text: Text::load(layout, &TEXT)?,
        level_word: words.text(translate_key_of(&level_literal).trim_end()),
        brackets: brackets(layout)?,
        meter_origin: Vec2::new(meter_matrix.tx, meter_matrix.ty),
        meter_scale: Vec2::new(meter_matrix.a, meter_matrix.d),
        outline_path: outline.edge_bounds,
        outline_width: outline
            .lines
            .first()
            .map(|line| line.width)
            .ok_or_else(|| unresolved(METER.path, "the meter outline has no line"))?,
        meter_fill_origin: Vec2::new(fill_matrix.tx, fill_matrix.ty),
        meter_fill_scale: Vec2::new(fill_matrix.a, fill_matrix.d),
        meter_fill_bounds: fill_shape.bounds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::pipboy::test_layout::pipboy_layout;

    #[test]
    fn the_level_word_is_the_translated_label_without_its_trailing_space() {
        let layout = pipboy_layout();
        let xp = load(&layout, &Words::new(&layout).unwrap()).unwrap();
        assert_ne!(xp.level_word, "");
        assert!(!xp.level_word.ends_with(' '));
    }

    #[test]
    fn the_meter_outline_has_a_stroke() {
        let layout = pipboy_layout();
        let xp = load(&layout, &Words::new(&layout).unwrap()).unwrap();
        assert!(xp.outline_width > 0.0);
    }

    #[test]
    fn a_bracket_style_other_than_horizontal_is_rejected() {
        let movie = crate::rendering::pipboy::pages::perk_chart::fixture::level_up_menu_movie()
            .replace("horizontal", "vertical");
        let layout = crate::ui_layout::test_support::layout_with_movie(super::super::MOVIE, &movie);
        let error = brackets(&layout).expect_err("the load fails");
        assert!(error.to_string().contains("not supported"), "{error}");
    }
}
