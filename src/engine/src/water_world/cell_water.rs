//! The water surface of a loaded exterior cell: the full-detail water drawn near
//! the player, in place of the distant LOD quad.
//!
//! A loaded cell has a surface when its effective water height lies above the
//! lowest point of its land. The surface is a flat 4096 x 4096 quad at the cell
//! centre (`grid * 4096 + 2048`) and the cell's effective water height, with the
//! cell's water type, drawn with depth, reflections and refractions and without
//! the LOD flag.

use std::collections::HashMap;
use std::hash::BuildHasher;
use std::sync::Arc;

use super::height::{CellWaterState, WorldspaceChain, water_type};
use super::lod::CellPlane;
use crate::assets::db::water_placement::CellWater;
use crate::bethesda_space::UNITS_FROM_ENGINE;

/// An exterior cell the streamer holds in full detail.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoadedCell {
    pub x: i32,
    pub y: i32,
    /// Height of the lowest vertex of the cell's land, engine units.
    pub land_lowest_y: f32,
}

/// The surface of one loaded cell.
#[derive(Debug, Clone, PartialEq)]
pub struct CellSurface {
    pub plane: CellPlane,
    /// The cell's WATR (the water type rule).
    pub water_type: Arc<str>,
}

/// The cell's water height (game units) when the cell gets a surface of its
/// own: its effective height must lie strictly above its lowest land, so a
/// height equal to the lowest land gets none.
pub fn surface_height(
    state: CellWaterState,
    worldspace_default: f32,
    land_lowest: f32,
) -> Option<f32> {
    state
        .effective_height(worldspace_default)
        .filter(|&height| height > land_lowest)
}

/// The surfaces of the `loaded` cells of an exterior worldspace; `cells` holds
/// the water columns of every cell that has any.
pub fn surfaces_of<S: BuildHasher>(
    loaded: impl IntoIterator<Item = LoadedCell>,
    cells: &HashMap<(i32, i32), CellWater, S>,
    chain: &WorldspaceChain,
) -> Vec<CellSurface> {
    loaded
        .into_iter()
        .filter_map(|cell| {
            let water = cells.get(&(cell.x, cell.y))?;
            let height = surface_height(
                CellWaterState::of_exterior(water),
                chain.default_water_height(),
                cell.land_lowest_y * UNITS_FROM_ENGINE,
            )?;
            Some(CellSurface {
                plane: CellPlane::of_cell(cell.x, cell.y, height),
                water_type: Arc::from(water_type(water.water_type.as_deref(), Some(chain))),
            })
        })
        .collect()
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact in f32 or compared with an epsilon"
)]
mod tests {
    use super::*;
    use crate::assets::db::water_placement::CellWaterHeight;
    use crate::water_world::height::WorldspaceWaterRecord;

    fn chain() -> WorldspaceChain {
        WorldspaceChain::new(vec![WorldspaceWaterRecord {
            form_key: "ws".to_owned(),
            parent: None,
            parent_flags: 0,
            water_form: Some("ocean".to_owned()),
            lod_water_form: None,
            lod_water_height: None,
            default_water_height: 450.0,
            flags: 0,
        }])
    }

    fn water(has_water: bool, height: CellWaterHeight, water_type: Option<&str>) -> CellWater {
        CellWater {
            has_water,
            no_lod_water: false,
            height,
            water_type: water_type.map(str::to_owned),
            environment_map: None,
            currents: None,
        }
    }

    fn loaded(x: i32, y: i32, land_lowest_game_units: f32) -> LoadedCell {
        LoadedCell {
            x,
            y,
            land_lowest_y: land_lowest_game_units / UNITS_FROM_ENGINE,
        }
    }

    fn state(height: CellWaterHeight) -> CellWaterState {
        CellWaterState {
            interior: false,
            has_water: true,
            xclw: height,
        }
    }

    #[test]
    fn a_cell_gets_a_surface_only_when_its_land_dips_below_the_water() {
        let unset = state(CellWaterHeight::Unset);
        assert_eq!(surface_height(unset, 450.0, 100.0), Some(450.0));
        assert_eq!(surface_height(unset, 450.0, 449.9), Some(450.0));
        assert_eq!(surface_height(unset, 450.0, 450.0), None);
        assert_eq!(surface_height(unset, 450.0, 900.0), None);
    }

    #[test]
    fn the_cell_height_overrides_the_worldspace_default() {
        let high = state(CellWaterHeight::Level(1300.0));
        assert_eq!(surface_height(high, 450.0, 1000.0), Some(1300.0));
    }

    #[test]
    fn a_cell_without_the_has_water_flag_has_no_surface() {
        let dry = CellWaterState {
            has_water: false,
            ..state(CellWaterHeight::Unset)
        };
        assert_eq!(surface_height(dry, 450.0, -500.0), None);
    }

    #[test]
    fn surfaces_cover_the_cell_at_its_height_with_its_water_type() {
        let cells = HashMap::from([
            ((0, 0), water(true, CellWaterHeight::Unset, None)),
            (
                (1, 0),
                water(true, CellWaterHeight::Level(1300.0), Some("lake")),
            ),
        ]);
        let surfaces = surfaces_of([loaded(0, 0, 0.0), loaded(1, 0, 1000.0)], &cells, &chain());
        assert_eq!(surfaces.len(), 2);
        let ocean = surfaces.iter().find(|s| s.plane.cell == (0, 0)).unwrap();
        assert_eq!(&*ocean.water_type, "ocean");
        assert!((ocean.plane.height - 45.0).abs() < 1e-4);
        let lake = surfaces.iter().find(|s| s.plane.cell == (1, 0)).unwrap();
        assert_eq!(&*lake.water_type, "lake");
        assert!((lake.plane.height - 130.0).abs() < 1e-4);
    }

    #[test]
    fn loaded_cells_without_a_water_record_or_with_high_land_get_none() {
        let cells = HashMap::from([((0, 0), water(true, CellWaterHeight::Unset, None))]);
        let surfaces = surfaces_of(
            [loaded(5, 5, -100.0), loaded(0, 0, 450.0)],
            &cells,
            &chain(),
        );
        assert!(surfaces.is_empty());
    }
}
