//! Cell and worldspace water rules of Fallout 4, in game units: the cell's effective water height, the water
//! height at a point, the water type and the worldspace parent-inheritance
//! getters.
//!
//! Everything here is pure; the data comes from the database through
//! [`super::WaterWorld`].

use crate::assets::db::water_placement::{CellWater, CellWaterHeight, WorldspaceWater};
use crate::assets::db::worldspace_lineage::{WorldspaceLink, owner_index, parent_use};

/// The hard-coded fallback water form (form ID 0x18, EDID `DefaultWater`) the
/// game uses when a cell, worldspace or LOD water type is null.
pub const DEFAULT_WATER_FORM_KEY: &str = "000018:Fallout4.esm";

/// The cell loader rounds `XCLW` to an integer; a value at or above this
/// (float max) means "use the worldspace default".
const USE_WORLDSPACE_DEFAULT_HEIGHT: f32 = 2_147_483_648.0;

/// `DATA` worldspace flag: no LOD water.
pub const WORLDSPACE_NO_LOD_WATER: u32 = 0x08;

/// A worldspace's own water record.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldspaceWaterRecord {
    pub form_key: String,
    pub parent: Option<String>,
    pub parent_flags: u16,
    /// `NAM2`.
    pub water_form: Option<String>,
    /// `NAM3`.
    pub lod_water_form: Option<String>,
    /// `NAM4`.
    pub lod_water_height: Option<f32>,
    /// `DNAM` default water height; the worldspace constructor sets 0.
    pub default_water_height: f32,
    pub flags: u32,
}

impl WorldspaceWaterRecord {
    pub fn from_db(form_key: &str, water: WorldspaceWater) -> Self {
        Self {
            form_key: form_key.to_owned(),
            parent: water.parent_worldspace,
            parent_flags: water.parent_flags.unwrap_or(0),
            water_form: water.water_type,
            lod_water_form: water.lod_water_type,
            lod_water_height: water.lod_water_height,
            default_water_height: water.default_water_height.unwrap_or(0.0),
            flags: water.flags.unwrap_or(0),
        }
    }
}

impl WorldspaceLink for WorldspaceWaterRecord {
    fn parent(&self) -> Option<&str> {
        self.parent.as_deref()
    }

    fn parent_flags(&self) -> u16 {
        self.parent_flags
    }
}

/// A worldspace and the parents its `PNAM` flags can defer to, nearest first.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldspaceChain {
    records: Vec<WorldspaceWaterRecord>,
}

impl WorldspaceChain {
    /// `records[0]` is the worldspace itself, each next one the parent of the
    /// previous. Panics on an empty chain: a worldspace always exists.
    pub fn new(records: Vec<WorldspaceWaterRecord>) -> Self {
        assert!(!records.is_empty(), "a water chain starts at a worldspace");
        Self { records }
    }

    /// The first record in the chain that keeps its own value for `flag`.
    fn owner(&self, flag: u16) -> &WorldspaceWaterRecord {
        &self.records[owner_index(&self.records, flag)]
    }

    pub fn worldspace(&self) -> &str {
        &self.records[0].form_key
    }

    /// The worldspace's water form, with parent inheritance.
    pub fn water_form(&self) -> &str {
        self.owner(parent_use::WATER_DATA)
            .water_form
            .as_deref()
            .unwrap_or(DEFAULT_WATER_FORM_KEY)
    }

    /// The worldspace's LOD water form, with parent inheritance.
    pub fn lod_water_form(&self) -> &str {
        self.owner(parent_use::LOD_DATA)
            .lod_water_form
            .as_deref()
            .unwrap_or(DEFAULT_WATER_FORM_KEY)
    }

    /// The worldspace's default water height, with parent inheritance.
    pub fn default_water_height(&self) -> f32 {
        self.owner(parent_use::LAND_DATA).default_water_height
    }

    /// `NAM4` of the worldspace itself: no parent chain is followed for it.
    pub fn lod_water_height(&self) -> Option<f32> {
        self.records[0].lod_water_height
    }

    /// Whether the worldspace draws LOD water at all.
    pub fn draws_lod_water(&self) -> bool {
        self.records[0].flags & WORLDSPACE_NO_LOD_WATER == 0
    }
}

/// The water facts of a cell the height rules read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellWaterState {
    pub interior: bool,
    pub has_water: bool,
    pub xclw: CellWaterHeight,
}

