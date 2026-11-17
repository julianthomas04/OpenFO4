//! Builds the Pip-Boy catalog (perks, item definitions, the Commonwealth world map) from the database.

use std::collections::{HashMap, HashSet};

use anyhow::Context;

use crate::assets::db::DbClient;
use crate::assets::db::item_components::ItemComponentRow;
use crate::assets::db::items::ItemRow;
use crate::assets::db::map_markers::MapMarkerRow;
use crate::assets::db::perks::{PerkChartRow, PerkRow};
use crate::assets::db::special_attributes::SpecialAttributeRow;
use crate::assets::layout::texture_key;
use crate::bethesda_space::position_to_engine;
use crate::game_flow::spawn_locations::COMMONWEALTH as COMMONWEALTH_WORLDSPACE;
use crate::sim::pipboy::character::{SpecialText, SpecialTexts};
use crate::sim::pipboy::inventory::{
    ApparelStats, ComponentAmount, DamageType, DamageValue, ItemDetails, WeaponStats,
};
use crate::sim::pipboy::perks::{PerkChartSlot, PerkRank};
use crate::sim::pipboy::{
    InventoryCategory, ItemDefinition, ItemKind, Limb, MapMarker, MapMarkerId, PerkDefinition,
    PerkId, PipBoyCatalog, Sex, Special, WorldMap, WorldMapProjection,
};

const MARKER_DISCOVERED_FLAG: u8 = 0x01;
const MARKER_CAN_TRAVEL_FLAG: u8 = 0x02;

/// Reads every definition the Pip-Boy needs. A database that predates the Pip-Boy tables yields
/// empty parts (logged through the schema's missing-group report), never an error.
pub fn load_pipboy_catalog(db: &DbClient) -> anyhow::Result<PipBoyCatalog> {
    let perks = build_perks(
        &db.get_perks().context("reading perks")?,
        &db.get_perk_chart().context("reading the perk chart")?,
    )?;
    let items = build_items(
        &db.get_items().context("reading items")?,
        &load_item_context(db)?,
    )?;
    let special_texts = build_special_texts(
        &db.get_special_attributes()
            .context("reading the SPECIAL attributes")?,
    )?;
    let world_map = db
        .get_worldspace_map_image(COMMONWEALTH_WORLDSPACE)
        .context("reading the Commonwealth map image")?
        .map(|image| {
            let markers = db
                .get_map_markers(COMMONWEALTH_WORLDSPACE)
                .context("reading the Commonwealth map markers")?;
            Ok::<_, anyhow::Error>(build_world_map(&image, &markers))
        })
        .transpose()?;
    tracing::info!(
        "Pip-Boy catalog: {} perks, {} items, {} map markers",
        perks.len(),
        items.len(),
        world_map.as_ref().map_or(0, |map| map.markers.len())
    );
    Ok(PipBoyCatalog {
        perks,
        items,
        special_texts,
        world_map,
    })
}

fn parse_special(name: &str) -> anyhow::Result<Special> {
    Ok(match name {
        "strength" => Special::Strength,
        "perception" => Special::Perception,
        "endurance" => Special::Endurance,
        "charisma" => Special::Charisma,
        "intelligence" => Special::Intelligence,
        "agility" => Special::Agility,
        "luck" => Special::Luck,
        other => anyhow::bail!("unknown SPECIAL name '{other}' in the perk chart"),
    })
}

/// The authored texts per SPECIAL attribute; attributes the database lacks keep empty texts.
pub fn build_special_texts(rows: &[SpecialAttributeRow]) -> anyhow::Result<SpecialTexts> {
    let mut texts = SpecialTexts::default();
    for row in rows {
        texts.0[parse_special(&row.special)? as usize] = SpecialText {
            name: row.name.clone().unwrap_or_default(),
            description: row.description.clone().unwrap_or_default(),
        };
    }
    Ok(texts)
}

/// Editor id stems (without the trailing rank number) of the perks that occupy a chart cell for
/// one sex only.
const MALE_ONLY_PERKS: [&str; 4] = ["actionboy", "aquaboy", "ladykiller", "partyboy"];
const FEMALE_ONLY_PERKS: [&str; 4] = ["actiongirl", "aquagirl", "blackwidow", "partygirl"];

fn perk_sex(editor_id: Option<&str>) -> Option<Sex> {
    let stem = editor_id?
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .to_ascii_lowercase();
    if MALE_ONLY_PERKS.contains(&stem.as_str()) {
        Some(Sex::Male)
    } else if FEMALE_ONLY_PERKS.contains(&stem.as_str()) {
        Some(Sex::Female)
    } else {
        None
    }
}

fn is_first_rank(row: &PerkRow) -> bool {
    row.first_rank.as_deref() == Some(row.form_key.as_str())
}

