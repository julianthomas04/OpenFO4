//! Static definitions the Pip-Boy model is built from, loaded once from the database at startup.

use super::character::SpecialTexts;
use super::inventory::{InventoryCategory, ItemDetails};
use super::map::WorldMap;
use super::perks::PerkDefinition;

/// The record type an item definition comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Weapon,
    Armor,
    Aid,
    Ammo,
    Misc,
    Book,
    Holotape,
    Key,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemDefinition {
    pub form_key: String,
    pub kind: ItemKind,
    pub editor_id: Option<String>,
    pub name: String,
    pub category: InventoryCategory,
    pub weight: f32,
    pub value: u32,
    pub details: ItemDetails,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PipBoyCatalog {
    /// Every playable perk, chart and non-chart.
    pub perks: Vec<PerkDefinition>,
    pub items: Vec<ItemDefinition>,
    /// Name and description of each SPECIAL attribute.
    pub special_texts: SpecialTexts,
    /// The Commonwealth world map; `None` when the database lacks it.
    pub world_map: Option<WorldMap>,
}

impl PipBoyCatalog {
    /// Linear scan, called once per frame while an item is aimed and once per pickup or drop.
    pub fn item(&self, form_key: &str) -> Option<&ItemDefinition> {
        self.items.iter().find(|item| item.form_key == form_key)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::definition;
    use super::*;

    #[test]
    fn the_catalog_finds_an_item_by_form_key_and_misses_an_unknown_one() {
        let catalog = PipBoyCatalog {
            items: vec![
                definition("0AA001:Test.esm", "Pipe Wrench"),
                definition("0AA002:Test.esm", "Spanner"),
            ],
            ..PipBoyCatalog::default()
        };

        assert_eq!(catalog.item("0AA002:Test.esm").unwrap().name, "Spanner");
        assert_eq!(catalog.item("0AA003:Test.esm"), None);
    }
}
