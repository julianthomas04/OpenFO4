//! The water world: which water exists where, as Fallout 4 answers it.
//! One model serves the renderer (what to draw, whether the camera is under
//! water) and the simulation (water height and current at a body).
//!
//! The data has three layers: the scope (an exterior worldspace with its
//! cells' water columns and LOD quads, or one interior cell), the placed
//! water volumes of the resident cells (pushed in every frame by whoever
//! streams the cells) and the WATR currents.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Instant;

use glam::Vec3;

use super::cell_water::{CellSurface, LoadedCell, surfaces_of};
use super::height::{
    CellWaterState, WorldspaceChain, WorldspaceWaterRecord, water_height, water_type,
};
use super::lod::LodWater;
use super::volume::PlacedWaterVolume;
use crate::assets::db::DbClient;
use crate::assets::db::water_placement::CellWater;
use crate::bethesda_space::{
    UNITS_FROM_ENGINE, UNITS_TO_ENGINE, axes_to_engine, position_from_engine,
};
use crate::sim::water_physics::WaterCurrent;

/// Side of an exterior cell, game units.
const CELL_SIZE: f32 = 4096.0;
/// The exterior grid cell of a game-space position.
pub fn exterior_cell_of(game: Vec3) -> (i32, i32) {
    (
        (game.x / CELL_SIZE).floor() as i32,
        (game.y / CELL_SIZE).floor() as i32,
    )
}

/// A worldspace parent chain this long is a cycle in the data.
const MAX_WORLDSPACE_DEPTH: usize = 8;