/// Perks as the player sees them: one definition per playable, visible, named rank chain. The
/// installer names each record's first rank (`first_rank`); the ranks are that record followed
/// through `next_perk` until the chain ends or returns to a rank already seen, so the recorded
/// rank count is not used. Chart rows that match no catalog perk are logged as an error.
pub fn build_perks(
    rows: &[PerkRow],
    chart: &[PerkChartRow],
) -> anyhow::Result<Vec<PerkDefinition>> {
    let by_key: HashMap<&str, &PerkRow> = rows.iter().map(|r| (r.form_key.as_str(), r)).collect();
    warn_about_merged_chains(rows, &by_key);
    let without_first_rank = rows.iter().filter(|r| r.first_rank.is_none()).count();
    if without_first_rank > 0 {
        tracing::error!(
            "Pip-Boy catalog: {without_first_rank} perk records carry no first rank; the perk data predates it. Fix: run the installer with `esm-only --tables perks,perk_chart`"
        );
    }

    let mut slots: HashMap<&str, PerkChartSlot> = HashMap::new();
    for cell in chart {
        slots.insert(
            cell.perk.as_str(),
            PerkChartSlot {
                special: parse_special(&cell.special)?,
                required_special: cell.required_special,
            },
        );
    }

    let mut heads: Vec<&PerkRow> = rows.iter().filter(|r| is_first_rank(r)).collect();
    heads.sort_by(|a, b| a.form_key.cmp(&b.form_key));
    let mut perks = Vec::new();
    let mut unnamed = 0usize;
    let mut placed = 0usize;
    for first in heads.into_iter().filter(|r| r.playable && !r.hidden) {
        let Some(name) = first.name.as_deref().filter(|n| !n.is_empty()) else {
            unnamed += 1;
            continue;
        };
        let chart_slot = slots.get(first.form_key.as_str()).copied();
        placed += usize::from(chart_slot.is_some());
        perks.push(PerkDefinition {
            id: PerkId(first.form_key.clone()),
            name: name.to_owned(),
            ranks: rank_chain(first, &by_key),
            vault_boy_swf: first.swf_path.clone().filter(|p| !p.is_empty()),
            chart: chart_slot,
            sex: perk_sex(first.editor_id.as_deref()),
        });
    }
    if unnamed > 0 {
        tracing::debug!("Pip-Boy catalog: skipped {unnamed} playable perks without a name");
    }
    if placed < slots.len() {
        let mut unmatched: Vec<&str> = slots
            .keys()
            .copied()
            .filter(|key| !perks.iter().any(|p| p.id.0 == *key))
            .collect();
        unmatched.sort_unstable();
        tracing::error!(
            "Pip-Boy catalog: {} perk chart rows match no playable first-rank perk and are left out: {}",
            unmatched.len(),
            unmatched.join(", ")
        );
    }
    Ok(perks)
}

/// A rank record that several records name as their next rank would merge two chains.
fn warn_about_merged_chains(rows: &[PerkRow], by_key: &HashMap<&str, &PerkRow>) {
    let mut linked_by: HashMap<&str, Vec<&str>> = HashMap::new();
    for row in rows {
        if let Some(next) = row.next_perk.as_deref().filter(|n| by_key.contains_key(n)) {
            linked_by.entry(next).or_default().push(&row.form_key);
        }
    }
    let mut merged: Vec<&str> = linked_by
        .iter()
        .filter(|(_, sources)| sources.len() > 1)
        .map(|(next, _)| *next)
        .collect();
    merged.sort_unstable();
    for next in merged {
        tracing::warn!(
            "Pip-Boy catalog: perk record {next} is the next rank of several records; their rank chains run into each other"
        );
    }
}

fn rank_chain(first: &PerkRow, by_key: &HashMap<&str, &PerkRow>) -> Vec<PerkRank> {
    let mut ranks = Vec::new();
    let mut seen = HashSet::new();
    let mut cursor = Some(first);
    while let Some(row) = cursor {
        if !seen.insert(row.form_key.as_str()) {
            break;
        }
        ranks.push(PerkRank {
            form_key: row.form_key.clone(),
            description: row.description.clone().unwrap_or_default(),
            required_level: row.level,
        });
        cursor = row
            .next_perk
            .as_deref()
            .and_then(|next| by_key.get(next).copied());
    }
    ranks
}

/// Editor id of the keyword that marks a miscellaneous item as a loose weapon mod.
pub const LOOSE_MOD_KEYWORD_EDITOR_ID: &str = "ObjectTypeLooseMod";

/// What the classification of miscellaneous items needs beyond the item rows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemContext {
    /// Form key of the loose-mod keyword; `None` when the database has no such keyword.
    pub loose_mod_keyword: Option<String>,
    /// The components of each item, by item form key; absent for items that list none.
    pub components: HashMap<String, Vec<ComponentAmount>>,
}

/// The kind of item a record signature stands for; `None` for signatures the Pip-Boy never lists.
pub(crate) fn item_kind_of(item_type: &str) -> Option<ItemKind> {
    Some(match item_type {
        "WEAP" => ItemKind::Weapon,
        "ARMO" => ItemKind::Armor,
        "ALCH" => ItemKind::Aid,
        "AMMO" => ItemKind::Ammo,
        "MISC" => ItemKind::Misc,
        "BOOK" => ItemKind::Book,
        "HOLT" => ItemKind::Holotape,
        "KEYM" => ItemKind::Key,
        _ => return None,
    })
}

/// The Pip-Boy tab of an item. A miscellaneous item is a mod when it carries the loose-mod
/// keyword, else junk when it lists components; the keyword wins, though the data never has both.
pub(crate) fn category_of(
    kind: ItemKind,
    keywords: Option<&[String]>,
    has_components: bool,
    loose_mod_keyword: Option<&str>,
) -> InventoryCategory {
    match kind {
        ItemKind::Weapon => InventoryCategory::Weapons,
        ItemKind::Armor => InventoryCategory::Apparel,
        ItemKind::Aid => InventoryCategory::Aid,
        ItemKind::Ammo => InventoryCategory::Ammo,
        ItemKind::Misc => {
            let is_loose_mod = loose_mod_keyword.is_some_and(|loose_mod| {
                keywords.is_some_and(|keywords| keywords.iter().any(|key| key == loose_mod))
            });
            if is_loose_mod {
                InventoryCategory::Mods
            } else if has_components {
                InventoryCategory::Junk
            } else {
                InventoryCategory::Misc
            }
        }
        ItemKind::Book | ItemKind::Holotape | ItemKind::Key => InventoryCategory::Misc,
    }
}

/// The component lists per item, each component named when `names` resolves its record and
/// otherwise shown by its form key.
fn component_amounts(
    rows: &[ItemComponentRow],
    names: &HashMap<String, String>,
) -> HashMap<String, Vec<ComponentAmount>> {
    let mut amounts: HashMap<String, Vec<ComponentAmount>> = HashMap::new();
    for row in rows {
        amounts
            .entry(row.item_form_key.clone())
            .or_default()
            .push(ComponentAmount {
                component: names.get(&row.component).unwrap_or(&row.component).clone(),
                count: row.count,
            });
    }
    amounts
}