impl CellWaterState {
    pub fn of_exterior(cell: &CellWater) -> Self {
        Self {
            interior: false,
            has_water: cell.has_water,
            xclw: cell.height,
        }
    }

    pub fn of_interior(cell: &CellWater) -> Self {
        Self {
            interior: true,
            ..Self::of_exterior(cell)
        }
    }

    /// An exterior cell without a water record in the database.
    pub fn exterior_without_water() -> Self {
        Self {
            interior: false,
            has_water: false,
            xclw: CellWaterHeight::Absent,
        }
    }

    /// The water height query answers nothing at all for an interior without the
    /// "has water" flag, even where water is placed.
    pub fn answers_water_queries(&self) -> bool {
        !self.interior || self.has_water
    }

    /// The cell's effective water height: its own plane. `None` is the game's
    /// `-FLT_MAX`: interiors (their `XCLW` is never read) and cells without
    /// "has water".
    pub fn effective_height(&self, worldspace_default: f32) -> Option<f32> {
        if self.interior || !self.has_water {
            return None;
        }
        match self.xclw {
            CellWaterHeight::Level(height) if height < USE_WORLDSPACE_DEFAULT_HEIGHT => {
                Some((height + 0.5).floor())
            }
            // A missing XCLW is read as "unset".
            CellWaterHeight::Level(_) | CellWaterHeight::Unset | CellWaterHeight::Absent => {
                Some(worldspace_default)
            }
        }
    }
}

/// The water height at a point: the highest of the cell plane and the placed-water
/// volumes that contain the point (`placed` holds those volumes' plane
/// heights at the point).
pub fn water_height(
    cell: CellWaterState,
    worldspace_default: f32,
    placed: impl IntoIterator<Item = f32>,
) -> Option<f32> {
    if !cell.answers_water_queries() {
        return None;
    }
    placed
        .into_iter()
        .chain(cell.effective_height(worldspace_default))
        .max_by(f32::total_cmp)
}

/// The water type: the cell's `XCWT`, else (exteriors) the worldspace water,
/// else the global default.
pub fn water_type<'a>(
    cell_type: Option<&'a str>,
    worldspace: Option<&'a WorldspaceChain>,
) -> &'a str {
    cell_type
        .or_else(|| worldspace.map(WorldspaceChain::water_form))
        .unwrap_or(DEFAULT_WATER_FORM_KEY)
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact in f32 or compared with an epsilon"
)]
mod tests {
    use super::*;

    fn record(name: &str, parent: Option<&str>, flags: u16) -> WorldspaceWaterRecord {
        WorldspaceWaterRecord {
            form_key: name.to_owned(),
            parent: parent.map(str::to_owned),
            parent_flags: flags,
            water_form: Some(format!("{name}-water")),
            lod_water_form: Some(format!("{name}-lod")),
            lod_water_height: Some(100.0),
            default_water_height: 450.0,
            flags: 0,
        }
    }

    fn exterior(xclw: CellWaterHeight) -> CellWaterState {
        CellWaterState {
            interior: false,
            has_water: true,
            xclw,
        }
    }

    #[test]
    fn xclw_below_the_sentinel_is_the_cell_height_rounded_half_up() {
        let state = exterior(CellWaterHeight::Level(1299.5));
        assert_eq!(state.effective_height(450.0), Some(1300.0));
        let state = exterior(CellWaterHeight::Level(-0.6));
        assert_eq!(state.effective_height(450.0), Some(-1.0));
    }

    #[test]
    fn a_cell_without_its_own_height_takes_the_worldspace_default() {
        for xclw in [
            CellWaterHeight::Unset,
            CellWaterHeight::Absent,
            CellWaterHeight::Level(2_147_483_648.0),
        ] {
            assert_eq!(exterior(xclw).effective_height(450.0), Some(450.0));
        }
    }

    #[test]
    fn an_explicit_zero_height_is_a_height() {
        let state = exterior(CellWaterHeight::Level(0.0));
        assert_eq!(state.effective_height(450.0), Some(0.0));
    }

    #[test]
    fn exteriors_without_has_water_have_no_cell_plane() {
        let state = CellWaterState {
            has_water: false,
            ..exterior(CellWaterHeight::Level(300.0))
        };
        assert_eq!(state.effective_height(450.0), None);
        assert!(state.answers_water_queries());
    }

    #[test]
    fn interiors_never_read_xclw() {
        let state = CellWaterState {
            interior: true,
            has_water: true,
            xclw: CellWaterHeight::Level(300.0),
        };
        assert_eq!(state.effective_height(450.0), None);
    }

