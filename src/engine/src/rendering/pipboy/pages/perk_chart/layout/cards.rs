//! The cards: which frame the menu sends a card's clip to for each state, the clips of the
//! SPECIAL row and the stray sprite the chart's grid holds.

use egui::{Pos2, Vec2};

use super::unresolved;
use crate::rendering::pipboy::pages::perk_chart_layout::COLUMNS;
use crate::ui_layout::{
    InstanceClassKey, KeyGroup, LiteralKey, PlacementKey, UiLayout, UiLayoutError,
};

literal!(PLAY_FRAMES, "PerkGrid.SetVBAnimState#$1.gotoAndPlay(0)", 2);
literal!(STOP_FRAMES, "PerkGrid.SetVBAnimState#$1.gotoAndStop(0)", 3);

pub(crate) static SPECIAL_CLASSES: [InstanceClassKey; COLUMNS] = [
    InstanceClassKey::new(super::MOVIE, "[PerkGrid]/PerksHolder_mc/PerkAnim_0_0"),
    InstanceClassKey::new(super::MOVIE, "[PerkGrid]/PerksHolder_mc/PerkAnim_0_1"),
    InstanceClassKey::new(super::MOVIE, "[PerkGrid]/PerksHolder_mc/PerkAnim_0_2"),
    InstanceClassKey::new(super::MOVIE, "[PerkGrid]/PerksHolder_mc/PerkAnim_0_3"),
    InstanceClassKey::new(super::MOVIE, "[PerkGrid]/PerksHolder_mc/PerkAnim_0_4"),
    InstanceClassKey::new(super::MOVIE, "[PerkGrid]/PerksHolder_mc/PerkAnim_0_5"),
    InstanceClassKey::new(super::MOVIE, "[PerkGrid]/PerksHolder_mc/PerkAnim_0_6"),
];

/// The sprite the grid holds at depth 1 besides its cards.
pub(crate) static STRAY: PlacementKey =
    PlacementKey::new(super::MOVIE, "[PerkGrid]/PerksHolder_mc/@1");

pub(crate) static KEYS: KeyGroup =
    KeyGroup(&[&PLAY_FRAMES, &STOP_FRAMES, &SPECIAL_CLASSES, &STRAY]);

/// Where a clip sits in grid units: its registration point and its scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub origin: Pos2,
    pub scale: Vec2,
}

pub struct CardLayout {
    /// The frame a held perk's card plays from (1-based).
    pub owned_from: u32,
    /// The frame a takeable perk's card holds (1-based).
    pub available_at: u32,
    /// The exported classes of the SPECIAL row's card clips, by column.
    pub special_classes: Vec<String>,
    pub stray: Anchor,
}

fn frame_number(value: f64, key: &LiteralKey) -> Result<u32, UiLayoutError> {
    if value < 1.0 {
        return Err(unresolved(key.path, "a frame number below 1"));
    }
    Ok(value as u32)
}

fn frame(layout: &UiLayout, key: &LiteralKey, index: usize) -> Result<u32, UiLayoutError> {
    frame_number(layout.lit_number(key, index)?, key)
}

pub fn load(layout: &UiLayout) -> Result<CardLayout, UiLayoutError> {
    let stray = layout.get(&STRAY)?.local.matrix;
    Ok(CardLayout {
        owned_from: frame(layout, &PLAY_FRAMES, 0)?,
        available_at: frame(layout, &STOP_FRAMES, 1)?,
        special_classes: SPECIAL_CLASSES
            .iter()
            .map(|key| layout.get(key))
            .collect::<Result<_, _>>()?,
        stray: Anchor {
            origin: Pos2::new(stray.tx, stray.ty),
            scale: Vec2::new(stray.a, stray.d),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::pipboy::test_layout::pipboy_layout;

    #[test]
    fn the_menus_frames_come_from_its_script_literals() {
        let layout = pipboy_layout();
        let cards = load(&layout).unwrap();
        assert_eq!(cards.owned_from, frame(&layout, &PLAY_FRAMES, 0).unwrap());
        assert_eq!(cards.available_at, frame(&layout, &STOP_FRAMES, 1).unwrap());
    }

    #[test]
    fn every_special_column_has_a_card_class() {
        let cards = load(&pipboy_layout()).unwrap();
        assert_eq!(cards.special_classes.len(), COLUMNS);
        assert!(cards.special_classes.iter().all(|class| !class.is_empty()));
    }

    #[test]
    fn a_frame_number_below_one_is_rejected() {
        assert!(frame_number(0.0, &PLAY_FRAMES).is_err());
        assert_eq!(frame_number(2.0, &PLAY_FRAMES).unwrap(), 2);
    }
}
