//! What the game's native code hands the inventory menu and its script only renders: which rows an
//! item card holds and in which order, their label keys and decimal places, the rating of a
//! difference, the owner text of the component view, the doll's underwear and the favourites glyph
//! of an item. None of it is in the game's data, so each is one named stand-in here.

use crate::sim::pipboy::inventory::{InventoryItem, ItemDetails};
use crate::ui_layout::TranslateKey;

/// A plain stat row: the translate key of its label and the decimals its value keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatSpec {
    pub label: &'static str,
    pub precision: u8,
}

pub const FIRE_RATE: StatSpec = StatSpec {
    label: "$ROF",
    precision: 0,
};
pub const RANGE: StatSpec = StatSpec {
    label: "$rng",
    precision: 0,
};
pub const ACCURACY: StatSpec = StatSpec {
    label: "$acc",
    precision: 0,
};
pub const WEIGHT: StatSpec = StatSpec {
    label: "$wt",
    precision: 1,
};
pub const VALUE: StatSpec = StatSpec {
    label: "$val",
    precision: 0,
};
/// The label of an aid effect that restores health.
pub const HEALTH: StatSpec = StatSpec {
    label: "$HP",
    precision: 0,
};
/// The decimals of a damage or resistance chip.
pub const CHIP_PRECISION: u8 = 0;

/// Every label key the stand-ins use, for the boot check.
pub static LABEL_KEYS: [TranslateKey; 6] = [
    TranslateKey::text(FIRE_RATE.label),
    TranslateKey::text(RANGE.label),
    TranslateKey::text(ACCURACY.label),
    TranslateKey::text(WEIGHT.label),
    TranslateKey::text(VALUE.label),
    TranslateKey::text(HEALTH.label),
];

/// How much a difference rates, in steps of the baseline: the relative change below which a
/// difference rates one step and below which it rates two; anything larger rates three.
pub const RATING_RELATIVE_STEPS: [f32; 2] = [0.1, 0.3];

/// The rating `-3..=3` of a change from `baseline` to `own`; `0` when they are equal.
pub fn diff_rating(own: f32, baseline: f32) -> i8 {
    let difference = own - baseline;
    if difference == 0.0 {
        return 0;
    }
    let change = if baseline > 0.0 {
        difference.abs() / baseline
    } else {
        f32::INFINITY
    };
    let steps = 1 + RATING_RELATIVE_STEPS
        .iter()
        .filter(|limit| change >= **limit)
        .count() as i8;
    steps * difference.signum() as i8
}

/// The text one owner shows in the component view.
pub fn owner_text(item_name: &str, amount: u32) -> String {
    format!("{item_name} ({amount})")
}

/// Whether the favourites menu may take `item`.
pub fn can_favourite(item: &InventoryItem) -> bool {
    matches!(
        item.details,
        ItemDetails::Weapon(_) | ItemDetails::Apparel(_) | ItemDetails::Aid(_)
    )
}

/// The favourites glyph (`1` is the empty tile) of an item.
pub const FAVOURITE_ICON_TYPE: u32 = 2;

pub const EMPTY_FAVOURITE_ICON_TYPE: u32 = 1;

/// Which base figure the doll shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnderwearType {
    Underwear,
    GenericClothes,
    VaultSuit,
}

pub const UNDERWEAR_TYPE: UnderwearType = UnderwearType::Underwear;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rating_steps_up_with_the_relative_change_and_keeps_the_sign() {
        assert_eq!(diff_rating(10.0, 10.0), 0);
        assert_eq!(diff_rating(10.5, 10.0), 1);
        assert_eq!(diff_rating(12.0, 10.0), 2);
        assert_eq!(diff_rating(20.0, 10.0), 3);
        assert_eq!(diff_rating(5.0, 10.0), -3);
        assert_eq!(diff_rating(9.5, 10.0), -1);
        assert_eq!(diff_rating(1.0, 0.0), 3);
    }
}
