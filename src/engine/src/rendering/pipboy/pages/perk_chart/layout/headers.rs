//! The column headers: the SPECIAL names, their shadows and the counter stars below them.

use egui::Pos2;

use super::Text;
use crate::rendering::pipboy::pages::perk_chart_layout::COLUMNS;
use crate::ui_layout::{KeyGroup, PlacementKey, TextKey, TranslateKey, UiLayout, UiLayoutError};

pub(crate) static TEXTS: [TextKey; COLUMNS] = [
    TextKey::new(super::MOVIE, "[PerkGrid]/Header_mc/STR_tf"),
    TextKey::new(super::MOVIE, "[PerkGrid]/Header_mc/PER_tf"),
    TextKey::new(super::MOVIE, "[PerkGrid]/Header_mc/END_tf"),
    TextKey::new(super::MOVIE, "[PerkGrid]/Header_mc/CHA_tf"),
    TextKey::new(super::MOVIE, "[PerkGrid]/Header_mc/INT_tf"),
    TextKey::new(super::MOVIE, "[PerkGrid]/Header_mc/AGI_tf"),
    TextKey::new(super::MOVIE, "[PerkGrid]/Header_mc/LCK_tf"),
];
pub(crate) static SHADOWS: [TextKey; COLUMNS] = [
    TextKey::new(super::MOVIE, "[PerkGrid]/Header_mc/STRShadow_tf"),
    TextKey::new(super::MOVIE, "[PerkGrid]/Header_mc/PERShadow_tf"),
    TextKey::new(super::MOVIE, "[PerkGrid]/Header_mc/ENDShadow_tf"),
    TextKey::new(super::MOVIE, "[PerkGrid]/Header_mc/CHAShadow_tf"),
    TextKey::new(super::MOVIE, "[PerkGrid]/Header_mc/INTShadow_tf"),
    TextKey::new(super::MOVIE, "[PerkGrid]/Header_mc/AGIShadow_tf"),
    TextKey::new(super::MOVIE, "[PerkGrid]/Header_mc/LCKShadow_tf"),
];
pub(crate) static STARS: [PlacementKey; COLUMNS] = [
    PlacementKey::new(super::MOVIE, "[PerkGrid]/Header_mc/STRStars_mc"),
    PlacementKey::new(super::MOVIE, "[PerkGrid]/Header_mc/PERStars_mc"),
    PlacementKey::new(super::MOVIE, "[PerkGrid]/Header_mc/ENDStars_mc"),
    PlacementKey::new(super::MOVIE, "[PerkGrid]/Header_mc/CHAStars_mc"),
    PlacementKey::new(super::MOVIE, "[PerkGrid]/Header_mc/INTStars_mc"),
    PlacementKey::new(super::MOVIE, "[PerkGrid]/Header_mc/AGIStars_mc"),
    PlacementKey::new(super::MOVIE, "[PerkGrid]/Header_mc/LCKStars_mc"),
];
pub(crate) static LABELS: [TranslateKey; COLUMNS] = [
    TranslateKey::text("$STRENGTH"),
    TranslateKey::text("$PERCEPTION"),
    TranslateKey::text("$ENDURANCE"),
    TranslateKey::text("$CHARISMA"),
    TranslateKey::text("$INTELLIGENCE"),
    TranslateKey::text("$AGILITY"),
    TranslateKey::text("$LUCK"),
];

pub(crate) static KEYS: KeyGroup = KeyGroup(&[&TEXTS, &SHADOWS, &STARS, &LABELS]);

pub struct HeaderLayout {
    pub label: String,
    pub text: Text,
    pub shadow: Text,
    pub stars_origin: Pos2,
}

pub fn load(layout: &UiLayout) -> Result<Vec<HeaderLayout>, UiLayoutError> {
    (0..COLUMNS)
        .map(|column| {
            let matrix = layout.get(&STARS[column])?.world.matrix;
            Ok(HeaderLayout {
                label: layout.get(&LABELS[column])?,
                text: Text::load(layout, &TEXTS[column])?,
                shadow: Text::load(layout, &SHADOWS[column])?,
                stars_origin: Pos2::new(matrix.tx, matrix.ty),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::pipboy::test_layout::pipboy_layout;

    #[test]
    fn every_column_has_a_label_a_shadow_and_a_star_origin() {
        let layout = pipboy_layout();
        let headers = load(&layout).unwrap();
        assert_eq!(headers.len(), COLUMNS);
        assert!(headers.iter().all(|header| !header.label.is_empty()));
    }
}