/// Reads the loose-mod keyword and the item components; each one the database lacks leaves
/// its tab empty (the schema's missing-group report covers the tables).
fn load_item_context(db: &DbClient) -> anyhow::Result<ItemContext> {
    let loose_mod_keyword = db
        .get_keyword_form_key(LOOSE_MOD_KEYWORD_EDITOR_ID)
        .context("reading the loose-mod keyword")?;
    if loose_mod_keyword.is_none() {
        tracing::warn!(
            "no keyword '{LOOSE_MOD_KEYWORD_EDITOR_ID}' in the database; the Mods tab stays empty"
        );
    }
    let components = match db
        .get_item_components()
        .context("reading item components")?
    {
        Some(rows) => component_amounts(
            &rows,
            &db.get_component_names()
                .context("reading component names")?,
        ),
        None => HashMap::new(),
    };
    Ok(ItemContext {
        loose_mod_keyword,
        components,
    })
}

/// Biped slot flags (`slot - 30` is the bit) that make a piece cover a limb on the paper doll.
const HEAD_SLOT_BITS: [u32; 7] = [0, 1, 2, 16, 17, 18, 19];
const TORSO_SLOT_BITS: [u32; 3] = [3, 6, 11];
const LEFT_ARM_SLOT_BITS: [u32; 2] = [7, 12];
const RIGHT_ARM_SLOT_BITS: [u32; 2] = [8, 13];
const LEFT_LEG_SLOT_BITS: [u32; 2] = [9, 14];
const RIGHT_LEG_SLOT_BITS: [u32; 2] = [10, 15];

fn limbs_covered_by(slot_mask: u64) -> Vec<Limb> {
    let covers = |bits: &[u32]| bits.iter().any(|bit| slot_mask & (1 << bit) != 0);
    [
        (Limb::Head, &HEAD_SLOT_BITS[..]),
        (Limb::Torso, &TORSO_SLOT_BITS[..]),
        (Limb::LeftArm, &LEFT_ARM_SLOT_BITS[..]),
        (Limb::RightArm, &RIGHT_ARM_SLOT_BITS[..]),
        (Limb::LeftLeg, &LEFT_LEG_SLOT_BITS[..]),
        (Limb::RightLeg, &RIGHT_LEG_SLOT_BITS[..]),
    ]
    .into_iter()
    .filter(|(_, bits)| covers(bits))
    .map(|(limb, _)| limb)
    .collect()
}

fn physical(amount: u32) -> Vec<DamageValue> {
    if amount == 0 {
        return Vec::new();
    }
    vec![DamageValue {
        kind: DamageType::Physical,
        amount: amount as f32,
    }]
}

fn required<T>(item: &ItemRow, field: &str, value: Option<T>) -> anyhow::Result<T> {
    value.with_context(|| format!("{} '{}' has no {field}", item.item_type, item.form_key))
}

fn non_negative(item: &ItemRow, field: &str, value: i64) -> anyhow::Result<u32> {
    u32::try_from(value)
        .with_context(|| format!("{} '{}' has {field} {value}", item.item_type, item.form_key))
}

/// The biped slot mask of a stored slot value; the database keeps the 32 flag bits as a signed
/// integer.
fn slot_mask_of(item: &ItemRow, slot: i64) -> anyhow::Result<u64> {
    u32::try_from(slot)
        .or_else(|_| i32::try_from(slot).map(|signed| signed as u32))
        .map(u64::from)
        .with_context(|| format!("armor '{}' has body slot {slot}", item.form_key))
}

fn weapon_details(row: &ItemRow, names: &HashMap<&str, &str>) -> anyhow::Result<ItemDetails> {
    let weapon = required(row, "weapon record", row.weapon.as_ref())?;
    let damage = required(row, "damage", weapon.damage)?;
    Ok(ItemDetails::Weapon(WeaponStats {
        damage: physical(non_negative(row, "damage", damage)?),
        fire_rate: required(row, "fire rate", weapon.fire_rate)? as f32,
        range: required(row, "range", weapon.range)? as f32,
        accuracy: required(row, "accuracy", weapon.accuracy)? as f32,
        ammo_name: weapon
            .ammo_type
            .as_deref()
            .and_then(|key| names.get(key))
            .map(|name| (*name).to_owned()),
    }))
}

fn apparel_details(row: &ItemRow) -> anyhow::Result<ItemDetails> {
    let armor = required(row, "armor record", row.armor.as_ref())?;
    let rating = required(row, "armor rating", armor.armor_rating)?;
    let slot_mask = slot_mask_of(row, required(row, "body slot", armor.body_slot)?)?;
    Ok(ItemDetails::Apparel(ApparelStats {
        resistances: physical(non_negative(row, "armor rating", rating)?),
        covers: limbs_covered_by(slot_mask),
        slot_mask,
    }))
}

fn details_of(
    category: InventoryCategory,
    row: &ItemRow,
    names: &HashMap<&str, &str>,
    components: &[ComponentAmount],
) -> anyhow::Result<ItemDetails> {
    Ok(match category {
        InventoryCategory::Weapons => weapon_details(row, names)?,
        InventoryCategory::Apparel => apparel_details(row)?,
        InventoryCategory::Aid => ItemDetails::Aid(Vec::new()),
        InventoryCategory::Ammo => ItemDetails::Ammo,
        InventoryCategory::Misc => ItemDetails::Misc,
        InventoryCategory::Junk => ItemDetails::Junk(components.to_vec()),
        InventoryCategory::Mods => ItemDetails::Mod,
    })
}

/// One definition per named item of a category the Pip-Boy lists. An item whose record lacks a
/// stat its category shows, or holds an impossible one, fails the load.
pub fn build_items(rows: &[ItemRow], context: &ItemContext) -> anyhow::Result<Vec<ItemDefinition>> {
    let names: HashMap<&str, &str> = rows
        .iter()
        .filter_map(|r| Some((r.form_key.as_str(), r.name.as_deref()?)))
        .collect();
    rows.iter()
        .filter_map(|row| {
            let kind = item_kind_of(&row.item_type)?;
            let name = row.name.as_deref().filter(|n| !n.is_empty())?;
            Some((row, kind, name))
        })
        .map(|(row, kind, name)| {
            let components = context
                .components
                .get(&row.form_key)
                .map_or(&[][..], Vec::as_slice);
            let category = category_of(
                kind,
                row.keywords.as_deref(),
                !components.is_empty(),
                context.loose_mod_keyword.as_deref(),
            );
            Ok(ItemDefinition {
                form_key: row.form_key.clone(),
                kind,
                editor_id: row.editor_id.clone(),
                name: name.to_owned(),
                category,
                weight: row.weight as f32,
                value: non_negative(row, "value", row.value)?,
                details: details_of(category, row, &names, components)?,
            })
        })
        .collect()
}

