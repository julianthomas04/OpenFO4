//! Which music types apply at the player's place. Pure data lookup, no I/O.
//!
//! Four sources are consulted: the cell's own music, the music of the cell's own location, the
//! sound regions of the place, and the worldspace.
//!
//! Inferred rules, none of them stated by the game data:
//! - A location's music applies only through a cell that names that location; the location's
//!   parents are not consulted.
//! - An interior cell has no regions and no worldspace music; only its cell and location count (no
//!   interior cell carries a region list in the game data).
//!
//! Region rules (Creation Kit "Region" documentation, applied to the sound tab):
//! - Among overlapping regions the highest region priority wins.
//! - The override flag marks the regions that take part in the competition: if any overlapping
//!   region has it, only those compete; otherwise all do. This is the reading of "the box must be on
//!   for the value to be taken into account by any other region"; it is not confirmed for music.
//! - UNPROVEN: when the top priority is shared, the region whose record comes first in the plugin
//!   (lowest `record_order`) wins, whatever its music. Neither the documentation nor the data says
//!   how a tie resolves.
//! - Region priority is never compared with music type priority.

use std::collections::HashMap;

use super::music_stack::PlaceSource;
use crate::assets::db::music::{CellPlace, MusicCatalog, RegionMusic};

#[derive(Debug, Clone, PartialEq)]
pub struct PlaceSnapshot {
    pub worldspace: String,
    pub interior_cell: Option<String>,
    pub grid: (i32, i32),
    pub point_beth: (f32, f32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub music_type: String,
    pub source: PlaceSource,
    pub priority: u8,
}

#[derive(Debug, Clone, PartialEq)]
struct SoundRegion {
    music: RegionMusic,
    areas: Vec<Vec<(f32, f32)>>,
}

impl SoundRegion {
    /// A region without a worldspace never contains a point.
    fn contains(&self, worldspace: &str, point: (f32, f32)) -> bool {
        self.music.worldspace.as_deref() == Some(worldspace)
            && self.areas.iter().any(|area| polygon_contains(area, point))
    }
}

#[derive(Debug, Clone, Default)]
pub struct PlaceIndex {
    interior_cells: HashMap<String, CellPlace>,
    exterior_cells: HashMap<(String, i32, i32), CellPlace>,
    location_music: HashMap<String, String>,
    worldspace_music: HashMap<String, String>,
    regions: Vec<SoundRegion>,
    music_type_priority: HashMap<String, u8>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PlaceError {
    #[error("music type {0} is referenced by a place but has no music type record")]
    UnknownMusicType(String),
    #[error("exterior cell {0} names music or a location but has no worldspace or grid position")]
    ExteriorCellWithoutPosition(String),
    #[error("exterior cells {first} and {second} share one grid position")]
    DuplicateExteriorCell { first: String, second: String },
}

impl PlaceIndex {
    /// The one place where catalog rows become the lookup structure.
    pub fn from_catalog(catalog: &MusicCatalog) -> Result<Self, PlaceError> {
        let mut index = Self {
            location_music: catalog.location_music.clone(),
            worldspace_music: catalog.worldspace_music.clone(),
            music_type_priority: catalog
                .music_types
                .iter()
                .map(|music_type| (music_type.form_key.clone(), music_type.priority))
                .collect(),
            ..Self::default()
        };
        index.add_cells(catalog)?;
        index.add_regions(catalog);
        Ok(index)
    }

    fn add_cells(&mut self, catalog: &MusicCatalog) -> Result<(), PlaceError> {
        let mut exterior_owner: HashMap<(String, i32, i32), &str> = HashMap::new();
        for place in &catalog.cell_places {
            if place.is_interior {
                self.interior_cells
                    .insert(place.cell.clone(), place.clone());
                continue;
            }
            let (Some(worldspace), Some((grid_x, grid_y))) = (&place.worldspace, place.grid) else {
                return Err(PlaceError::ExteriorCellWithoutPosition(place.cell.clone()));
            };
            let key = (worldspace.clone(), grid_x, grid_y);
            if let Some(first) = exterior_owner.insert(key.clone(), &place.cell) {
                return Err(PlaceError::DuplicateExteriorCell {
                    first: first.to_string(),
                    second: place.cell.clone(),
                });
            }
            self.exterior_cells.insert(key, place.clone());
        }
        Ok(())
    }

