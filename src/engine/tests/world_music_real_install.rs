//! The world music of a converted installation: where the player is decides the music type.
//!
//! Ignored by default (needs an installation converted with the music tables and audio). Run with
//! `cargo test --test world_music_real_install -- --ignored --nocapture`.

mod common;

use openfo4::assets::db::DbClient;
use openfo4::assets::db::music::MusicCatalog;
use openfo4::assets::layout::audio_dir;
use openfo4::audio::music_stack::PlaceSource;
use openfo4::audio::place::{Candidate, PlaceSnapshot, polygon_contains};
use openfo4::audio::world_music::WorldMusic;

const COMMONWEALTH: &str = "00003C:Fallout4.esm";
const DIAMOND_CITY: &str = "000F94:Fallout4.esm";
const SANCTUARY_HILLS_REGION: &str = "039C9D:Fallout4.esm";
const DOWNTOWN_REGION: &str = "23F1FD:Fallout4.esm";
const VAULT_111_CRYO: &str = "0016D8:Fallout4.esm";
const CONCORD_FACTORY: &str = "0E3CC4:Fallout4.esm";
const CELL_SIZE: f32 = 4096.0;
const SEARCH_HALF_EXTENT_CELLS: i32 = 60;
const AUDIT_EXAMPLES: usize = 20;
const SEED: u64 = 1;

fn exterior(worldspace: &str, point: (f32, f32)) -> PlaceSnapshot {
    PlaceSnapshot {
        worldspace: worldspace.to_string(),
        interior_cell: None,
        grid: (
            (point.0 / CELL_SIZE).floor() as i32,
            (point.1 / CELL_SIZE).floor() as i32,
        ),
        point_beth: point,
    }
}

fn interior(cell: &str) -> PlaceSnapshot {
    PlaceSnapshot {
        worldspace: String::new(),
        interior_cell: Some(cell.to_string()),
        grid: (0, 0),
        point_beth: (0.0, 0.0),
    }
}

const POLYGON_SCAN_STEP: f32 = 256.0;

/// A point inside the region's first polygon whose cell names neither music nor a location, so
/// the region's own music is what the place offers besides the worldspace's.
fn region_point(music: &WorldMusic, catalog: &MusicCatalog, region: &str) -> (f32, f32) {
    let area = catalog
        .region_areas
        .iter()
        .find(|area| area.region == region && area.points.len() >= 3)
        .unwrap_or_else(|| panic!("region {region} has no polygon"));
    let (min_x, max_x) = bounds(area.points.iter().map(|p| p.0));
    let (min_y, max_y) = bounds(area.points.iter().map(|p| p.1));
    let mut y = min_y;
    while y <= max_y {
        let mut x = min_x;
        while x <= max_x {
            let point = (x, y);
            if polygon_contains(&area.points, point) {
                let (candidates, _) = music.candidates(&exterior(COMMONWEALTH, point)).unwrap();
                if candidates.iter().all(|c| c.source >= PlaceSource::Region) {
                    return point;
                }
            }
            x += POLYGON_SCAN_STEP;
        }
        y += POLYGON_SCAN_STEP;
    }
    panic!("every scanned point of region {region} lies in a cell with music or a location");
}

fn bounds(values: impl Iterator<Item = f32>) -> (f32, f32) {
    values.fold((f32::MAX, f32::MIN), |(low, high), v| {
        (low.min(v), high.max(v))
    })
}

/// A Commonwealth point that no sound region contains and no cell names music for.
fn point_in_no_region(music: &WorldMusic) -> (f32, f32) {
    for x in -SEARCH_HALF_EXTENT_CELLS..SEARCH_HALF_EXTENT_CELLS {
        for y in -SEARCH_HALF_EXTENT_CELLS..SEARCH_HALF_EXTENT_CELLS {
            let point = (
                x as f32 * CELL_SIZE + CELL_SIZE / 2.0,
                y as f32 * CELL_SIZE + CELL_SIZE / 2.0,
            );
            let (candidates, _) = music.candidates(&exterior(COMMONWEALTH, point)).unwrap();
            if candidates.len() == 1 {
                return point;
            }
        }
    }
    panic!("every Commonwealth cell near the origin has a region or cell music");
}

struct Resolver<'a> {
    music: &'a WorldMusic,
}

