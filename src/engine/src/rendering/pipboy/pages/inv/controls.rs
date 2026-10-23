//! Which key triggers which inventory action. The one place the approved bindings are written
//! down: the page reads its input and its hint keys from here.

use crate::rendering::pipboy::tab::HintKey;
use crate::sim::pipboy::InventoryCategory;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Inspect,
    Drop,
    Favourite,
    Sort,
    CycleDamage,
    ComponentView,
}

impl Action {
    pub const ALL: [Action; 6] = [
        Action::Inspect,
        Action::Drop,
        Action::CycleDamage,
        Action::ComponentView,
        Action::Favourite,
        Action::Sort,
    ];

    /// The key `self` is bound to on `category`'s page, `None` when the page has no such action.
    pub fn key(self, category: InventoryCategory) -> Option<HintKey> {
        match (self, category) {
            (Action::Inspect, _) => Some(HintKey::X),
            (Action::Drop, _) => Some(HintKey::R),
            (Action::Favourite, _) => Some(HintKey::Q),
            (Action::Sort, InventoryCategory::Apparel)
            | (Action::ComponentView, InventoryCategory::Junk) => Some(HintKey::Z),
            (Action::Sort, _) | (Action::CycleDamage, InventoryCategory::Apparel) => {
                Some(HintKey::C)
            }
            (Action::CycleDamage | Action::ComponentView, _) => None,
        }
    }

    /// The action `key` triggers on `category`'s page.
    pub fn of_key(category: InventoryCategory, key: HintKey) -> Option<Action> {
        Action::ALL
            .into_iter()
            .find(|action| action.key(category) == Some(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apparel_swaps_the_sort_and_cycle_keys_and_junk_gets_the_component_key() {
        use InventoryCategory::{Apparel, Junk, Weapons};
        assert_eq!(Action::of_key(Weapons, HintKey::C), Some(Action::Sort));
        assert_eq!(
            Action::of_key(Apparel, HintKey::C),
            Some(Action::CycleDamage)
        );
        assert_eq!(Action::of_key(Apparel, HintKey::Z), Some(Action::Sort));
        assert_eq!(
            Action::of_key(Junk, HintKey::Z),
            Some(Action::ComponentView)
        );
        assert_eq!(Action::of_key(Weapons, HintKey::Z), None);
    }
}