pub fn build_world_map(map_image: &str, markers: &[MapMarkerRow]) -> WorldMap {
    WorldMap {
        worldspace: COMMONWEALTH_WORLDSPACE.to_owned(),
        texture_key: texture_key(map_image),
        projection: WorldMapProjection::COMMONWEALTH,
        markers: markers
            .iter()
            .map(|row| MapMarker {
                id: MapMarkerId(row.form_key.clone()),
                name: row.name.clone().unwrap_or_default(),
                marker_type: row.marker_type,
                flags: row.flags,
                position: position_to_engine(
                    row.position[0] as f32,
                    row.position[1] as f32,
                    row.position[2] as f32,
                ),
                discovered: row.flags & MARKER_DISCOVERED_FLAG != 0,
                can_fast_travel: row.flags & MARKER_CAN_TRAVEL_FLAG != 0,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::db::items::{ArmorRow, WeaponRow};

    /// A first-rank record; use [`in_chain`] for the later ranks.
    fn perk_row(key: &str, name: &str, next: Option<&str>, level: u32) -> PerkRow {
        PerkRow {
            form_key: key.to_owned(),
            editor_id: None,
            name: Some(name.to_owned()),
            description: Some(format!("{name} text")),
            next_perk: next.map(str::to_owned),
            level,
            playable: true,
            hidden: false,
            swf_path: Some("Components\\VaultBoys\\Perks\\X.swf".to_owned()),
            first_rank: Some(key.to_owned()),
        }
    }

    fn in_chain(head: &str, mut row: PerkRow) -> PerkRow {
        row.first_rank = Some(head.to_owned());
        row
    }

    fn with_editor_id(mut row: PerkRow, editor_id: &str) -> PerkRow {
        row.editor_id = Some(editor_id.to_owned());
        row
    }

    fn chart_row(perk: &str, special: &str, required: u8) -> PerkChartRow {
        PerkChartRow {
            perk: perk.to_owned(),
            special: special.to_owned(),
            required_special: required,
        }
    }

    fn item_row(key: &str, item_type: &str, name: Option<&str>) -> ItemRow {
        ItemRow {
            form_key: key.to_owned(),
            item_type: item_type.to_owned(),
            editor_id: Some(format!("{key}Id")),
            name: name.map(str::to_owned),
            value: 25,
            weight: 1.5,
            weapon: None,
            armor: None,
            model_path: None,
            keywords: None,
            is_marker: None,
        }
    }

    /// A row of a category whose stats the catalog requires, with every stat present.
    fn complete_row(key: &str, item_type: &str) -> ItemRow {
        let mut row = item_row(key, item_type, Some(key));
        match item_type {
            "WEAP" => row.weapon = Some(weapon_stats(Some(1), Some(1.0))),
            "ARMO" => {
                row.armor = Some(ArmorRow {
                    armor_rating: Some(1),
                    body_slot: Some(1),
                });
            }
            _ => {}
        }
        row
    }

    #[test]
    fn a_rank_chain_is_followed_through_next_perk_in_order() {
        let rows = [
            in_chain("A1", perk_row("A3", "Armorer", None, 21)),
            perk_row("A1", "Armorer", Some("A2"), 5),
            in_chain("A1", perk_row("A2", "Armorer", Some("A3"), 13)),
        ];
        let perks = build_perks(&rows, &[]).unwrap();
        assert_eq!(perks.len(), 1);
        let armorer = &perks[0];
        assert_eq!(armorer.id, PerkId("A1".to_owned()));
        let ranks: Vec<(&str, u32)> = armorer
            .ranks
            .iter()
            .map(|r| (r.form_key.as_str(), r.required_level))
            .collect();
        assert_eq!(ranks, [("A1", 5), ("A2", 13), ("A3", 21)]);
        assert_eq!(armorer.ranks[0].description, "Armorer text");
        assert!(armorer.vault_boy_swf.is_some());
    }

    #[test]
    fn unplayable_hidden_and_unnamed_perks_are_left_out() {
        let mut unplayable = perk_row("U", "Unplayable", None, 1);
        unplayable.playable = false;
        let mut hidden = perk_row("H", "Hidden", None, 1);
        hidden.hidden = true;
        let mut unnamed = perk_row("N", "x", None, 1);
        unnamed.name = None;
        let kept = perk_row("K", "Kept", None, 1);
        let perks = build_perks(&[unplayable, hidden, unnamed, kept], &[]).unwrap();
        assert_eq!(perks.len(), 1);
        assert_eq!(perks[0].name, "Kept");
    }

    #[test]
    fn a_closed_cycle_stops_on_return_and_ignores_the_recorded_rank_count() {
        let rows = [
            perk_row("Z1", "Blitz", Some("A2"), 1),
            in_chain("Z1", perk_row("A2", "Blitz", Some("M3"), 11)),
            in_chain("Z1", perk_row("M3", "Blitz", Some("Z1"), 31)),
        ];
        let perks = build_perks(&rows, &[]).unwrap();
        assert_eq!(perks.len(), 1);
        assert_eq!(perks[0].id, PerkId("Z1".to_owned()));
        let keys: Vec<&str> = perks[0].ranks.iter().map(|r| r.form_key.as_str()).collect();
        assert_eq!(keys, ["Z1", "A2", "M3"]);
    }

    #[test]
    fn the_stored_first_rank_decides_the_head_not_the_level_or_the_listing_order() {
        let rows = [
            in_chain("O1", perk_row("O2", "Open", None, 2)),
            perk_row("O1", "Open", Some("O2"), 20),
        ];
        let perks = build_perks(&rows, &[]).unwrap();
        assert_eq!(perks.len(), 1);
        assert_eq!(perks[0].id, PerkId("O1".to_owned()));
        let levels: Vec<u32> = perks[0].ranks.iter().map(|r| r.required_level).collect();
        assert_eq!(levels, [20, 2]);
    }

    #[test]
    fn records_without_a_first_rank_do_not_become_perks() {
        let mut orphan = perk_row("X", "Orphan", None, 1);
        orphan.first_rank = None;
        let perks = build_perks(&[orphan, perk_row("K", "Kept", None, 1)], &[]).unwrap();
        assert_eq!(perks.len(), 1);
        assert_eq!(perks[0].name, "Kept");
    }

    #[test]
    fn open_chains_and_dangling_links_terminate() {
        let rows = [
            perk_row("D", "Dangling", Some("missing"), 1),
            perk_row("O1", "Open", Some("O2"), 1),
            in_chain("O1", perk_row("O2", "Open", None, 9)),
        ];
        let perks = build_perks(&rows, &[]).unwrap();
        let by_name = |name: &str| perks.iter().find(|p| p.name == name).unwrap();
        assert_eq!(by_name("Dangling").ranks.len(), 1);
        assert_eq!(by_name("Open").ranks.len(), 2);
    }

    #[test]
    fn two_chains_running_into_one_rank_stay_separate_perks_with_their_stored_heads() {
        let rows = [
            perk_row("P1", "First", Some("T"), 1),
            perk_row("P2", "Second", Some("T"), 1),
            in_chain("P1", perk_row("T", "First", None, 9)),
        ];
        let perks = build_perks(&rows, &[]).unwrap();
        let ranks = |name: &str| perks.iter().find(|p| p.name == name).unwrap().ranks.len();
        assert_eq!((ranks("First"), ranks("Second")), (2, 2));
    }

    #[test]
    fn sex_specific_chart_perks_are_tagged_by_editor_id_whatever_the_language() {
        let rows = [
            with_editor_id(perk_row("B", "Frauenheld", None, 1), "LadyKiller01"),
            with_editor_id(perk_row("G", "Schwarze Witwe", None, 1), "BlackWidow01"),
            with_editor_id(perk_row("N", "Armorer", None, 1), "Armorer01"),
            perk_row("U", "Lady Killer", None, 1),
        ];
        let chart = [chart_row("B", "charisma", 2), chart_row("G", "charisma", 2)];
        let perks = build_perks(&rows, &chart).unwrap();
        let sex_of = |name: &str| perks.iter().find(|p| p.name == name).unwrap().sex;
        assert_eq!(sex_of("Frauenheld"), Some(Sex::Male));
        assert_eq!(sex_of("Schwarze Witwe"), Some(Sex::Female));
        assert_eq!(sex_of("Armorer"), None);
        assert_eq!(
            sex_of("Lady Killer"),
            None,
            "an English name without the editor id does not count"
        );
    }

    #[test]
    fn every_sex_specific_editor_id_stem_maps_to_its_sex() {
        for (stem, sex) in [
            ("ActionBoy02", Sex::Male),
            ("Aquaboy01", Sex::Male),
            ("LadyKiller03", Sex::Male),
            ("PartyBoy01", Sex::Male),
            ("ActionGirl01", Sex::Female),
            ("Aquagirl02", Sex::Female),
            ("BlackWidow02", Sex::Female),
            ("PartyGirl03", Sex::Female),
        ] {
            assert_eq!(perk_sex(Some(stem)), Some(sex), "{stem}");
        }
        assert_eq!(perk_sex(None), None);
    }

    #[test]
    fn chart_slots_join_on_the_first_rank_record() {
        let rows = [
            perk_row("A1", "Armorer", Some("A2"), 1),
            in_chain("A1", perk_row("A2", "Armorer", None, 9)),
            perk_row("M", "Magazine perk", None, 1),
        ];
        let chart = [chart_row("A1", "strength", 3)];
        let perks = build_perks(&rows, &chart).unwrap();
        let armorer = perks.iter().find(|p| p.name == "Armorer").unwrap();
        assert_eq!(
            armorer.chart,
            Some(PerkChartSlot {
                special: Special::Strength,
                required_special: 3
            })
        );
        assert_eq!(
            perks
                .iter()
                .find(|p| p.name == "Magazine perk")
                .unwrap()
                .chart,
            None
        );
    }

    #[test]
    fn a_chart_row_naming_a_later_rank_or_an_unknown_perk_places_nothing() {
        let rows = [
            perk_row("A1", "Armorer", Some("A2"), 1),
            in_chain("A1", perk_row("A2", "Armorer", None, 9)),
        ];
        let chart = [
            chart_row("A2", "strength", 3),
            chart_row("ghost", "luck", 1),
        ];
        let perks = build_perks(&rows, &chart).unwrap();
        assert_eq!(perks.len(), 1);
        assert_eq!(perks[0].chart, None);
    }

    #[test]
    fn an_unknown_special_name_in_the_chart_is_an_error() {
        let error = build_perks(&[], &[chart_row("A1", "charm", 3)]).unwrap_err();
        assert!(error.to_string().contains("charm"));
    }

    #[test]
    fn special_texts_land_on_their_attribute_and_missing_ones_stay_empty() {
        let row = |special: &str, name: &str, description: Option<&str>| SpecialAttributeRow {
            special: special.to_owned(),
            name: Some(name.to_owned()),
            description: description.map(str::to_owned),
        };
        let texts = build_special_texts(&[
            row("luck", "Luck", Some("Fortune")),
            row("strength", "Strength", None),
        ])
        .unwrap();
        assert_eq!(texts.get(Special::Luck).description, "Fortune");
        assert_eq!(texts.get(Special::Strength).name, "Strength");
        assert_eq!(texts.get(Special::Strength).description, "");
        assert_eq!(texts.get(Special::Agility), &SpecialText::default());
    }

    #[test]
    fn an_unknown_special_attribute_row_is_an_error() {
        let row = SpecialAttributeRow {
            special: "charm".to_owned(),
            name: None,
            description: None,
        };
        assert!(build_special_texts(&[row]).is_err());
    }

    #[test]
    fn every_special_name_parses() {
        let names = [
            "strength",
            "perception",
            "endurance",
            "charisma",
            "intelligence",
            "agility",
            "luck",
        ];
        let parsed: Vec<Special> = names.iter().map(|n| parse_special(n).unwrap()).collect();
        assert_eq!(parsed, Special::ALL);
    }

    #[test]
    fn item_types_map_to_inventory_categories() {
        let rows: Vec<ItemRow> = [
            "WEAP", "ARMO", "ALCH", "AMMO", "MISC", "BOOK", "HOLT", "KEYM", "LIGH",
        ]
        .iter()
        .map(|t| complete_row(t, t))
        .collect();
        let categories: Vec<InventoryCategory> = build_items(&rows, &ItemContext::default())
            .unwrap()
            .iter()
            .map(|d| d.category)
            .collect();
        assert_eq!(
            categories,
            [
                InventoryCategory::Weapons,
                InventoryCategory::Apparel,
                InventoryCategory::Aid,
                InventoryCategory::Ammo,
                InventoryCategory::Misc,
                InventoryCategory::Misc,
                InventoryCategory::Misc,
                InventoryCategory::Misc,
            ]
        );
    }

    #[test]
    fn record_signatures_map_to_item_kinds() {
        let kinds: Vec<Option<ItemKind>> = [
            "WEAP", "ARMO", "ALCH", "AMMO", "MISC", "BOOK", "HOLT", "KEYM", "LIGH", "INGR",
        ]
        .iter()
        .map(|signature| item_kind_of(signature))
        .collect();
        assert_eq!(
            kinds,
            [
                Some(ItemKind::Weapon),
                Some(ItemKind::Armor),
                Some(ItemKind::Aid),
                Some(ItemKind::Ammo),
                Some(ItemKind::Misc),
                Some(ItemKind::Book),
                Some(ItemKind::Holotape),
                Some(ItemKind::Key),
                None,
                None,
            ]
        );
    }

    const LOOSE_MOD: &str = "0L1:Test.esm";

    fn keywords(keys: &[&str]) -> Vec<String> {
        keys.iter().map(|key| (*key).to_owned()).collect()
    }

    #[test]
    fn a_misc_item_is_a_mod_with_the_loose_mod_keyword_junk_with_components_else_misc() {
        let loose = keywords(&["0K1:Test.esm", LOOSE_MOD]);
        let other = keywords(&["0K1:Test.esm"]);
        let cases = [
            (Some(&loose[..]), false, InventoryCategory::Mods),
            (Some(&other[..]), false, InventoryCategory::Misc),
            (Some(&other[..]), true, InventoryCategory::Junk),
            (None, true, InventoryCategory::Junk),
            (None, false, InventoryCategory::Misc),
            (Some(&[][..]), false, InventoryCategory::Misc),
            (Some(&loose[..]), true, InventoryCategory::Mods),
        ];
        for (item_keywords, has_components, expected) in cases {
            assert_eq!(
                category_of(
                    ItemKind::Misc,
                    item_keywords,
                    has_components,
                    Some(LOOSE_MOD)
                ),
                expected,
                "{item_keywords:?} {has_components}"
            );
        }
    }

    #[test]
    fn without_a_loose_mod_keyword_nothing_is_a_mod() {
        let loose = keywords(&[LOOSE_MOD]);
        assert_eq!(
            category_of(ItemKind::Misc, Some(&loose), false, None),
            InventoryCategory::Misc
        );
        assert_eq!(
            category_of(ItemKind::Misc, Some(&loose), true, None),
            InventoryCategory::Junk
        );
    }

    #[test]
    fn books_holotapes_and_keys_stay_misc_whatever_they_carry() {
        let loose = keywords(&[LOOSE_MOD]);
        for kind in [ItemKind::Book, ItemKind::Holotape, ItemKind::Key] {
            assert_eq!(
                category_of(kind, Some(&loose), true, Some(LOOSE_MOD)),
                InventoryCategory::Misc,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn the_other_kinds_have_their_own_tab() {
        let expectations = [
            (ItemKind::Weapon, InventoryCategory::Weapons),
            (ItemKind::Armor, InventoryCategory::Apparel),
            (ItemKind::Aid, InventoryCategory::Aid),
            (ItemKind::Ammo, InventoryCategory::Ammo),
        ];
        for (kind, expected) in expectations {
            assert_eq!(category_of(kind, None, true, Some(LOOSE_MOD)), expected);
        }
    }

    fn component_row(item: &str, component: &str, count: u32) -> ItemComponentRow {
        ItemComponentRow {
            item_form_key: item.to_owned(),
            component: component.to_owned(),
            count,
        }
    }

    #[test]
    fn components_are_named_when_their_record_resolves_and_keyed_otherwise() {
        let rows = [
            component_row("0A", "0C1", 3),
            component_row("0B", "0C1", 1),
            component_row("0A", "0C2", 8),
        ];
        let names = HashMap::from([("0C1".to_owned(), "Brass".to_owned())]);

        let amounts = component_amounts(&rows, &names);

        let of = |component: &str, count: u32| ComponentAmount {
            component: component.to_owned(),
            count,
        };
        assert_eq!(amounts["0A"], [of("Brass", 3), of("0C2", 8)]);
        assert_eq!(amounts["0B"], [of("Brass", 1)]);
        assert_eq!(amounts.len(), 2);
    }

    #[test]
    fn build_items_classifies_misc_rows_by_the_context_and_lists_junk_components() {
        let mut mod_row = item_row("M", "MISC", Some("Barrel"));
        mod_row.keywords = Some(keywords(&[LOOSE_MOD]));
        let junk_row = item_row("J", "MISC", Some("Wrench"));
        let plain_row = item_row("P", "MISC", Some("Doll"));
        let tape_row = item_row("T", "HOLT", Some("Tape"));
        let context = ItemContext {
            loose_mod_keyword: Some(LOOSE_MOD.to_owned()),
            components: HashMap::from([(
                "J".to_owned(),
                vec![ComponentAmount {
                    component: "Steel".to_owned(),
                    count: 2,
                }],
            )]),
        };

        let defs = build_items(&[mod_row, junk_row, plain_row, tape_row], &context).unwrap();

        let summary: Vec<_> = defs
            .iter()
            .map(|def| (def.form_key.as_str(), def.kind, def.category))
            .collect();
        assert_eq!(
            summary,
            [
                ("M", ItemKind::Misc, InventoryCategory::Mods),
                ("J", ItemKind::Misc, InventoryCategory::Junk),
                ("P", ItemKind::Misc, InventoryCategory::Misc),
                ("T", ItemKind::Holotape, InventoryCategory::Misc),
            ]
        );
        assert_eq!(defs[0].details, ItemDetails::Mod);
        assert_eq!(
            defs[1].details,
            ItemDetails::Junk(vec![ComponentAmount {
                component: "Steel".to_owned(),
                count: 2
            }])
        );
        assert_eq!(defs[2].details, ItemDetails::Misc);
        assert_eq!(defs[3].details, ItemDetails::Misc);
    }

    #[test]
    fn nameless_items_are_skipped_and_scalars_carried_over() {
        let mut priced = item_row("B", "MISC", Some("Thing"));
        priced.value = 5;
        let defs = build_items(
            &[
                item_row("A", "MISC", None),
                item_row("C", "MISC", Some("")),
                priced,
            ],
            &ItemContext::default(),
        )
        .unwrap();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].value, 5);
        assert!((defs[0].weight - 1.5).abs() < f32::EPSILON);
        assert_eq!(defs[0].editor_id.as_deref(), Some("BId"));
    }

    #[test]
    fn a_negative_value_fails_the_load_naming_the_item() {
        let mut debt = item_row("B", "MISC", Some("Debt"));
        debt.value = -5;

        let error = build_items(&[debt], &ItemContext::default())
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("'B'") && error.contains("value -5"),
            "{error}"
        );
    }

    #[test]
    fn weapon_rows_become_physical_damage_with_the_ammo_name_resolved() {
        let mut rifle = item_row("W", "WEAP", Some("Combat Rifle"));
        rifle.weapon = Some(WeaponRow {
            damage: Some(36),
            fire_rate: Some(7.5),
            range: Some(120.0),
            accuracy: Some(64.0),
            ammo_type: Some("AM".to_owned()),
        });
        let ammo = item_row("AM", "AMMO", Some(".45 Round"));
        let defs = build_items(&[rifle, ammo], &ItemContext::default()).unwrap();
        let ItemDetails::Weapon(stats) = &defs[0].details else {
            panic!("not a weapon");
        };
        assert_eq!(
            stats.damage,
            [DamageValue {
                kind: DamageType::Physical,
                amount: 36.0
            }]
        );
        assert!((stats.fire_rate - 7.5).abs() < f32::EPSILON);
        assert_eq!(stats.ammo_name.as_deref(), Some(".45 Round"));
    }

    fn weapon_stats(damage: Option<i64>, fire_rate: Option<f64>) -> WeaponRow {
        WeaponRow {
            damage,
            fire_rate,
            range: Some(1.0),
            accuracy: Some(1.0),
            ammo_type: None,
        }
    }

    #[test]
    fn a_weapon_with_zero_damage_has_no_damage_entries() {
        let mut bat = item_row("W", "WEAP", Some("Bat"));
        bat.weapon = Some(weapon_stats(Some(0), Some(1.0)));
        let ItemDetails::Weapon(stats) =
            &build_items(&[bat], &ItemContext::default()).unwrap()[0].details
        else {
            panic!("not a weapon");
        };
        assert_eq!(stats.damage, []);
        assert_eq!(stats.ammo_name, None);
    }

    #[test]
    fn a_weapon_missing_a_stat_or_its_record_fails_the_load_naming_the_item_and_stat() {
        let mut no_damage = item_row("W1", "WEAP", Some("A"));
        no_damage.weapon = Some(weapon_stats(None, Some(1.0)));
        let mut no_rate = item_row("W2", "WEAP", Some("B"));
        no_rate.weapon = Some(weapon_stats(Some(1), None));
        let mut negative = item_row("W3", "WEAP", Some("C"));
        negative.weapon = Some(weapon_stats(Some(-1), Some(1.0)));
        let no_record = item_row("W4", "WEAP", Some("D"));

        for (row, needle) in [
            (no_damage, "'W1' has no damage"),
            (no_rate, "'W2' has no fire rate"),
            (negative, "'W3' has damage -1"),
            (no_record, "'W4' has no weapon record"),
        ] {
            let error = build_items(&[row], &ItemContext::default())
                .unwrap_err()
                .to_string();
            assert!(error.contains(needle), "{error}");
        }
    }

    #[test]
    fn armor_rows_carry_the_slot_mask_resistance_and_covered_limbs() {
        let mut vest = item_row("V", "ARMO", Some("Vest"));
        vest.armor = Some(ArmorRow {
            armor_rating: Some(30),
            body_slot: Some((1 << 11) | (1 << 12)),
        });
        let ItemDetails::Apparel(stats) =
            &build_items(&[vest], &ItemContext::default()).unwrap()[0].details
        else {
            panic!("not apparel");
        };
        assert_eq!(stats.slot_mask, (1 << 11) | (1 << 12));
        assert_eq!(stats.covers, [Limb::Torso, Limb::LeftArm]);
        assert_eq!(stats.resistances.len(), 1);
    }

    #[test]
    fn armor_missing_a_stat_or_with_an_impossible_one_fails_the_load() {
        let armor = |rating: Option<i64>, slot: Option<i64>| {
            let mut row = item_row("V", "ARMO", Some("Vest"));
            row.armor = Some(ArmorRow {
                armor_rating: rating,
                body_slot: slot,
            });
            row
        };

        for (row, needle) in [
            (armor(None, Some(1)), "no armor rating"),
            (armor(Some(1), None), "no body slot"),
            (armor(Some(-3), Some(1)), "armor rating -3"),
            (armor(Some(1), Some(i64::MAX)), "body slot"),
            (item_row("V", "ARMO", Some("Vest")), "no armor record"),
        ] {
            let error = build_items(&[row], &ItemContext::default())
                .unwrap_err()
                .to_string();
            assert!(error.contains(needle), "{error}");
        }
    }

    #[test]
    fn a_body_slot_stored_with_its_top_bit_set_keeps_the_bit() {
        let mut skin = item_row("S", "ARMO", Some("Skin"));
        skin.armor = Some(ArmorRow {
            armor_rating: Some(0),
            body_slot: Some(i64::from(i32::MIN)),
        });

        let ItemDetails::Apparel(stats) =
            &build_items(&[skin], &ItemContext::default()).unwrap()[0].details
        else {
            panic!("not apparel");
        };

        assert_eq!(stats.slot_mask, 1 << 31);
    }

    #[test]
    fn markers_convert_to_engine_space_and_decode_their_flags() {
        let row = |key: &str, flags: u8| MapMarkerRow {
            form_key: key.to_owned(),
            worldspace: COMMONWEALTH_WORLDSPACE.to_owned(),
            name: Some(key.to_owned()),
            marker_type: 11,
            flags,
            position: [-80768.0, 88175.0, 500.0],
        };
        let map = build_world_map(
            "Interface\\Pip-Boy\\WorldMap_d.dds",
            &[row("none", 0x08), row("seen", 0x01), row("both", 0x03)],
        );
        assert_eq!(
            map.texture_key,
            "textures\\Interface\\Pip-Boy\\WorldMap_d.dds"
        );
        assert_eq!(map.worldspace, COMMONWEALTH_WORLDSPACE);
        let flags: Vec<(bool, bool)> = map
            .markers
            .iter()
            .map(|m| (m.discovered, m.can_fast_travel))
            .collect();
        assert_eq!(flags, [(false, false), (true, false), (true, true)]);
        let position = map.markers[0].position;
        assert!((position - glam::Vec3::new(-8076.8, 50.0, -8817.5)).length() < 0.01);
        assert_eq!(map.markers[0].marker_type, 11);
    }

    /// The facts about the converted game data the item tabs rest on: 620 miscellaneous bases list
    /// components, 1,297 carry the loose-mod keyword, none both.
    #[test]
    #[ignore = "needs an installation converted with misc_components (OPENFO4_INSTALL_DIR or OPENFO4_DB)"]
    fn the_installed_database_has_620_misc_bases_with_components_and_1297_loose_mods() {
        let db = crate::assets::db::test_support::installed_database();
        let misc: Vec<ItemRow> = db
            .get_items()
            .unwrap()
            .into_iter()
            .filter(|row| row.item_type == "MISC")
            .collect();
        let with_components: HashSet<String> = db
            .get_item_components()
            .unwrap()
            .expect("misc_components table")
            .into_iter()
            .map(|row| row.item_form_key)
            .collect();
        let loose_mod = db
            .get_keyword_form_key(LOOSE_MOD_KEYWORD_EDITOR_ID)
            .unwrap()
            .expect("the loose-mod keyword");
        let is_loose_mod = |row: &ItemRow| {
            row.keywords
                .as_deref()
                .is_some_and(|keywords| keywords.contains(&loose_mod))
        };

        let components = misc
            .iter()
            .filter(|row| with_components.contains(&row.form_key))
            .count();
        let loose_mods = misc.iter().filter(|row| is_loose_mod(row)).count();
        let both = misc
            .iter()
            .filter(|row| is_loose_mod(row) && with_components.contains(&row.form_key))
            .count();

        assert_eq!(components, 620);
        assert_eq!(loose_mods, 1297);
        assert_eq!(both, 0);
    }

    /// Facts about the converted item rows: 922 component entries, the loose-mod keyword's form
    /// key, 116 marker bases (114 MISC, 2 WEAP), and the keyword column's NULL / array split.
    #[test]
    #[ignore = "needs an installation converted with misc_components and items.is_marker (OPENFO4_INSTALL_DIR or OPENFO4_DB)"]
    fn the_installed_database_has_922_component_rows_116_marker_bases_and_the_loose_mod_keyword() {
        let db = crate::assets::db::test_support::installed_database();
        let rows = db.get_items().unwrap();

        let component_rows = db
            .get_item_components()
            .unwrap()
            .expect("misc_components table");
        let markers: Vec<&ItemRow> = rows.iter().filter(|r| r.is_marker == Some(true)).collect();
        let marker_count_of =
            |item_type: &str| markers.iter().filter(|r| r.item_type == item_type).count();
        let without_keywords = rows.iter().filter(|r| r.keywords.is_none()).count();
        let with_keywords = rows.iter().filter(|r| r.keywords.is_some()).count();

        assert_eq!(component_rows.len(), 922);
        assert_eq!(markers.len(), 116);
        assert_eq!(marker_count_of("MISC"), 114);
        assert_eq!(marker_count_of("WEAP"), 2);
        assert_eq!(
            db.get_keyword_form_key(LOOSE_MOD_KEYWORD_EDITOR_ID)
                .unwrap()
                .as_deref(),
            Some("135C17:Fallout4.esm")
        );
        assert_eq!(without_keywords, 1307);
        assert_eq!(with_keywords, 2769);
    }

    /// Developer check against a real install: every item the database lists converts, and the
    /// installed art manifest covers every marker type of the world map.
    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the installed database and art manifest"]
    fn the_installed_database_builds_the_catalog_and_the_art_covers_its_markers() {
        let install = std::env::var(crate::assets::INSTALL_DIR_VAR).expect("OPENFO4_INSTALL_DIR");
        let root = format!("{install}/{}", crate::assets::layout::ASSETS_DIR);
        let db = DbClient::new(&format!(
            "{}/data.db",
            crate::assets::layout::data_dir(&root)
        ))
        .unwrap();

        let catalog = load_pipboy_catalog(&db).unwrap();

        let map = catalog.world_map.as_ref().expect("the Commonwealth map");
        let library =
            crate::assets::pipboy_art::ArtLibrary::load(std::path::Path::new(&root)).unwrap();
        library
            .require_marker_clips(map.markers.iter().map(|marker| marker.marker_type))
            .unwrap();
        assert_ne!(catalog.items, []);
    }
}