/// The water world's place: what the streamer has loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope<'a> {
    /// An exterior worldspace, by form key.
    Exterior(&'a str),
    /// A single interior cell, by form key.
    Interior(&'a str),
}

impl Scope<'_> {
    fn owned(self) -> OwnedScope {
        match self {
            Scope::Exterior(key) => OwnedScope::Exterior(key.to_owned()),
            Scope::Interior(key) => OwnedScope::Interior(key.to_owned()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum OwnedScope {
    Exterior(String),
    Interior(String),
}

impl OwnedScope {
    fn is(&self, scope: Scope<'_>) -> bool {
        match (self, scope) {
            (Self::Exterior(a), Scope::Exterior(b)) | (Self::Interior(a), Scope::Interior(b)) => {
                a == b
            }
            _ => false,
        }
    }
}

struct ExteriorWater {
    chain: WorldspaceChain,
    cells: HashMap<(i32, i32), CellWater>,
    lod: LodWater,
}

enum Loaded {
    /// Nothing is loaded, or its data could not be read (logged).
    Empty,
    Exterior(ExteriorWater),
    Interior(CellWater),
}

/// The water at a point: its surface height (engine units) and WATR.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterHit<'a> {
    pub height: f32,
    pub water_type: &'a str,
}

pub struct WaterWorld {
    db_path: String,
    scope: Option<OwnedScope>,
    loaded: Loaded,
    /// Placed water of the resident cells, keyed by the cell that owns the
    /// reference (see [`Self::cell_key`]).
    placed: HashMap<(i32, i32), Vec<PlacedWaterVolume>>,
    /// The surfaces of the loaded exterior cells (see [`Self::set_loaded`]).
    cell_surfaces: Vec<CellSurface>,
    cell_surface_cells: HashSet<(i32, i32)>,
    currents: HashMap<String, WaterCurrent>,
}

impl WaterWorld {
    /// An empty world that reads the database at `db_path` when a scope is
    /// set. The WATR currents are read now; failing that, no water has a
    /// current (logged).
    pub fn new(db_path: &str) -> Self {
        let waters = DbClient::new(db_path).and_then(|db| db.get_waters());
        let currents = match waters {
            Ok(waters) => waters
                .into_iter()
                .filter_map(|water| {
                    let data = water.data?;
                    let current = current_of_watr(data.linear_velocity, data.angular_velocity)?;
                    Some((water.form_key, current))
                })
                .collect(),
            Err(error) => {
                tracing::error!("water world: WATR currents unavailable: {error:#}");
                HashMap::new()
            }
        };
        Self {
            db_path: db_path.to_owned(),
            scope: None,
            loaded: Loaded::Empty,
            placed: HashMap::new(),
            cell_surfaces: Vec::new(),
            cell_surface_cells: HashSet::new(),
            currents,
        }
    }

    /// A world over exterior data held in memory.
    pub fn with_exterior(
        chain: WorldspaceChain,
        cells: HashMap<(i32, i32), CellWater>,
        lod: LodWater,
    ) -> Self {
        Self {
            loaded: Loaded::Exterior(ExteriorWater { chain, cells, lod }),
            ..Self::empty()
        }
    }

    /// A world over one interior cell held in memory.
    pub fn with_interior(cell: CellWater) -> Self {
        Self {
            loaded: Loaded::Interior(cell),
            ..Self::empty()
        }
    }

    #[must_use]
    pub fn with_currents(mut self, currents: HashMap<String, WaterCurrent>) -> Self {
        self.currents = currents;
        self
    }

    fn empty() -> Self {
        Self {
            db_path: String::new(),
            scope: None,
            loaded: Loaded::Empty,
            placed: HashMap::new(),
            cell_surfaces: Vec::new(),
            cell_surface_cells: HashSet::new(),
            currents: HashMap::new(),
        }
    }

    /// Switches to `scope`, reading its data from the database; a no-op while
    /// it is already loaded. A scope whose data cannot be read leaves the
    /// world without water and logs why.
    pub fn set_scope(&mut self, scope: Scope<'_>) {
        if self.scope.as_ref().is_some_and(|loaded| loaded.is(scope)) {
            return;
        }
        let started = Instant::now();
        self.scope = Some(scope.owned());
        self.placed.clear();
        self.set_cell_surfaces(Vec::new());
        let loaded = DbClient::new(&self.db_path).and_then(|db| match scope {
            Scope::Exterior(worldspace) => load_exterior(&db, worldspace).map(Loaded::Exterior),
            Scope::Interior(cell) => db.get_cell_water(cell).map(Loaded::Interior),
        });
        self.loaded = loaded.unwrap_or_else(|error| {
            tracing::error!("water world: {scope:?} has no water data: {error:#}");
            Loaded::Empty
        });
        tracing::info!(
            "water world: loaded {scope:?} ({}) in {:?}",
            self.describe(),
            started.elapsed()
        );
    }

    fn describe(&self) -> String {
        match &self.loaded {
            Loaded::Empty => "empty".to_owned(),
            Loaded::Exterior(exterior) => format!(
                "{} water cells, {} LOD quads, water {}, LOD water {}",
                exterior.cells.len(),
                exterior.lod.len(),
                exterior.chain.water_form(),
                exterior.lod.water_type
            ),
            Loaded::Interior(cell) => format!(
                "interior has_water={}, water type {}",
                cell.has_water,
                cell.water_type.as_deref().unwrap_or("none")
            ),
        }
    }

    /// Replaces the placed water volumes of the resident cells.
    pub fn set_placed(&mut self, volumes: impl IntoIterator<Item = PlacedWaterVolume>) {
        self.placed.clear();
        for volume in volumes {
            let key = self.cell_key(position_from_engine(volume.origin()));
            self.placed.entry(key).or_default().push(volume);
        }
    }

    /// Replaces the loaded exterior cells: each whose water is above its land
    /// gets a surface of its own (`cell_water`). Interiors have none.
    pub fn set_loaded(&mut self, cells: impl IntoIterator<Item = LoadedCell>) {
        let surfaces = match &self.loaded {
            Loaded::Exterior(exterior) => surfaces_of(cells, &exterior.cells, &exterior.chain),
            _ => Vec::new(),
        };
        self.set_cell_surfaces(surfaces);
    }

    fn set_cell_surfaces(&mut self, surfaces: Vec<CellSurface>) {
        self.cell_surface_cells = surfaces.iter().map(|surface| surface.plane.cell).collect();
        self.cell_surfaces = surfaces;
    }

    /// The surfaces of the loaded cells that have water.
    pub fn cell_surfaces(&self) -> &[CellSurface] {
        &self.cell_surfaces
    }

    /// Whether the loaded cell has a surface of its own, which replaces its
    /// LOD quad.
    pub fn has_cell_surface(&self, cell: (i32, i32)) -> bool {
        self.cell_surface_cells.contains(&cell)
    }

    pub fn placed_count(&self) -> usize {
        self.placed.values().map(Vec::len).sum()
    }

    pub fn is_interior(&self) -> bool {
        matches!(self.loaded, Loaded::Interior(_))
    }

    /// Whether the player's interior cell has no water at all, so nothing is drawn.
    pub fn water_is_cleared(&self) -> bool {
        matches!(&self.loaded, Loaded::Interior(cell) if !cell.has_water)
    }

    /// The WATR form key of the exterior worldspace's water,
    /// which the shader's far-field registers read; `None` indoors and while
    /// nothing is loaded.
    pub fn worldspace_water_form(&self) -> Option<&str> {
        match &self.loaded {
            Loaded::Exterior(exterior) => Some(exterior.chain.water_form()),
            _ => None,
        }
    }

    /// The LOD water of the exterior worldspace; interiors draw none.
    pub fn lod_water(&self) -> Option<&LodWater> {
        match &self.loaded {
            Loaded::Exterior(exterior) if exterior.chain.draws_lod_water() => Some(&exterior.lod),
            _ => None,
        }
    }

    /// The cell that owns a game-space position: its grid cell outdoors, the
    /// one loaded cell indoors.
    fn cell_key(&self, game: Vec3) -> (i32, i32) {
        match self.loaded {
            Loaded::Interior(_) => (0, 0),
            _ => exterior_cell_of(game),
        }
    }

    /// The water height and the water type at an engine-space point.
    pub fn surface_at(&self, point: Vec3) -> Option<WaterHit<'_>> {
        let game = position_from_engine(point);
        let key = self.cell_key(game);
        let (state, cell_type, chain, default_height) = match &self.loaded {
            Loaded::Empty => return None,
            Loaded::Interior(cell) => (
                CellWaterState::of_interior(cell),
                cell.water_type.as_deref(),
                None,
                0.0,
            ),
            Loaded::Exterior(exterior) => {
                let cell = exterior.cells.get(&key);
                (
                    cell.map_or_else(
                        CellWaterState::exterior_without_water,
                        CellWaterState::of_exterior,
                    ),
                    cell.and_then(|cell| cell.water_type.as_deref()),
                    Some(&exterior.chain),
                    exterior.chain.default_water_height(),
                )
            }
        };
        let covering: Vec<(f32, &PlacedWaterVolume)> = self
            .placed
            .get(&key)
            .into_iter()
            .flatten()
            .filter(|volume| volume.contains(point))
            .filter_map(|volume| {
                let height = volume.plane.height_at(point.x, point.z)?;
                Some((height * UNITS_FROM_ENGINE, volume))
            })
            .collect();
        let height = water_height(state, default_height, covering.iter().map(|(h, _)| *h))?;
        let cell_wins = state
            .effective_height(default_height)
            .is_some_and(|cell_height| cell_height >= height);
        let placed_type = covering
            .iter()
            .filter(|(h, _)| *h >= height)
            .find_map(|(_, volume)| volume.water_type.as_deref());
        let resolved_type = if cell_wins {
            water_type(cell_type, chain)
        } else {
            // A placed activator without WNAM takes the cell's type.
            placed_type.unwrap_or_else(|| water_type(cell_type, chain))
        };
        Some(WaterHit {
            height: height * UNITS_TO_ENGINE,
            water_type: resolved_type,
        })
    }

    /// The global current of a WATR (`NAM0`/`NAM1`), if it has one.
    pub fn current_of(&self, water_type: &str) -> Option<WaterCurrent> {
        self.currents.get(water_type).copied()
    }
}

/// WATR `NAM0`/`NAM1` as a global current in engine axes; `None` when both
/// are zero (the game then adds no current element).
pub fn current_of_watr(linear: [f32; 3], angular: [f32; 3]) -> Option<WaterCurrent> {
    let to_engine = |v: [f32; 3]| axes_to_engine(v[0], v[1], v[2]);
    let (linear, angular) = (to_engine(linear), to_engine(angular));
    (linear != Vec3::ZERO || angular != Vec3::ZERO)
        .then(|| WaterCurrent::from_havok(linear, angular))
}

/// Debug probe: lifts the LOD water quads by this many game units.
const LOD_RAISE_ENV: &str = "OPENFO4_WATER_LOD_RAISE";

fn load_exterior(db: &DbClient, worldspace: &str) -> anyhow::Result<ExteriorWater> {
    let chain = load_chain(db, worldspace)?;
    let cells = db.get_exterior_cell_waters(worldspace)?;
    let quads = match db.get_lod_water_quads(worldspace) {
        Ok(quads) => quads,
        Err(error) => {
            tracing::warn!("water world: no LOD water for {worldspace}: {error:#}");
            Vec::new()
        }
    };
    let mut lod = LodWater::new(&quads, chain.lod_water_form(), chain.lod_water_height());
    if let Some(raise) = crate::utils::env_opt::<f32>(LOD_RAISE_ENV) {
        tracing::warn!(
            "{LOD_RAISE_ENV}={raise}: LOD water lifted by {raise} game units (debug probe)"
        );
        lod = lod.raised(raise);
    }
    Ok(ExteriorWater { chain, cells, lod })
}

/// The worldspace's default water height in game units, through its parent chain.
pub fn worldspace_default_water_height(db: &DbClient, worldspace: &str) -> anyhow::Result<f32> {
    Ok(load_chain(db, worldspace)?.default_water_height())
}

fn load_chain(db: &DbClient, worldspace: &str) -> anyhow::Result<WorldspaceChain> {
    let mut records = Vec::new();
    let mut seen = HashSet::new();
    let mut next = Some(worldspace.to_owned());
    while let Some(key) = next.take() {
        anyhow::ensure!(
            seen.insert(key.clone()) && seen.len() <= MAX_WORLDSPACE_DEPTH,
            "worldspace parent chain of {worldspace} loops at {key}"
        );
        match db.get_worldspace_water(&key) {
            Ok(water) => {
                let record = WorldspaceWaterRecord::from_db(&key, water);
                next.clone_from(&record.parent);
                records.push(record);
            }
            // The worldspace itself must exist; a missing parent only ends
            // the chain.
            Err(error) if !records.is_empty() => {
                tracing::warn!("water world: parent worldspace {key} of {worldspace}: {error:#}");
            }
            Err(error) => return Err(error),
        }
    }
    Ok(WorldspaceChain::new(records))
}

/// The water world shared by the renderer, which keeps it current, and the
/// simulation, which queries it.
#[derive(Clone)]
pub struct SharedWaterWorld(Arc<RwLock<WaterWorld>>);

impl SharedWaterWorld {
    pub fn new(world: WaterWorld) -> Self {
        Self(Arc::new(RwLock::new(world)))
    }

    pub fn read(&self) -> RwLockReadGuard<'_, WaterWorld> {
        self.0.read().expect("water world lock poisoned")
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, WaterWorld> {
        self.0.write().expect("water world lock poisoned")
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact in f32 or compared with an epsilon"
)]
mod tests {
    use glam::{Mat4, Quat};

    use super::*;
    use crate::assets::db::water_placement::CellWaterHeight;
    use crate::sim::collision::{ColShape, PartShape};
    use crate::water_world::height::{DEFAULT_WATER_FORM_KEY, WORLDSPACE_NO_LOD_WATER};
    use crate::water_world::volume::VolumeShapes;

    fn cell(has_water: bool, height: CellWaterHeight, water_type: Option<&str>) -> CellWater {
        CellWater {
            has_water,
            no_lod_water: false,
            height,
            water_type: water_type.map(str::to_owned),
            environment_map: None,
            currents: None,
        }
    }

    fn chain_record(default_height: f32) -> WorldspaceWaterRecord {
        WorldspaceWaterRecord {
            form_key: "ws".to_owned(),
            parent: None,
            parent_flags: 0,
            water_form: Some("ocean".to_owned()),
            lod_water_form: Some("lod".to_owned()),
            lod_water_height: Some(default_height),
            default_water_height: default_height,
            flags: 0,
        }
    }

    fn chain(default_height: f32) -> WorldspaceChain {
        WorldspaceChain::new(vec![chain_record(default_height)])
    }

    fn ocean_world() -> WaterWorld {
        let cells = HashMap::from([
            ((0, 0), cell(true, CellWaterHeight::Unset, None)),
            (
                (1, 0),
                cell(true, CellWaterHeight::Level(1300.0), Some("lake")),
            ),
        ]);
        WaterWorld::with_exterior(chain(450.0), cells, LodWater::none())
    }

    /// A point in game units, as engine space.
    fn at(x: f32, y: f32, z: f32) -> Vec3 {
        crate::bethesda_space::position_to_engine(x, y, z)
    }

    fn slab() -> Arc<VolumeShapes> {
        let shape = PartShape {
            shape: ColShape::Box {
                center: Vec3::new(0.0, -51.2, 0.0),
                rotation: Quat::IDENTITY,
                half_extents: Vec3::new(51.2, 51.2, 51.2),
            },
            node: None,
            joint: None,
            material: 0,
        };
        Arc::new(VolumeShapes::from_sidecar(&[shape], "slab").unwrap())
    }

    fn pond(game_x: f32, game_y: f32, game_z: f32, water_type: Option<&str>) -> PlacedWaterVolume {
        PlacedWaterVolume::new(
            Mat4::from_translation(at(game_x, game_y, game_z)),
            water_type.map(Arc::from),
            Some(slab()),
        )
    }

    #[test]
    fn an_exterior_cell_without_its_own_height_has_the_worldspace_default() {
        let world = ocean_world();
        let hit = world.surface_at(at(100.0, 100.0, 0.0)).unwrap();
        assert!((hit.height - 45.0).abs() < 1e-3);
        assert_eq!(hit.water_type, "ocean");
    }

    #[test]
    fn a_cell_with_xclw_and_xcwt_uses_both() {
        let world = ocean_world();
        let hit = world.surface_at(at(5000.0, 100.0, 0.0)).unwrap();
        assert!((hit.height - 130.0).abs() < 1e-3);
        assert_eq!(hit.water_type, "lake");
    }

    #[test]
    fn cells_without_water_and_missing_cells_answer_nothing() {
        let world = ocean_world();
        assert_eq!(world.surface_at(at(-5000.0, 100.0, 0.0)), None);
        let dry = WaterWorld::with_exterior(
            chain(450.0),
            HashMap::from([((0, 0), cell(false, CellWaterHeight::Level(10.0), None))]),
            LodWater::none(),
        );
        assert_eq!(dry.surface_at(at(100.0, 100.0, 0.0)), None);
    }

    #[test]
    fn a_placed_volume_above_the_cell_plane_wins_inside_its_volume_only() {
        let mut world = ocean_world();
        world.set_placed([pond(100.0, 100.0, 460.0, Some("pond"))]);
        let inside = world.surface_at(at(100.0, 100.0, 455.0)).unwrap();
        assert!((inside.height - 46.0).abs() < 1e-3);
        assert_eq!(inside.water_type, "pond");
        let outside = world.surface_at(at(900.0, 100.0, 455.0)).unwrap();
        assert!((outside.height - 45.0).abs() < 1e-3);
        assert_eq!(outside.water_type, "ocean");
    }

    #[test]
    fn a_placed_volume_below_the_cell_plane_does_not_lower_it() {
        let mut world = ocean_world();
        world.set_placed([pond(100.0, 100.0, 300.0, Some("pond"))]);
        let hit = world.surface_at(at(100.0, 100.0, 250.0)).unwrap();
        assert!((hit.height - 45.0).abs() < 1e-3);
        assert_eq!(hit.water_type, "ocean");
    }

    #[test]
    fn a_placed_volume_counts_only_for_the_cell_owning_its_reference() {
        let mut world = ocean_world();
        // Origin in cell (0,0), slab reaching into cell (1,0) by 100 units.
        world.set_placed([pond(4050.0, 100.0, 460.0, None)]);
        assert!(world.surface_at(at(4050.0, 100.0, 455.0)).unwrap().height > 45.5);
        let next_cell = world.surface_at(at(4130.0, 100.0, 455.0)).unwrap();
        assert!((next_cell.height - 130.0).abs() < 1e-3, "{next_cell:?}");
    }

    #[test]
    fn a_placed_water_without_a_water_type_takes_the_cell_type() {
        let mut world = ocean_world();
        world.set_placed([pond(100.0, 100.0, 460.0, None)]);
        let hit = world.surface_at(at(100.0, 100.0, 455.0)).unwrap();
        assert_eq!(hit.water_type, "ocean");
    }

    #[test]
    fn an_interior_with_has_water_is_only_its_placed_volumes() {
        let mut world = WaterWorld::with_interior(cell(true, CellWaterHeight::Level(0.0), None));
        assert_eq!(world.surface_at(at(100.0, 100.0, -5.0)), None);
        world.set_placed([pond(100.0, 100.0, 0.0, Some("pool"))]);
        let hit = world.surface_at(at(100.0, 100.0, -5.0)).unwrap();
        assert!(hit.height.abs() < 1e-4);
        assert_eq!(hit.water_type, "pool");
    }

    #[test]
    fn an_interior_without_water_is_cleared_and_answers_nothing() {
        let mut world = WaterWorld::with_interior(cell(false, CellWaterHeight::Absent, None));
        world.set_placed([pond(100.0, 100.0, 0.0, Some("pool"))]);
        assert!(world.water_is_cleared());
        assert_eq!(world.surface_at(at(100.0, 100.0, -5.0)), None);
    }

    #[test]
    fn an_interior_without_a_cell_water_type_falls_back_to_the_default_water() {
        let mut world = WaterWorld::with_interior(cell(true, CellWaterHeight::Absent, None));
        world.set_placed([pond(0.0, 0.0, 0.0, None)]);
        let hit = world.surface_at(at(0.0, 0.0, -5.0)).unwrap();
        assert_eq!(hit.water_type, DEFAULT_WATER_FORM_KEY);
    }

    #[test]
    fn an_empty_world_has_no_water() {
        assert_eq!(WaterWorld::empty().surface_at(Vec3::ZERO), None);
    }

    #[test]
    fn the_lod_water_is_exterior_only_and_honours_the_worldspace_flag() {
        assert!(ocean_world().lod_water().is_some());
        assert!(
            WaterWorld::with_interior(cell(true, CellWaterHeight::Absent, None))
                .lod_water()
                .is_none()
        );
        let no_lod = WorldspaceChain::new(vec![WorldspaceWaterRecord {
            flags: WORLDSPACE_NO_LOD_WATER,
            ..chain_record(450.0)
        }]);
        let world = WaterWorld::with_exterior(no_lod, HashMap::new(), LodWater::none());
        assert!(world.lod_water().is_none());
    }

    #[test]
    fn a_zero_current_is_no_current() {
        assert_eq!(current_of_watr([0.0; 3], [0.0; 3]), None);
    }

    #[test]
    fn a_current_is_converted_from_havok_game_axes_to_engine_axes() {
        let current = current_of_watr([1.0, 2.0, 3.0], [0.0, 0.0, 0.5]).unwrap();
        // Game (x, y, z) maps to engine (x, z, -y); linear m/s become units/s.
        assert!(
            current
                .linear
                .abs_diff_eq(Vec3::new(7.0, 21.0, -14.0), 1e-4)
        );
        assert!(current.angular.abs_diff_eq(Vec3::new(0.0, 0.5, 0.0), 1e-6));
    }

    #[test]
    fn the_current_of_a_water_comes_from_the_table() {
        let world = ocean_world().with_currents(HashMap::from([(
            "ocean".to_owned(),
            current_of_watr([0.5, 0.0, 0.0], [0.0; 3]).unwrap(),
        )]));
        assert!(world.current_of("ocean").is_some());
        assert!(world.current_of("lake").is_none());
    }

    fn loaded(x: i32, y: i32, land_lowest_game_units: f32) -> LoadedCell {
        LoadedCell {
            x,
            y,
            land_lowest_y: land_lowest_game_units * UNITS_TO_ENGINE,
        }
    }

    #[test]
    fn loaded_cells_with_water_above_their_land_get_a_surface_that_replaces_their_lod_quad() {
        let mut world = ocean_world();
        world.set_loaded([loaded(0, 0, 100.0), loaded(1, 0, 1400.0), loaded(2, 0, 0.0)]);
        let cells: Vec<_> = world.cell_surfaces().iter().map(|s| s.plane.cell).collect();
        assert_eq!(cells, [(0, 0)]);
        assert!(world.has_cell_surface((0, 0)));
        assert!(!world.has_cell_surface((1, 0)), "land above the water");
        assert!(!world.has_cell_surface((2, 0)), "no water record");
        assert!(!world.has_cell_surface((9, 9)), "not loaded");
    }

    #[test]
    fn unloading_a_cell_returns_its_lod_quad() {
        let mut world = ocean_world();
        world.set_loaded([loaded(0, 0, 100.0)]);
        world.set_loaded([]);
        assert!(!world.has_cell_surface((0, 0)));
        assert!(world.cell_surfaces().is_empty());
    }

    #[test]
    fn interiors_and_empty_worlds_have_no_cell_surfaces() {
        let mut interior = WaterWorld::with_interior(cell(true, CellWaterHeight::Unset, None));
        interior.set_loaded([loaded(0, 0, 0.0)]);
        assert!(interior.cell_surfaces().is_empty());
        let mut empty = WaterWorld::new("");
        empty.set_loaded([loaded(0, 0, 0.0)]);
        assert!(empty.cell_surfaces().is_empty());
    }

    /// Against a converted database: `OPENFO4_DB=<data.db> cargo test
    /// real_database -- --ignored`.
    #[test]
    #[ignore = "needs a converted data.db (OPENFO4_DB)"]
    fn real_database_answers_the_commonwealth_ocean_and_an_interior() {
        let db_path = std::env::var("OPENFO4_DB").expect("OPENFO4_DB names a data.db");
        let mut world = WaterWorld::new(&db_path);
        world.set_scope(Scope::Exterior("00003C:Fallout4.esm"));
        let ocean = world.surface_at(at(2048.0, -2048.0, 300.0)).unwrap();
        assert!((ocean.height - 45.0).abs() < 1e-3, "{ocean:?}");
        assert_eq!(ocean.water_type, "000018:Fallout4.esm");
        assert!(world.lod_water().unwrap().len() > 20_000);
        assert!(world.current_of("000018:Fallout4.esm").is_some());
        world.set_scope(Scope::Interior("15054F:Fallout4.esm"));
        assert!(!world.water_is_cleared() && world.lod_water().is_none());
        world.set_scope(Scope::Interior("0317D0:Fallout4.esm"));
        assert!(world.water_is_cleared());
    }
}
