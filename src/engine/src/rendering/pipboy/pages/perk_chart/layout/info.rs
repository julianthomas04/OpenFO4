//! The info panel: its background, text fields, the numbers of the menu's layout rule and the
//! error timer.

use egui::Vec2;

use super::{Text, Words, local_origin, number, single, unresolved};
use crate::ui_layout::{KeyGroup, PlacementKey, ShapeKey, TextKey, UiLayout, UiLayoutError};

number!(LOWER_Y, "LevelUpMenu.INFO_LOWER_Y");
number!(UPPER_Y, "LevelUpMenu.INFO_UPPER_Y");
number!(Y_BOUND, "LevelUpMenu.INFO_Y_BOUND");

literal!(
    ENTRY_Y_OFFSET,
    "PerkGrid.PopulateGrid#(PerksHolder_mc.y+clipParent.y)+",
    1
);
literal!(STAR_GAP, "LevelUpMenu.UpdateSelectionText#$3=", 1);
literal!(REQUIREMENTS_GAP, "LevelUpMenu.UpdateSelectionText#$4=", 1);
literal!(CONFIRM_GAP, "LevelUpMenu.UpdateSelectionText#$5=", 1);
literal!(WIDTH_PADDING, "LevelUpMenu.UpdateSelectionText#$7=", 1);
literal!(WIDTH_HALVES, "LevelUpMenu.UpdateSelectionText#$7/", 1);
literal!(HEIGHT_PADDING, "LevelUpMenu.UpdateSelectionText#$8=", 1);
literal!(STAR_PITCH, "LevelUpMenu.UpdateSelectionText#*$9", 1);
literal!(STAR_SCALE, "LevelUpMenu.UpdateSelectionText#$10.scaleX=", 1);
literal!(
    STAR_CENTRE,
    "LevelUpMenu.UpdateSelectionText#(PerkInfo_mc.Background_mc.width-PerkInfo_mc.StarHolder_mc.width)/",
    1
);
literal!(ERROR_TIMER_MS, "LevelUpMenu.ShowErrorText#new Timer(0)", 1);
text_literal!(
    CONFIRM_PROMPT,
    "LevelUpMenu.StartConfirmation#GlobalFunc.SetText(1)",
    1
);
text_literal!(
    REQUIREMENTS_NOT_MET,
    "LevelUpMenu.TryToAcquirePerk#ShowErrorText(0)",
    1
);

pub(crate) static PANEL: PlacementKey =
    PlacementKey::new(super::MOVIE, "[LevelUpMenu]/PerkInfo_mc");
pub(crate) static BACKGROUND: PlacementKey =
    PlacementKey::new(super::MOVIE, "[LevelUpMenu]/PerkInfo_mc/Background_mc");
pub(crate) static BACKGROUND_SHAPE: ShapeKey =
    ShapeKey::new(super::MOVIE, "[LevelUpMenu]/PerkInfo_mc/Background_mc/@1");
pub(crate) static NAME: TextKey =
    TextKey::new(super::MOVIE, "[LevelUpMenu]/PerkInfo_mc/PerkName_tf");
pub(crate) static DESCRIPTION: TextKey =
    TextKey::new(super::MOVIE, "[LevelUpMenu]/PerkInfo_mc/DescriptionText_tf");
pub(crate) static REQUIREMENTS: TextKey =
    TextKey::new(super::MOVIE, "[LevelUpMenu]/PerkInfo_mc/Reqs_tf");
pub(crate) static CONFIRM: TextKey =
    TextKey::new(super::MOVIE, "[LevelUpMenu]/PerkInfo_mc/Confirm_tf");

pub(crate) static KEYS: KeyGroup = KeyGroup(&[
    &LOWER_Y,
    &UPPER_Y,
    &Y_BOUND,
    &ENTRY_Y_OFFSET,
    &STAR_GAP,
    &REQUIREMENTS_GAP,
    &CONFIRM_GAP,
    &WIDTH_PADDING,
    &WIDTH_HALVES,
    &HEIGHT_PADDING,
    &STAR_PITCH,
    &STAR_SCALE,
    &STAR_CENTRE,
    &ERROR_TIMER_MS,
    &CONFIRM_PROMPT,
    &REQUIREMENTS_NOT_MET,
    &PANEL,
    &BACKGROUND,
    &BACKGROUND_SHAPE,
    &NAME,
    &DESCRIPTION,
    &REQUIREMENTS,
    &CONFIRM,
]);

#[derive(Clone, Debug, PartialEq)]
pub struct BackgroundLayout {
    /// The background sprite's position in the panel, the bounds of its shape with the stroke and
    /// the extent of its path.
    pub origin: Vec2,
    pub bounds: [f32; 4],
    pub edge_bounds: [f32; 4],
    pub fill: [u8; 4],
    pub line_width: f32,
}