    fn add_regions(&mut self, catalog: &MusicCatalog) {
        let mut areas: HashMap<&str, Vec<Vec<(f32, f32)>>> = HashMap::new();
        for area in &catalog.region_areas {
            areas
                .entry(area.region.as_str())
                .or_default()
                .push(area.points.clone());
        }
        self.regions = catalog
            .region_music
            .iter()
            .map(|music| SoundRegion {
                areas: areas.remove(music.region.as_str()).unwrap_or_default(),
                music: music.clone(),
            })
            .collect();
    }

    pub fn resolve(&self, place: &PlaceSnapshot) -> Result<Vec<Candidate>, PlaceError> {
        let mut found: Vec<(&str, PlaceSource)> = Vec::new();
        let cell = self.cell(place);
        if let Some(music) = cell.and_then(|cell| cell.music_type.as_deref()) {
            found.push((music, PlaceSource::Cell));
        }
        if let Some(music) = cell
            .and_then(|cell| cell.location.as_deref())
            .and_then(|location| self.location_music.get(location))
        {
            found.push((music, PlaceSource::Location));
        }
        if place.interior_cell.is_none() {
            if let Some(music) = self.region_music_at_point(&place.worldspace, place.point_beth) {
                found.push((music, PlaceSource::Region));
            }
            if let Some(music) = self.worldspace_music.get(&place.worldspace) {
                found.push((music, PlaceSource::Worldspace));
            }
        }
        found
            .into_iter()
            .map(|(music, source)| self.candidate(music, source))
            .collect()
    }

    fn candidate(&self, music: &str, source: PlaceSource) -> Result<Candidate, PlaceError> {
        let priority = *self
            .music_type_priority
            .get(music)
            .ok_or_else(|| PlaceError::UnknownMusicType(music.to_string()))?;
        Ok(Candidate {
            music_type: music.to_string(),
            source,
            priority,
        })
    }

    fn cell(&self, place: &PlaceSnapshot) -> Option<&CellPlace> {
        match &place.interior_cell {
            Some(name) => self.interior_cells.get(name),
            None => {
                self.exterior_cells
                    .get(&(place.worldspace.clone(), place.grid.0, place.grid.1))
            }
        }
    }

    fn region_music_at_point(&self, worldspace: &str, point: (f32, f32)) -> Option<&str> {
        let containing: Vec<&RegionMusic> = self
            .regions
            .iter()
            .filter(|region| region.contains(worldspace, point))
            .map(|region| &region.music)
            .collect();
        winning_region_music(&containing)
    }
}

fn winning_region_music<'a>(containing: &[&'a RegionMusic]) -> Option<&'a str> {
    let overriding: Vec<&RegionMusic> = containing
        .iter()
        .copied()
        .filter(|region| region.override_flag)
        .collect();
    let competing = if overriding.is_empty() {
        containing
    } else {
        &overriding
    };
    let top = competing.iter().map(|region| region.priority).max()?;
    competing
        .iter()
        .filter(|region| region.priority == top)
        .min_by_key(|region| region.record_order)
        .map(|region| region.music_type.as_str())
}

/// Ray casting with the boundary counted as inside.
pub fn polygon_contains(polygon: &[(f32, f32)], point: (f32, f32)) -> bool {
    let (px, py) = (f64::from(point.0), f64::from(point.1));
    let mut inside = false;
    for (index, &(ax, ay)) in polygon.iter().enumerate() {
        let (bx, by) = polygon[(index + 1) % polygon.len()];
        let (ax, ay, bx, by) = (f64::from(ax), f64::from(ay), f64::from(bx), f64::from(by));
        if on_segment((ax, ay), (bx, by), (px, py)) {
            return true;
        }
        if (ay > py) != (by > py) && px < (bx - ax) * (py - ay) / (by - ay) + ax {
            inside = !inside;
        }
    }
    inside
}

