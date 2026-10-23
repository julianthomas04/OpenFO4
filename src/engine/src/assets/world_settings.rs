//! World-extent settings the game keeps in its INI cascade: the loaded cell
//! block (`uGridsToLoad`).

use crate::assets::ini::{GameIni, IniError, IniKey, capped_at_engine_limit};

const CELL_SIZE_GAME_UNITS: f32 = 4096.0;

const GRIDS_TO_LOAD: IniKey = IniKey::new("General", "uGridsToLoad");

/// The block of loaded exterior cells, `uGridsToLoad` cells on a side.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct LoadedGrid {
    grids_to_load: u32,
}

impl LoadedGrid {
    /// The game needs an odd side of at least 3 so the block centres on a cell. A side above
    /// `max_grids_to_load` (what the streamer keeps resident) is clamped to it, with a warning,
    /// so the collision ring, water cell surfaces and grass cap never reach past the loaded
    /// cells. `max_grids_to_load` must be odd.
    pub fn from_ini(ini: &GameIni, max_grids_to_load: u32) -> Result<Self, IniError> {
        debug_assert!(max_grids_to_load % 2 == 1, "the maximum side must be odd");
        let requested =
            ini.require_valid::<u32>(GRIDS_TO_LOAD, "an odd number, 3 or more", |n| {
                *n >= 3 && n % 2 == 1
            })?;
        Ok(Self {
            grids_to_load: capped_at_engine_limit(GRIDS_TO_LOAD, requested, max_grids_to_load),
        })
    }

    /// Cells between the centre cell and the block's edge.
    pub fn radius(self) -> i32 {
        i32::try_from(self.grids_to_load / 2).expect("grid radius fits an i32")
    }

    pub fn extent_game_units(self) -> f32 {
        self.grids_to_load as f32 * CELL_SIZE_GAME_UNITS
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;

    /// The side a test's streamer keeps resident at most.
    pub const MAX_SIDE: u32 = 17;

    pub fn grid_of_side(side: u32) -> LoadedGrid {
        let ini = GameIni::from_texts([format!("uGridsToLoad={side}\n").as_str()]);
        LoadedGrid::from_ini(&ini, MAX_SIDE).unwrap()
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals parsed or multiplied, not accumulated"
)]
mod tests {
    use super::test_support::{MAX_SIDE, grid_of_side};
    use super::*;

    #[test]
    fn the_grid_radius_is_half_the_side_rounded_down() {
        assert_eq!(grid_of_side(5).radius(), 2);
        assert_eq!(grid_of_side(7).radius(), 3);
    }

    #[test]
    fn the_grid_extent_is_the_side_in_cells() {
        assert_eq!(grid_of_side(5).extent_game_units(), 20_480.0);
    }

    #[test]
    fn an_absent_key_is_an_error_naming_it() {
        let error = LoadedGrid::from_ini(&GameIni::from_texts([""]), MAX_SIDE).unwrap_err();
        assert!(matches!(
            error,
            IniError::MissingKey {
                key: "uGridsToLoad",
                ..
            }
        ));
    }

    #[test]
    fn an_even_or_too_small_grid_side_is_an_error() {
        for text in ["uGridsToLoad=4\n", "uGridsToLoad=1\n", "uGridsToLoad=0\n"] {
            let error = LoadedGrid::from_ini(&GameIni::from_texts([text]), MAX_SIDE).unwrap_err();
            assert!(matches!(error, IniError::Invalid { .. }), "{text}");
        }
    }

    #[test]
    fn a_negative_or_unparsable_grid_side_is_an_error() {
        for text in ["uGridsToLoad=-5\n", "uGridsToLoad=many\n"] {
            let error = LoadedGrid::from_ini(&GameIni::from_texts([text]), MAX_SIDE).unwrap_err();
            assert!(matches!(error, IniError::Unparsable { .. }), "{text}");
        }
    }

    #[test]
    fn the_smallest_and_largest_supported_sides_are_kept() {
        assert_eq!(grid_of_side(3).radius(), 1);
        assert_eq!(grid_of_side(17).radius(), 8);
    }

    #[test]
    fn a_side_beyond_what_streaming_supports_is_clamped() {
        for side in [19, 25, 101] {
            let grid = grid_of_side(side);
            assert_eq!(grid.radius(), 8, "{side}");
            assert_eq!(grid.extent_game_units(), 17.0 * 4096.0, "{side}");
        }
    }
}
