//! Resolves what a foot lands on to a material hash. Mesh colliders already carry theirs; terrain
//! has none, so the material comes from the landscape texture that dominates under the foot.

use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::Context;
use glam::Vec3;

use crate::assets::db::DbClient;
use crate::assets::types::terrain::{
    CELL_SIZE, LandscapeLayer, TERRAIN_QUAD_SIZE, TERRAIN_VERTS, decode_layers, dominant_layer_at,
};
use crate::bethesda_space;
use crate::footsteps::SurfaceMaterial;

/// Cells whose decoded layers stay resident: the cell under the player and its neighbours at an edge.
const CELL_CACHE_CAPACITY: usize = 4;

/// Where the layers of a landscape come from.
pub trait LandscapeSource {
    /// The decoded layers of the landscape of an exterior cell, `None` when the cell has no
    /// landscape or the landscape has no layer data.
    fn layers(
        &self,
        worldspace: &str,
        grid_x: i32,
        grid_y: i32,
    ) -> anyhow::Result<Option<Vec<LandscapeLayer>>>;
}

impl LandscapeSource for DbClient {
    fn layers(
        &self,
        worldspace: &str,
        grid_x: i32,
        grid_y: i32,
    ) -> anyhow::Result<Option<Vec<LandscapeLayer>>> {
        let landscape = self.get_landscape_at(worldspace, grid_x, grid_y)?;
        Ok(landscape.and_then(|landscape| landscape.layers.as_deref().and_then(decode_layers)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CellKey {
    worldspace: String,
    grid_x: i32,
    grid_y: i32,
}

type CachedLayers = Option<Vec<LandscapeLayer>>;

/// Terrain material lookup with the decoded layers of the last few cells.
///
/// INFERRED: the foot takes the material of the landscape texture with the highest blend weight at
/// the nearest terrain vertex, the base texture weighing what the overlays leave.
pub struct FootSurface<S> {
    source: S,
    /// Landscape texture form key to its material hash.
    landscape_materials: HashMap<String, Option<u32>>,
    cells: VecDeque<(CellKey, CachedLayers)>,
    reported: HashSet<(CellKey, String)>,
}

impl<S: LandscapeSource> FootSurface<S> {
    pub fn new(source: S, landscape_materials: HashMap<String, Option<u32>>) -> Self {
        Self {
            source,
            landscape_materials,
            cells: VecDeque::with_capacity(CELL_CACHE_CAPACITY),
            reported: HashSet::new(),
        }
    }

    /// The material hash under a footstep surface; `Ok(None)` when it has no material. Terrain needs
    /// the exterior worldspace the player is in (`None` inside an interior, which has no terrain).
    /// A landscape that cannot be read is an error and is never cached.
    pub fn material_hash(
        &mut self,
        worldspace: Option<&str>,
        surface: SurfaceMaterial,
    ) -> anyhow::Result<Option<u32>> {
        match surface {
            SurfaceMaterial::None => Ok(None),
            SurfaceMaterial::Hash(hash) => Ok(Some(hash)),
            SurfaceMaterial::Terrain { point } => match worldspace {
                Some(worldspace) => self.terrain_material(worldspace, Vec3::from(point)),
                None => Ok(None),
            },
        }
    }

    fn terrain_material(&mut self, worldspace: &str, point: Vec3) -> anyhow::Result<Option<u32>> {
        let (key, vertex_x, vertex_y) = vertex_under(worldspace, point);
        self.ensure_cached(&key)?;
        let layers = self.cells.front().and_then(|(_, layers)| layers.as_deref());
        let Some(layers) = layers else {
            return Ok(None);
        };
        let Some(layer) = dominant_layer_at(layers, vertex_x, vertex_y) else {
            self.report(&key, "its landscape has no textured layer at the foot");
            return Ok(None);
        };
        let Some(texture) = layer.texture_key.clone() else {
            return Ok(None);
        };
        if let Some(Some(hash)) = self.landscape_materials.get(&texture) {
            return Ok(Some(*hash));
        }
        self.report(
            &key,
            &format!("landscape texture {texture} names no material"),
        );
        Ok(None)
    }

    /// Moves the cell to the front of the cache, reading it first when it is not resident.
    fn ensure_cached(&mut self, key: &CellKey) -> anyhow::Result<()> {
        if let Some(position) = self.cells.iter().position(|(cached, _)| cached == key) {
            if let Some(entry) = self.cells.remove(position) {
                self.cells.push_front(entry);
            }
            return Ok(());
        }
        let layers = self
            .source
            .layers(&key.worldspace, key.grid_x, key.grid_y)
            .with_context(|| {
                format!(
                    "cannot read the landscape of cell ({}, {}) of {}",
                    key.grid_x, key.grid_y, key.worldspace
                )
            })?;
        if layers.is_none() {
            self.report(key, "its landscape has no layers");
        }
        self.cells.push_front((key.clone(), layers));
        self.cells.truncate(CELL_CACHE_CAPACITY);
        Ok(())
    }

    fn report(&mut self, key: &CellKey, reason: &str) {
        if self.reported.insert((key.clone(), reason.to_owned())) {
            tracing::warn!(
                "footsteps on terrain cell ({}, {}) of {} are silent: {reason}",
                key.grid_x,
                key.grid_y,
                key.worldspace
            );
        }
    }

    /// How many distinct (cell, reason) failures were logged so far.
    #[cfg(test)]
    fn reported_count(&self) -> usize {
        self.reported.len()
    }
}

/// The cell under an engine-space point and the nearest vertex of its 33 x 33 grid.
fn vertex_under(worldspace: &str, point: Vec3) -> (CellKey, usize, usize) {
    let game = bethesda_space::position_from_engine(point);
    let cell = |coordinate: f32| (coordinate / CELL_SIZE).floor() as i32;
    let (grid_x, grid_y) = (cell(game.x), cell(game.y));
    let vertex = |coordinate: f32, grid: i32| {
        let local = coordinate - grid as f32 * CELL_SIZE;
        ((local / TERRAIN_QUAD_SIZE).round() as usize).min(TERRAIN_VERTS - 1)
    };
    (
        CellKey {
            worldspace: worldspace.to_owned(),
            grid_x,
            grid_y,
        },
        vertex(game.x, grid_x),
        vertex(game.y, grid_y),
    )
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::assets::types::terrain::ALPHA_GRID;

    const WORLD: &str = "Commonwealth";
    const DIRT: &str = "dirt";
    const ROCK: &str = "rock";
    const NO_MATERIAL: &str = "mystery";
    const DIRT_HASH: u32 = 0xD1;
    const ROCK_HASH: u32 = 0xB0;

    fn layer(key: &str, quadrant: u8, is_base: bool, opacity: f32) -> LandscapeLayer {
        LandscapeLayer {
            texture_key: Some(key.to_owned()),
            quadrant,
            is_base,
            alpha_grid: (!is_base).then_some([[opacity; ALPHA_GRID]; ALPHA_GRID]),
            coverage: 0.0,
        }
    }

    struct Landscapes {
        cells: HashMap<(i32, i32), Option<Vec<LandscapeLayer>>>,
        reads: Cell<usize>,
    }

    impl Landscapes {
        fn with(
            cells: impl IntoIterator<Item = ((i32, i32), Option<Vec<LandscapeLayer>>)>,
        ) -> Self {
            Self {
                cells: cells.into_iter().collect(),
                reads: Cell::new(0),
            }
        }
    }

    impl LandscapeSource for &Landscapes {
        fn layers(
            &self,
            worldspace: &str,
            grid_x: i32,
            grid_y: i32,
        ) -> anyhow::Result<Option<Vec<LandscapeLayer>>> {
            assert_eq!(worldspace, WORLD);
            self.reads.set(self.reads.get() + 1);
            self.cells
                .get(&(grid_x, grid_y))
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no such cell"))
        }
    }

    fn materials() -> HashMap<String, Option<u32>> {
        HashMap::from([
            (DIRT.to_owned(), Some(DIRT_HASH)),
            (ROCK.to_owned(), Some(ROCK_HASH)),
            (NO_MATERIAL.to_owned(), None),
        ])
    }

    /// An engine-space point `east` / `north` game units from the south-west corner of cell (0, 0).
    fn point(east: f32, north: f32) -> SurfaceMaterial {
        let engine = bethesda_space::position_to_engine(east, north, 0.0);
        SurfaceMaterial::Terrain {
            point: engine.to_array(),
        }
    }

    fn resolve(landscapes: &Landscapes, surface: SurfaceMaterial) -> Option<u32> {
        FootSurface::new(landscapes, materials())
            .material_hash(Some(WORLD), surface)
            .unwrap()
    }

    #[test]
    fn a_base_only_quadrant_is_its_base_material() {
        let landscapes = Landscapes::with([((0, 0), Some(vec![layer(DIRT, 0, true, 0.0)]))]);
        assert_eq!(resolve(&landscapes, point(100.0, 100.0)), Some(DIRT_HASH));
    }

    #[test]
    fn an_overlay_over_half_wins_over_the_base() {
        let landscapes = Landscapes::with([(
            (0, 0),
            Some(vec![layer(DIRT, 0, true, 0.0), layer(ROCK, 0, false, 0.6)]),
        )]);
        assert_eq!(resolve(&landscapes, point(100.0, 100.0)), Some(ROCK_HASH));
    }

    #[test]
    fn an_overlay_under_half_loses_to_the_base() {
        let landscapes = Landscapes::with([(
            (0, 0),
            Some(vec![layer(DIRT, 0, true, 0.0), layer(ROCK, 0, false, 0.4)]),
        )]);
        assert_eq!(resolve(&landscapes, point(100.0, 100.0)), Some(DIRT_HASH));
    }

    #[test]
    fn the_quadrant_of_the_foot_decides() {
        let landscapes = Landscapes::with([(
            (0, 0),
            Some(vec![layer(DIRT, 0, true, 0.0), layer(ROCK, 3, true, 0.0)]),
        )]);
        assert_eq!(resolve(&landscapes, point(100.0, 100.0)), Some(DIRT_HASH));
        assert_eq!(resolve(&landscapes, point(3000.0, 3000.0)), Some(ROCK_HASH));
    }

    #[test]
    fn the_cell_under_the_foot_is_found_by_its_grid_position() {
        let landscapes = Landscapes::with([
            ((0, 0), Some(vec![layer(DIRT, 0, true, 0.0)])),
            ((1, -1), Some(vec![layer(ROCK, 2, true, 0.0)])),
        ]);
        assert_eq!(
            resolve(&landscapes, point(4096.0 + 100.0, -4096.0 + 3000.0)),
            Some(ROCK_HASH)
        );
    }

    #[test]
    fn a_landscape_without_layers_has_no_material_and_is_logged_once() {
        let landscapes = Landscapes::with([((0, 0), None)]);
        let mut surface = FootSurface::new(&landscapes, materials());
        for _ in 0..3 {
            assert_eq!(
                surface
                    .material_hash(Some(WORLD), point(100.0, 100.0))
                    .unwrap(),
                None
            );
        }
        assert_eq!(surface.reported_count(), 1);
    }

    #[test]
    fn a_texture_without_material_has_no_material_and_is_logged_once() {
        let landscapes = Landscapes::with([((0, 0), Some(vec![layer(NO_MATERIAL, 0, true, 0.0)]))]);
        let mut surface = FootSurface::new(&landscapes, materials());
        for _ in 0..3 {
            assert_eq!(
                surface
                    .material_hash(Some(WORLD), point(100.0, 100.0))
                    .unwrap(),
                None
            );
        }
        assert_eq!(surface.reported_count(), 1);
    }

    #[test]
    fn a_quadrant_without_layers_has_no_material() {
        let landscapes = Landscapes::with([((0, 0), Some(vec![layer(DIRT, 0, true, 0.0)]))]);
        let mut surface = FootSurface::new(&landscapes, materials());
        assert_eq!(
            surface
                .material_hash(Some(WORLD), point(3000.0, 3000.0))
                .unwrap(),
            None
        );
        assert_eq!(surface.reported_count(), 1);
    }

    #[test]
    fn an_unreadable_landscape_is_an_error_and_is_not_cached() {
        let landscapes = Landscapes::with([]);
        let mut surface = FootSurface::new(&landscapes, materials());
        for expected_reads in 1..=2 {
            let error = surface
                .material_hash(Some(WORLD), point(100.0, 100.0))
                .unwrap_err();
            assert!(format!("{error:#}").contains("no such cell"), "{error:#}");
            assert_eq!(landscapes.reads.get(), expected_reads);
        }
        assert_eq!(surface.reported_count(), 0);
    }

    #[test]
    fn a_cell_is_read_once_while_cached() {
        let landscapes = Landscapes::with([((0, 0), Some(vec![layer(DIRT, 0, true, 0.0)]))]);
        let mut surface = FootSurface::new(&landscapes, materials());
        for _ in 0..3 {
            surface
                .material_hash(Some(WORLD), point(100.0, 100.0))
                .unwrap();
        }
        assert_eq!(landscapes.reads.get(), 1);
    }

    #[test]
    fn the_oldest_cell_is_evicted_past_the_capacity() {
        let cells = (0..=CELL_CACHE_CAPACITY as i32)
            .map(|x| ((x, 0), Some(vec![layer(DIRT, 0, true, 0.0)])))
            .collect::<Vec<_>>();
        let landscapes = Landscapes::with(cells);
        let mut surface = FootSurface::new(&landscapes, materials());
        for x in 0..=CELL_CACHE_CAPACITY {
            surface
                .material_hash(Some(WORLD), point(x as f32 * 4096.0 + 100.0, 100.0))
                .unwrap();
        }
        surface
            .material_hash(Some(WORLD), point(100.0, 100.0))
            .unwrap();
        assert_eq!(landscapes.reads.get(), CELL_CACHE_CAPACITY + 2);
    }

    #[test]
    fn mesh_and_empty_surfaces_resolve_without_terrain() {
        let landscapes = Landscapes::with([]);
        let mut surface = FootSurface::new(&landscapes, materials());
        assert_eq!(
            surface
                .material_hash(None, SurfaceMaterial::Hash(7))
                .unwrap(),
            Some(7)
        );
        assert_eq!(
            surface.material_hash(None, SurfaceMaterial::None).unwrap(),
            None
        );
    }

    #[test]
    fn terrain_in_an_interior_has_no_material() {
        let landscapes = Landscapes::with([((0, 0), Some(vec![layer(DIRT, 0, true, 0.0)]))]);
        let mut surface = FootSurface::new(&landscapes, materials());
        assert_eq!(
            surface.material_hash(None, point(100.0, 100.0)).unwrap(),
            None
        );
        assert_eq!(landscapes.reads.get(), 0);
    }
}