fn on_segment(a: (f64, f64), b: (f64, f64), p: (f64, f64)) -> bool {
    let cross = (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
    cross == 0.0
        && p.0 >= a.0.min(b.0)
        && p.0 <= a.0.max(b.0)
        && p.1 >= a.1.min(b.1)
        && p.1 <= a.1.max(b.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORLD: &str = "Commonwealth";
    const SQUARE: [(f32, f32); 4] = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];

    fn square(offset: f32) -> Vec<(f32, f32)> {
        SQUARE.iter().map(|&(x, y)| (x + offset, y)).collect()
    }

    #[test]
    fn polygon_inside_outside_and_edges() {
        let polygon = square(0.0);
        assert!(polygon_contains(&polygon, (50.0, 50.0)));
        assert!(!polygon_contains(&polygon, (150.0, 50.0)));
        assert!(!polygon_contains(&polygon, (50.0, -0.5)));
        assert!(polygon_contains(&polygon, (100.0, 50.0)));
        assert!(polygon_contains(&polygon, (50.0, 0.0)));
        assert!(polygon_contains(&polygon, (0.0, 0.0)));
    }

    #[test]
    fn concave_polygon_excludes_its_notch() {
        let u_shape = [
            (0.0, 0.0),
            (30.0, 0.0),
            (30.0, 70.0),
            (70.0, 70.0),
            (70.0, 0.0),
            (100.0, 0.0),
            (100.0, 100.0),
            (0.0, 100.0),
        ];
        assert!(!polygon_contains(&u_shape, (50.0, 30.0)));
        assert!(polygon_contains(&u_shape, (50.0, 85.0)));
        assert!(polygon_contains(&u_shape, (10.0, 30.0)));
        assert!(polygon_contains(&u_shape, (90.0, 30.0)));
    }

    fn region(music: &str, priority: u8, override_flag: bool, offset: f32) -> SoundRegion {
        SoundRegion {
            music: RegionMusic {
                region: format!("{music}-region"),
                worldspace: Some(WORLD.to_string()),
                music_type: music.to_string(),
                priority,
                override_flag,
                record_order: 0,
            },
            areas: vec![square(offset)],
        }
    }

    fn with_order(mut region: SoundRegion, record_order: u32) -> SoundRegion {
        region.music.record_order = record_order;
        region
    }

    fn cell_place(music_type: Option<&str>, location: Option<&str>) -> CellPlace {
        CellPlace {
            cell: "cell".to_string(),
            music_type: music_type.map(str::to_string),
            location: location.map(str::to_string),
            worldspace: Some(WORLD.to_string()),
            grid: Some((3, -2)),
            is_interior: false,
        }
    }

    fn index_with(regions: Vec<SoundRegion>) -> PlaceIndex {
        let music_type_priority = [
            ("MUSzExplore", 90),
            ("MUSzPublic", 70),
            ("MUSDungeonA", 80),
            ("FoothillsSanctuary", 80),
            ("Vault111", 85),
        ]
        .into_iter()
        .map(|(name, priority)| (name.to_string(), priority))
        .collect();
        PlaceIndex {
            regions,
            music_type_priority,
            worldspace_music: HashMap::from([(WORLD.to_string(), "MUSzExplore".to_string())]),
            ..PlaceIndex::default()
        }
    }

    fn exterior(point: (f32, f32)) -> PlaceSnapshot {
        PlaceSnapshot {
            worldspace: WORLD.to_string(),
            interior_cell: None,
            grid: (3, -2),
            point_beth: point,
        }
    }

    fn sources(candidates: &[Candidate]) -> Vec<(&str, PlaceSource, u8)> {
        candidates
            .iter()
            .map(|c| (c.music_type.as_str(), c.source, c.priority))
            .collect()
    }

    #[test]
    fn exterior_gets_region_and_worldspace_candidates_with_music_type_priorities() {
        let index = index_with(vec![region("MUSzPublic", 55, false, 0.0)]);
        let found = index.resolve(&exterior((50.0, 50.0))).unwrap();
        assert_eq!(
            sources(&found),
            [
                ("MUSzPublic", PlaceSource::Region, 70),
                ("MUSzExplore", PlaceSource::Worldspace, 90),
            ]
        );
        let outside = index.resolve(&exterior((500.0, 50.0))).unwrap();
        assert_eq!(
            sources(&outside),
            [("MUSzExplore", PlaceSource::Worldspace, 90)]
        );
    }

    #[test]
    fn region_of_another_worldspace_is_ignored() {
        let mut other = region("MUSzPublic", 55, false, 0.0);
        other.music.worldspace = Some("Elsewhere".to_string());
        let found = index_with(vec![other])
            .resolve(&exterior((50.0, 50.0)))
            .unwrap();
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn region_without_a_worldspace_never_matches() {
        let mut nowhere = region("MUSzPublic", 55, false, 0.0);
        nowhere.music.worldspace = None;
        let found = index_with(vec![nowhere])
            .resolve(&exterior((50.0, 50.0)))
            .unwrap();
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn overlapping_regions_highest_region_priority_wins_and_is_not_music_priority() {
        // The higher RDAT priority wins although its MUSC priority number (90) is the worse one.
        let index = index_with(vec![
            region("MUSzPublic", 55, false, 0.0),
            region("MUSzExplore", 60, false, 10.0),
        ]);
        let found = index.resolve(&exterior((50.0, 50.0))).unwrap();
        assert_eq!(found[0].music_type, "MUSzExplore");
        assert_eq!(found[0].source, PlaceSource::Region);
    }

    #[test]
    fn tie_between_different_music_goes_to_the_region_first_in_the_plugin() {
        let tied = |first: u32, second: u32| {
            index_with(vec![
                with_order(region("MUSzPublic", 55, false, 0.0), first),
                with_order(region("MUSDungeonA", 55, false, 10.0), second),
            ])
        };
        let winner = |index: PlaceIndex| index.resolve(&exterior((50.0, 50.0))).unwrap()[0].clone();

        assert_eq!(winner(tied(4, 9)).music_type, "MUSzPublic");
        assert_eq!(winner(tied(9, 4)).music_type, "MUSDungeonA");
        assert_eq!(winner(tied(9, 4)).source, PlaceSource::Region);
    }

    #[test]
    fn a_higher_priority_beats_an_earlier_record() {
        let index = index_with(vec![
            with_order(region("MUSzPublic", 55, false, 0.0), 0),
            with_order(region("MUSDungeonA", 56, false, 10.0), 9),
        ]);
        let found = index.resolve(&exterior((50.0, 50.0))).unwrap();
        assert_eq!(found[0].music_type, "MUSDungeonA");
    }

    #[test]
    fn tie_between_regions_with_the_same_music_still_applies() {
        let index = index_with(vec![
            region("MUSzPublic", 55, false, 0.0),
            region("MUSzPublic", 55, false, 10.0),
        ]);
        let found = index.resolve(&exterior((50.0, 50.0))).unwrap();
        assert_eq!(found[0].music_type, "MUSzPublic");
    }

    #[test]
    fn override_flag_restricts_the_competition_to_overriding_regions() {
        let index = index_with(vec![
            region("MUSzPublic", 90, false, 0.0),
            region("MUSDungeonA", 10, true, 10.0),
        ]);
        let found = index.resolve(&exterior((50.0, 50.0))).unwrap();
        assert_eq!(found[0].music_type, "MUSDungeonA");
    }

    #[test]
    fn cell_music_and_the_cells_own_location_are_candidates() {
        let mut index = index_with(vec![]);
        index.exterior_cells.insert(
            (WORLD.to_string(), 3, -2),
            cell_place(Some("Vault111"), Some("SanctuaryHillsLocation")),
        );
        index.location_music.insert(
            "SanctuaryHillsLocation".to_string(),
            "FoothillsSanctuary".to_string(),
        );
        let found = index.resolve(&exterior((500.0, 50.0))).unwrap();
        assert_eq!(
            sources(&found),
            [
                ("Vault111", PlaceSource::Cell, 85),
                ("FoothillsSanctuary", PlaceSource::Location, 80),
                ("MUSzExplore", PlaceSource::Worldspace, 90),
            ]
        );
    }

    #[test]
    fn a_location_without_music_of_its_own_offers_nothing_even_when_a_parent_has_music() {
        let mut index = index_with(vec![]);
        index.exterior_cells.insert(
            (WORLD.to_string(), 3, -2),
            cell_place(None, Some("SanctuaryHillsLocation")),
        );
        index.location_music.insert(
            "CommonwealthLocation".to_string(),
            "FoothillsSanctuary".to_string(),
        );

        let found = index.resolve(&exterior((500.0, 50.0))).unwrap();

        assert_eq!(
            sources(&found),
            [("MUSzExplore", PlaceSource::Worldspace, 90)]
        );
    }

    #[test]
    fn interior_ignores_regions_and_worldspace() {
        let mut index = index_with(vec![region("MUSzPublic", 55, false, 0.0)]);
        index.interior_cells.insert(
            "Vault111Cryo".to_string(),
            CellPlace {
                is_interior: true,
                worldspace: None,
                grid: None,
                ..cell_place(Some("Vault111"), None)
            },
        );
        let place = PlaceSnapshot {
            worldspace: WORLD.to_string(),
            interior_cell: Some("Vault111Cryo".to_string()),
            grid: (0, 0),
            point_beth: (50.0, 50.0),
        };
        assert_eq!(
            sources(&index.resolve(&place).unwrap()),
            [("Vault111", PlaceSource::Cell, 85)]
        );
    }

    #[test]
    fn unknown_music_type_is_an_error() {
        let mut index = index_with(vec![]);
        index
            .worldspace_music
            .insert(WORLD.to_string(), "MUSNowhere".to_string());
        assert_eq!(
            index.resolve(&exterior((0.0, 0.0))),
            Err(PlaceError::UnknownMusicType("MUSNowhere".to_string()))
        );
    }
}
