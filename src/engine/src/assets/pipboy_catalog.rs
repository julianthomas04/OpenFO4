//! Builds the Pip-Boy catalog (perks, item definitions, the Commonwealth world map) from the database.

use std::collections::{HashMap, HashSet};

use anyhow::Context;

use crate::assets::db::DbClient;
use crate::assets::db::items::ItemRow;
use crate::assets::db::map_markers::MapMarkerRow;
use crate::assets::db::perks::{PerkChartRow, PerkRow};
use crate::assets::db::special_attributes::SpecialAttributeRow;
use crate::assets::layout::texture_key;
use crate::bethesda_space::position_to_engine;
use crate::game_flow::spawn_locations::COMMONWEALTH as COMMONWEALTH_WORLDSPACE;
use crate::sim::pipboy::character::{SpecialText, SpecialTexts};
use crate::sim::pipboy::inventory::{
    ApparelStats, DamageType, DamageValue, ItemDetails, WeaponStats,
};
use crate::sim::pipboy::perks::{PerkChartSlot, PerkRank};
use crate::sim::pipboy::{
    InventoryCategory, ItemDefinition, Limb, MapMarker, MapMarkerId, PerkDefinition, PerkId,
    PipBoyCatalog, Sex, Special, WorldMap, WorldMapProjection,
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
    let items = build_items(&db.get_items().context("reading items")?)?;
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

fn category_of(item_type: &str) -> Option<InventoryCategory> {
    Some(match item_type {
        "WEAP" => InventoryCategory::Weapons,
        "ARMO" => InventoryCategory::Apparel,
        "ALCH" => InventoryCategory::Aid,
        "AMMO" => InventoryCategory::Ammo,
        "MISC" | "BOOK" | "KEYM" => InventoryCategory::Misc,
        _ => return None,
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
) -> anyhow::Result<ItemDetails> {
    Ok(match category {
        InventoryCategory::Weapons => weapon_details(row, names)?,
        InventoryCategory::Apparel => apparel_details(row)?,
        InventoryCategory::Aid => ItemDetails::Aid(Vec::new()),
        InventoryCategory::Ammo => ItemDetails::Ammo,
        InventoryCategory::Misc => ItemDetails::Misc,
        InventoryCategory::Junk => ItemDetails::Junk(Vec::new()),
        InventoryCategory::Mods => ItemDetails::Mod,
    })
}

/// One definition per named item of a category the Pip-Boy lists. An item whose record lacks a
/// stat its category shows, or holds an impossible one, fails the load.
pub fn build_items(rows: &[ItemRow]) -> anyhow::Result<Vec<ItemDefinition>> {
    let names: HashMap<&str, &str> = rows
        .iter()
        .filter_map(|r| Some((r.form_key.as_str(), r.name.as_deref()?)))
        .collect();
    rows.iter()
        .filter_map(|row| {
            let category = category_of(&row.item_type)?;
            let name = row.name.as_deref().filter(|n| !n.is_empty())?;
            Some((row, category, name))
        })
        .map(|(row, category, name)| {
            Ok(ItemDefinition {
                form_key: row.form_key.clone(),
                editor_id: row.editor_id.clone(),
                name: name.to_owned(),
                category,
                weight: row.weight as f32,
                value: non_negative(row, "value", row.value)?,
                details: details_of(category, row, &names)?,
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
            "WEAP", "ARMO", "ALCH", "AMMO", "MISC", "BOOK", "KEYM", "LIGH",
        ]
        .iter()
        .map(|t| complete_row(t, t))
        .collect();
        let categories: Vec<InventoryCategory> = build_items(&rows)
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
            ]
        );
    }

    #[test]
    fn nameless_items_are_skipped_and_scalars_carried_over() {
        let mut priced = item_row("B", "MISC", Some("Thing"));
        priced.value = 5;
        let defs = build_items(&[
            item_row("A", "MISC", None),
            item_row("C", "MISC", Some("")),
            priced,
        ])
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

        let error = build_items(&[debt]).unwrap_err().to_string();

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
        let defs = build_items(&[rifle, ammo]).unwrap();
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
        let ItemDetails::Weapon(stats) = &build_items(&[bat]).unwrap()[0].details else {
            panic!("not a weapon");
        };
        assert!(stats.damage.is_empty());
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
            let error = build_items(&[row]).unwrap_err().to_string();
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
        let ItemDetails::Apparel(stats) = &build_items(&[vest]).unwrap()[0].details else {
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
            let error = build_items(&[row]).unwrap_err().to_string();
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

        let ItemDetails::Apparel(stats) = &build_items(&[skin]).unwrap()[0].details else {
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
        assert!(!catalog.items.is_empty());
    }
}
