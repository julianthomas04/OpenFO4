//! Static definitions the Pip-Boy model is built from, loaded once from the database at startup.

use super::character::SpecialTexts;
use super::inventory::{InventoryCategory, ItemDetails};
use super::map::WorldMap;
use super::perks::PerkDefinition;

#[derive(Clone, Debug, PartialEq)]
pub struct ItemDefinition {
    pub form_key: String,
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