/// The numbers the info panel's layout rule works with; every length is in the panel's own
/// coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct InfoRules {
    pub panel: Vec2,
    pub background: BackgroundLayout,
    pub name_y: f32,
    pub description_y: f32,
    pub requirements_y: f32,
    pub confirm_y: f32,
    pub star_gap: f32,
    pub requirements_gap: f32,
    pub confirm_gap: f32,
    pub width_padding: f32,
    pub width_halves: f32,
    pub height_padding: f32,
    pub star_pitch: f32,
    pub star_scale: f32,
    pub star_centre_halves: f32,
    pub lower_y: f32,
    pub upper_y: f32,
    pub y_bound: f32,
    pub entry_y_offset: f32,
}

pub struct InfoLayout {
    pub rules: InfoRules,
    pub name: Text,
    pub description: Text,
    pub requirements: Text,
    pub confirm: Text,
    pub error_seconds: f32,
    pub confirm_prompt: String,
    pub requirements_not_met: String,
}

fn background(
    layout: &UiLayout,
    sprite: &PlacementKey,
    shape: &ShapeKey,
) -> Result<BackgroundLayout, UiLayoutError> {
    let shape = layout.get(shape)?;
    let fill = shape
        .fills
        .first()
        .and_then(|fill| fill.color)
        .ok_or_else(|| unresolved(sprite.path, "its shape has no solid fill"))?;
    let line_width = shape.lines.first().map_or(0.0, |line| line.width);
    Ok(BackgroundLayout {
        origin: local_origin(layout, sprite)?,
        bounds: shape.bounds,
        edge_bounds: shape.edge_bounds,
        fill,
        line_width,
    })
}

pub fn load(layout: &UiLayout, words: &Words<'_>) -> Result<InfoLayout, UiLayoutError> {
    let name = Text::load(layout, &NAME)?;
    let description = Text::load(layout, &DESCRIPTION)?;
    let requirements = Text::load(layout, &REQUIREMENTS)?;
    let confirm = Text::load(layout, &CONFIRM)?;
    Ok(InfoLayout {
        rules: InfoRules {
            panel: local_origin(layout, &PANEL)?,
            background: background(layout, &BACKGROUND, &BACKGROUND_SHAPE)?,
            name_y: name.local_y(),
            description_y: description.local_y(),
            requirements_y: requirements.local_y(),
            confirm_y: confirm.local_y(),
            star_gap: single(layout, &STAR_GAP)?,
            requirements_gap: single(layout, &REQUIREMENTS_GAP)?,
            confirm_gap: single(layout, &CONFIRM_GAP)?,
            width_padding: single(layout, &WIDTH_PADDING)?,
            width_halves: single(layout, &WIDTH_HALVES)?,
            height_padding: single(layout, &HEIGHT_PADDING)?,
            star_pitch: single(layout, &STAR_PITCH)?,
            star_scale: single(layout, &STAR_SCALE)?,
            star_centre_halves: single(layout, &STAR_CENTRE)?,
            lower_y: number(layout, &LOWER_Y)?,
            upper_y: number(layout, &UPPER_Y)?,
            y_bound: number(layout, &Y_BOUND)?,
            entry_y_offset: single(layout, &ENTRY_Y_OFFSET)?,
        },
        name,
        description,
        requirements,
        confirm,
        error_seconds: single(layout, &ERROR_TIMER_MS)? / 1000.0,
        confirm_prompt: words.text(&layout.lit_string(&CONFIRM_PROMPT, 0)?),
        requirements_not_met: words.text(&layout.lit_string(&REQUIREMENTS_NOT_MET, 0)?),
    })
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "exact literals")]
mod tests {
    use super::*;
    use crate::rendering::pipboy::test_layout::pipboy_layout;

    fn info() -> InfoLayout {
        let layout = pipboy_layout();
        load(&layout, &Words::new(&layout).unwrap()).unwrap()
    }

    #[test]
    fn the_error_timer_is_read_in_seconds_from_milliseconds() {
        let layout = pipboy_layout();
        let milliseconds = single(&layout, &ERROR_TIMER_MS).unwrap();
        assert_eq!(info().error_seconds, milliseconds / 1000.0);
    }

    #[test]
    fn the_menus_words_are_resolved_through_the_translation_table() {
        let info = info();
        assert!(!info.confirm_prompt.is_empty());
        assert!(!info.requirements_not_met.is_empty());
        assert_ne!(info.confirm_prompt, info.requirements_not_met);
    }

    #[test]
    fn the_background_carries_its_fill_and_the_extent_of_its_path() {
        let background = info().rules.background;
        assert!(background.fill[3] > 0);
        assert!(background.edge_bounds[2] > background.edge_bounds[0]);
    }

    #[test]
    fn the_fields_keep_the_y_the_movie_gives_them() {
        let info = info();
        assert_eq!(info.rules.name_y, info.name.local_y());
        assert_eq!(info.rules.confirm_y, info.confirm.local_y());
    }
}
