//! The Pip-Boy data the installer produced: the catalog read from the real `data.db` and the art
//! manifest. Ignored by default (needs a converted installation). Run with
//! `OPENFO4_DB=<install>/assets/data/data.db cargo test --test pipboy_real_install -- --ignored`.

use std::path::{Path, PathBuf};

use openfo4::assets::db::DbClient;
use openfo4::assets::pipboy_art::{ArtLibrary, FixedArt};
use openfo4::assets::pipboy_catalog::load_pipboy_catalog;
use openfo4::sim::pipboy::actions::{RADAWAY_FORM_KEY, STIMPAK_FORM_KEY};
use openfo4::sim::pipboy::{InventoryCategory, PerkDefinition, PipBoyCatalog, Sex, Special};

const COMMONWEALTH_MARKERS: usize = 310;
const CHART_PERKS: usize = 74;
const CHART_CELLS: usize = 70;

fn db_path() -> String {
    std::env::var("OPENFO4_DB").expect("set OPENFO4_DB to data.db")
}

fn asset_root(db_path: &str) -> PathBuf {
    Path::new(db_path)
        .parent()
        .and_then(Path::parent)
        .expect("data.db sits in <assets>/data")
        .to_path_buf()
}

fn real_catalog() -> PipBoyCatalog {
    let db = DbClient::new(&db_path()).expect("open data.db");
    load_pipboy_catalog(&db).expect("load the Pip-Boy catalog")
}

#[test]
#[ignore = "needs a converted installation (OPENFO4_DB)"]
fn the_commonwealth_map_has_every_marker_and_its_texture_exists() {
    let catalog = real_catalog();
    let map = catalog.world_map.expect("the Commonwealth has a world map");
    assert_eq!(map.markers.len(), COMMONWEALTH_MARKERS);

    let texture = asset_root(&db_path()).join(map.texture_key.replace('\\', "/"));
    let found = std::fs::metadata(&texture).is_ok()
        || texture
            .parent()
            .and_then(|dir| std::fs::read_dir(dir).ok())
            .is_some_and(|mut entries| {
                entries.any(|e| {
                    e.is_ok_and(|e| {
                        e.file_name()
                            .eq_ignore_ascii_case(texture.file_name().unwrap())
                    })
                })
            });
    assert!(found, "world map texture {} is missing", texture.display());
}

#[test]
#[ignore = "needs a converted installation (OPENFO4_DB)"]
fn the_perk_chart_has_seventy_cells_and_a_rank_chain_per_perk() {
    let catalog = real_catalog();
    let chart: Vec<_> = catalog.perks.iter().filter(|p| p.chart.is_some()).collect();
    assert_eq!(chart.len(), CHART_PERKS);
    let chart_rows = DbClient::new(&db_path())
        .expect("open data.db")
        .get_perk_chart()
        .expect("read the perk chart");
    assert_eq!(
        chart.len(),
        chart_rows.len(),
        "every chart row names a catalog perk"
    );
    assert!(chart.iter().all(|perk| !perk.ranks.is_empty()));
    let cells: std::collections::HashSet<_> = chart
        .iter()
        .filter_map(|perk| {
            perk.chart
                .map(|slot| (slot.special as u8, slot.required_special))
        })
        .collect();
    assert_eq!(cells.len(), CHART_CELLS);
}

#[test]
#[ignore = "needs a converted installation (OPENFO4_DB)"]
fn the_quick_use_form_keys_name_the_stimpak_and_radaway() {
    let catalog = real_catalog();
    let name_of = |key: &str| {
        catalog
            .items
            .iter()
            .find(|item| item.form_key == key)
            .map(|item| (item.name.as_str(), item.category))
    };
    assert_eq!(
        name_of(STIMPAK_FORM_KEY),
        Some(("Stimpak", InventoryCategory::Aid))
    );
    assert_eq!(
        name_of(RADAWAY_FORM_KEY),
        Some(("RadAway", InventoryCategory::Aid))
    );
}

#[test]
#[ignore = "needs a converted installation (OPENFO4_DB)"]
fn the_art_manifest_loads_with_every_fixed_clip() {
    let library = ArtLibrary::load(&asset_root(&db_path())).expect("load the art manifest");
    for art in FixedArt::all() {
        assert!(library.clip(&art.clip_id()).is_some(), "{art:?}");
    }
}

fn chart_cell(perk: &PerkDefinition) -> (Special, u8) {
    let slot = perk.chart.expect("a chart perk");
    (slot.special, slot.required_special)
}

#[test]
#[ignore = "needs a converted installation (OPENFO4_DB)"]
fn the_eight_sex_specific_perks_pair_up_on_shared_chart_cells() {
    let catalog = real_catalog();
    let sexed = |sex: Sex| -> Vec<&PerkDefinition> {
        catalog
            .perks
            .iter()
            .filter(|perk| perk.sex == Some(sex))
            .collect()
    };
    let (male, female) = (sexed(Sex::Male), sexed(Sex::Female));
    assert_eq!((male.len(), female.len()), (4, 4));
    for perk in male.iter().chain(&female) {
        assert!(perk.chart.is_some(), "{} sits on the chart", perk.name);
    }
    for perk in &male {
        let partners = female
            .iter()
            .filter(|other| chart_cell(other) == chart_cell(perk))
            .count();
        assert_eq!(
            partners, 1,
            "{} shares its cell with one female perk",
            perk.name
        );
    }
}

#[test]
#[ignore = "needs a converted installation (OPENFO4_DB)"]
fn well_known_perks_have_their_authored_rank_counts() {
    let catalog = real_catalog();
    for (name, ranks) in [
        ("Gun Nut", 4),
        ("Armorer", 4),
        ("Locksmith", 4),
        ("Hacker", 4),
        ("Rifleman", 5),
        ("Awareness", 1),
    ] {
        let perk = catalog
            .perks
            .iter()
            .find(|perk| perk.name == name && perk.chart.is_some())
            .unwrap_or_else(|| panic!("no chart perk named {name}"));
        assert_eq!(perk.ranks.len(), ranks, "{name}");
        assert!(
            perk.ranks
                .windows(2)
                .all(|pair| pair[0].required_level <= pair[1].required_level),
            "{name} ranks ascend in level"
        );
    }
}

#[test]
#[ignore = "needs a converted installation (OPENFO4_DB)"]
fn every_special_attribute_has_a_name_and_a_description() {
    let catalog = real_catalog();
    for special in Special::ALL {
        let text = catalog.special_texts.get(special);
        assert!(!text.name.is_empty(), "{special:?} name");
        assert!(!text.description.is_empty(), "{special:?} description");
    }
}
