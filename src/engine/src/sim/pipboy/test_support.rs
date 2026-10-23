//! Builders for the Pip-Boy unit tests.

use std::sync::Arc;

use super::character::SpecialTexts;
use super::inventory::{
    AidEffect, ApparelStats, DamageType, DamageValue, InventoryCategory, InventoryItem,
    ItemDetails, ItemId, WeaponStats,
};
use super::map::{MapMarker, MapMarkerId, WorldMap, WorldMapProjection};
use super::perks::{PerkChartSlot, PerkDefinition, PerkId, PerkRank};
use super::{PipBoyCatalog, PipBoyModel, Special};

pub fn item(name: &str, category: InventoryCategory, details: ItemDetails) -> InventoryItem {
    InventoryItem {
        id: ItemId(0),
        form_key: None,
        name: name.to_owned(),
        category,
        count: 1,
        weight: 1.0,
        value: 10,
        equipped: false,
        favourite: None,
        legendary: false,
        details,
    }
}

pub fn weapon(name: &str, damage: f32, fire_rate: f32) -> InventoryItem {
    item(
        name,
        InventoryCategory::Weapons,
        ItemDetails::Weapon(WeaponStats {
            damage: vec![DamageValue {
                kind: DamageType::Physical,
                amount: damage,
            }],
            fire_rate,
            range: 100.0,
            accuracy: 50.0,
            ammo_name: None,
        }),
    )
}

pub fn apparel(name: &str, slot_mask: u64) -> InventoryItem {
    item(
        name,
        InventoryCategory::Apparel,
        ItemDetails::Apparel(ApparelStats {
            resistances: Vec::new(),
            covers: Vec::new(),
            slot_mask,
        }),
    )
}

pub fn aid(name: &str, restores_hp: f32, duration_seconds: Option<f32>) -> InventoryItem {
    item(
        name,
        InventoryCategory::Aid,
        ItemDetails::Aid(vec![AidEffect {
            description: format!("{name} effect"),
            restores_hp,
            duration_seconds,
        }]),
    )
}

/// A chart perk whose ranks need the given levels.
pub fn perk(name: &str, special: Special, required_special: u8, levels: &[u32]) -> PerkDefinition {
    PerkDefinition {
        id: PerkId(format!("{name}:Fallout4.esm")),
        name: name.to_owned(),
        ranks: levels
            .iter()
            .enumerate()
            .map(|(index, &required_level)| PerkRank {
                form_key: format!("{name}{index}:Fallout4.esm"),
                description: String::new(),
                required_level,
            })
            .collect(),
        vault_boy_swf: None,
        chart: Some(PerkChartSlot {
            special,
            required_special,
        }),
        sex: None,
    }
}

pub fn marker(id: &str, discovered: bool, can_fast_travel: bool) -> MapMarker {
    MapMarker {
        id: MapMarkerId(id.to_owned()),
        name: id.to_owned(),
        marker_type: 1,
        flags: 0,
        position: glam::Vec3::ZERO,
        discovered,
        can_fast_travel,
    }
}

pub fn catalog_with(perks: Vec<PerkDefinition>, markers: Vec<MapMarker>) -> Arc<PipBoyCatalog> {
    Arc::new(PipBoyCatalog {
        perks,
        items: Vec::new(),
        special_texts: SpecialTexts::default(),
        world_map: Some(WorldMap {
            worldspace: "00003C:Fallout4.esm".to_owned(),
            texture_key: "textures\\map.dds".to_owned(),
            projection: WorldMapProjection::COMMONWEALTH,
            markers,
        }),
    })
}

pub fn empty_model() -> PipBoyModel {
    PipBoyModel::new(Arc::default())
}

/// Catalog of `GunNut` (INT 3, rank levels 5 and 15) and `Savant` (LUK 1, level 1) over a character
/// at level 10 with two unspent points and INT base 3.
pub fn perk_model() -> PipBoyModel {
    let catalog = catalog_with(
        vec![
            perk("GunNut", Special::Intelligence, 3, &[5, 15]),
            perk("Savant", Special::Luck, 1, &[1]),
        ],
        Vec::new(),
    );
    let mut model = PipBoyModel::new(catalog);
    model.character.progression.level = 10;
    model.character.progression.unspent_perk_points = 2;
    model.character.special.0[Special::Intelligence as usize].base = 3;
    model
}

pub fn gun_nut() -> PerkId {
    PerkId("GunNut:Fallout4.esm".to_owned())
}