    #[test]
    fn an_interior_without_has_water_answers_nothing_even_with_placed_water() {
        let state = CellWaterState {
            interior: true,
            has_water: false,
            xclw: CellWaterHeight::Absent,
        };
        assert_eq!(water_height(state, 0.0, [10.0]), None);
    }

    #[test]
    fn an_interior_with_has_water_is_the_highest_placed_volume() {
        let state = CellWaterState {
            interior: true,
            has_water: true,
            xclw: CellWaterHeight::Level(0.0),
        };
        assert_eq!(water_height(state, 0.0, [-40.0, 25.0, 10.0]), Some(25.0));
        assert_eq!(water_height(state, 0.0, []), None);
    }

    #[test]
    fn the_height_is_the_maximum_of_cell_plane_and_placed_volumes() {
        let state = exterior(CellWaterHeight::Unset);
        assert_eq!(water_height(state, 450.0, [460.0]), Some(460.0));
        assert_eq!(water_height(state, 450.0, [300.0]), Some(450.0));
        assert_eq!(water_height(state, 450.0, []), Some(450.0));
    }

    #[test]
    fn exteriors_without_has_water_still_see_placed_volumes() {
        let state = CellWaterState::exterior_without_water();
        assert_eq!(water_height(state, 450.0, [20.0]), Some(20.0));
        assert_eq!(water_height(state, 450.0, []), None);
    }

    #[test]
    fn a_worldspace_without_parent_flags_uses_its_own_values() {
        let chain = WorldspaceChain::new(vec![
            record("child", Some("parent"), 0),
            record("parent", None, 0),
        ]);
        assert_eq!(chain.water_form(), "child-water");
        assert_eq!(chain.lod_water_form(), "child-lod");
    }

    #[test]
    fn each_parent_flag_inherits_only_its_own_value() {
        let chain = WorldspaceChain::new(vec![
            WorldspaceWaterRecord {
                default_water_height: -10_000.0,
                ..record("child", Some("parent"), parent_use::WATER_DATA)
            },
            record("parent", None, 0),
        ]);
        assert_eq!(chain.water_form(), "parent-water");
        assert_eq!(chain.lod_water_form(), "child-lod");
        assert_eq!(chain.default_water_height(), -10_000.0);
    }

    #[test]
    fn inheritance_follows_the_chain_while_flags_stay_set() {
        let chain = WorldspaceChain::new(vec![
            record("a", Some("b"), parent_use::LAND_DATA),
            record("b", Some("c"), parent_use::LAND_DATA),
            WorldspaceWaterRecord {
                default_water_height: 7.0,
                ..record("c", None, 0)
            },
        ]);
        assert_eq!(chain.default_water_height(), 7.0);
    }

    #[test]
    fn a_set_flag_without_a_loaded_parent_falls_back_to_the_own_record() {
        let chain = WorldspaceChain::new(vec![record("child", Some("missing"), 0xFF)]);
        assert_eq!(chain.water_form(), "child-water");
    }

    #[test]
    fn a_missing_form_falls_back_to_the_default_water() {
        let chain = WorldspaceChain::new(vec![WorldspaceWaterRecord {
            water_form: None,
            lod_water_form: None,
            ..record("ws", None, 0)
        }]);
        assert_eq!(chain.water_form(), DEFAULT_WATER_FORM_KEY);
        assert_eq!(chain.lod_water_form(), DEFAULT_WATER_FORM_KEY);
    }

    #[test]
    fn lod_height_and_lod_flag_come_from_the_worldspace_itself() {
        let chain = WorldspaceChain::new(vec![
            WorldspaceWaterRecord {
                flags: WORLDSPACE_NO_LOD_WATER,
                lod_water_height: None,
                ..record("child", Some("parent"), 0xFF)
            },
            record("parent", None, 0),
        ]);
        assert_eq!(chain.lod_water_height(), None);
        assert!(!chain.draws_lod_water());
    }

    #[test]
    fn the_water_type_is_cell_then_worldspace_then_default() {
        let chain = WorldspaceChain::new(vec![record("ws", None, 0)]);
        assert_eq!(water_type(Some("cell"), Some(&chain)), "cell");
        assert_eq!(water_type(None, Some(&chain)), "ws-water");
        assert_eq!(water_type(None, None), DEFAULT_WATER_FORM_KEY);
    }
}