impl Resolver<'_> {
    fn name(&self, key: &str) -> String {
        self.music
            .music_type(key)
            .and_then(|music_type| music_type.editor_id.clone())
            .unwrap_or_else(|| key.to_string())
    }

    fn describe(&self, candidate: &Candidate) -> String {
        format!(
            "{} ({:?}, priority {})",
            self.name(&candidate.music_type),
            candidate.source,
            candidate.priority
        )
    }

    /// Prints the candidates and returns the editor id of the picked music type.
    fn resolve(&self, label: &str, place: &PlaceSnapshot) -> String {
        let (candidates, picked) = self.music.candidates(place).unwrap();
        let listed: Vec<String> = candidates.iter().map(|c| self.describe(c)).collect();
        let picked = picked.expect("the place has a music type");
        let name = self.name(&picked.music_type);
        println!("{label}: {name}; candidates: {listed:?}");
        name
    }
}

/// The candidate of the most specific source, the alternative to the priority stack.
fn specificity_pick(candidates: &[Candidate]) -> Option<&Candidate> {
    candidates
        .iter()
        .min_by_key(|candidate| (candidate.source, candidate.priority))
}

fn audit_pick_disagreements(music: &WorldMusic, resolver: &Resolver, catalog: &MusicCatalog) {
    let mut disagreements = Vec::new();
    for cell in &catalog.cell_places {
        let place = match (&cell.worldspace, cell.grid, cell.is_interior) {
            (_, _, true) => interior(&cell.cell),
            (Some(worldspace), Some((grid_x, grid_y)), false) => exterior(
                worldspace,
                (
                    grid_x as f32 * CELL_SIZE + CELL_SIZE / 2.0,
                    grid_y as f32 * CELL_SIZE + CELL_SIZE / 2.0,
                ),
            ),
            _ => continue,
        };
        let (candidates, by_priority) = music.candidates(&place).unwrap();
        let by_specificity = specificity_pick(&candidates);
        if by_priority.as_ref().map(|c| &c.music_type) != by_specificity.map(|c| &c.music_type) {
            disagreements.push(format!(
                "cell {}: priority stack {} / specificity {}",
                cell.cell,
                resolver.describe(&by_priority.unwrap()),
                resolver.describe(by_specificity.unwrap()),
            ));
        }
    }
    println!(
        "audit: {} of {} cells with music or a location resolve differently by priority and by specificity",
        disagreements.len(),
        catalog.cell_places.len()
    );
    for line in disagreements.iter().take(AUDIT_EXAMPLES) {
        println!("  {line}");
    }
}

#[test]
#[ignore = "needs a converted installation with music tables and audio"]
fn places_of_the_installed_game_resolve_to_the_expected_music_types() {
    let db = DbClient::new(common::database_path().to_str().unwrap()).unwrap();
    let catalog = db.get_music_catalog().unwrap();
    let audio = audio_dir(common::asset_root().to_str().unwrap());
    let music = WorldMusic::new(catalog.clone(), std::path::Path::new(&audio), SEED).unwrap();
    let resolver = Resolver { music: &music };
    println!(
        "world music volume {:.2} dB, {} music files missing",
        music.volume().0,
        music.missing_files().len()
    );

    let sanctuary = exterior(
        COMMONWEALTH,
        region_point(&music, &catalog, SANCTUARY_HILLS_REGION),
    );
    assert_eq!(
        resolver.resolve("Sanctuary", &sanctuary),
        "MUSzExploreFoothillsSanctuary"
    );

    let downtown = exterior(
        COMMONWEALTH,
        region_point(&music, &catalog, DOWNTOWN_REGION),
    );
    assert_eq!(resolver.resolve("Downtown", &downtown), "MUSzExploreUrban");

    let open_country = exterior(COMMONWEALTH, point_in_no_region(&music));
    assert_eq!(
        resolver.resolve("Commonwealth, no region", &open_country),
        "MUSzExplore"
    );

    assert_eq!(
        resolver.resolve("Vault111Cryo", &interior(VAULT_111_CRYO)),
        "MUSDungeonVault111"
    );
    assert_eq!(
        resolver.resolve("ConcordFactory", &interior(CONCORD_FACTORY)),
        "MUSDungeonA"
    );

    let diamond_city = exterior(DIAMOND_CITY, (0.0, 0.0));
    assert_eq!(
        resolver.resolve("DiamondCity worldspace", &diamond_city),
        "MUSzPublic"
    );

    audit_pick_disagreements(&music, &resolver, &catalog);
}
