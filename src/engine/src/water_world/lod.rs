//! Distant ("LOD") water: one flat 4096 x 4096 game-unit quad per water cell,
//! baked into the level-4 terrain LOD blocks at the cell's water height. The
//! game draws them with the worldspace's LOD water type, under and around the
//! loaded cells too, out to the distance in the `fBlockMaximumDistance` INI key.

use std::sync::Arc;

use glam::{Vec2, Vec3};

use crate::assets::db::water_placement::LodWaterQuad;
use crate::bethesda_space::UNITS_TO_ENGINE;

/// Side of a cell, game units.
const CELL_SIZE: f32 = 4096.0;

/// A flat water square covering one exterior cell, placed in engine space:
/// a distant LOD quad, or the surface of a loaded cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellPlane {
    /// The exterior grid cell it covers.
    pub cell: (i32, i32),
    /// Horizontal extent on the engine X and Z axes.
    pub min: Vec2,
    pub max: Vec2,
    /// The baked height of the quad's vertices (engine units).
    pub height: f32,
}

impl CellPlane {
    /// The square of cell (`cell_x`, `cell_y`) at `height` game units.
    pub fn of_cell(cell_x: i32, cell_y: i32, height: f32) -> Self {
        let size = CELL_SIZE * UNITS_TO_ENGINE;
        let x = cell_x as f32 * size;
        // Bethesda Y maps to engine -Z.
        let z_far = -(cell_y as f32) * size;
        Self {
            cell: (cell_x, cell_y),
            min: Vec2::new(x, z_far - size),
            max: Vec2::new(x + size, z_far),
            height: height * UNITS_TO_ENGINE,
        }
    }

    /// Horizontal distance from `point` to the nearest point of the quad.
    fn distance_to(&self, point: Vec3) -> f32 {
        let nearest = Vec2::new(point.x, point.z).clamp(self.min, self.max);
        nearest.distance(Vec2::new(point.x, point.z))
    }
}

/// The LOD water of one worldspace.
#[derive(Debug, Clone)]
pub struct LodWater {
    planes: Vec<CellPlane>,
    /// The worldspace's LOD water type (`NAM3` with parent inheritance).
    pub water_type: Arc<str>,
    /// `NAM4`, the plane constant the game gives every LOD water shape, whatever height
    /// the quad's vertices carry; engine units. `None` when the worldspace sets none.
    pub plane_height: Option<f32>,
}

impl LodWater {
    pub fn new(quads: &[LodWaterQuad], water_type: &str, plane_height: Option<f32>) -> Self {
        Self {
            planes: quads
                .iter()
                .map(|quad| CellPlane::of_cell(quad.cell_x, quad.cell_y, quad.height))
                .collect(),
            water_type: Arc::from(water_type),
            plane_height: plane_height.map(|height| height * UNITS_TO_ENGINE),
        }
    }

    /// Every quad lifted by `game_units`: a debug probe that shows whether
    /// geometry between the quad height and the lifted height is what keeps
    /// the water from drawing (`OPENFO4_WATER_LOD_RAISE`).
    #[must_use]
    pub fn raised(mut self, game_units: f32) -> Self {
        let lift = game_units * UNITS_TO_ENGINE;
        for plane in &mut self.planes {
            plane.height += lift;
        }
        self
    }

    pub fn none() -> Self {
        Self::new(
            &[],
            crate::water_world::height::DEFAULT_WATER_FORM_KEY,
            None,
        )
    }

    pub fn len(&self) -> usize {
        self.planes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.planes.is_empty()
    }

    /// The quads whose nearest point is within `range_game_units` of `camera` (engine units).
    pub fn within_range(
        &self,
        camera: Vec3,
        range_game_units: f32,
    ) -> impl Iterator<Item = &CellPlane> {
        let range = range_game_units * UNITS_TO_ENGINE;
        self.planes
            .iter()
            .filter(move |plane| plane.distance_to(camera) <= range)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(cell_x: i32, cell_y: i32, height: f32) -> LodWaterQuad {
        LodWaterQuad {
            cell_x,
            cell_y,
            height,
        }
    }

    #[test]
    fn a_cell_quad_covers_its_4096_unit_square_in_engine_axes() {
        let plane = CellPlane::of_cell(2, 3, 450.0);
        assert!(plane.min.abs_diff_eq(Vec2::new(819.2, -1638.4), 1e-3));
        assert!(plane.max.abs_diff_eq(Vec2::new(1228.8, -1228.8), 1e-3));
        assert!((plane.height - 45.0).abs() < 1e-4);
    }

    #[test]
    fn negative_cells_land_on_the_other_side_of_the_origin() {
        let plane = CellPlane::of_cell(-1, -1, 0.0);
        assert!(plane.min.abs_diff_eq(Vec2::new(-409.6, 0.0), 1e-3));
        assert!(plane.max.abs_diff_eq(Vec2::new(0.0, 409.6), 1e-3));
    }

    /// A made-up range, game units; the tests pass it in like the configured one.
    const RANGE: f32 = 100_000.0;

    #[test]
    fn the_range_test_uses_the_nearest_point_of_the_quad() {
        let water = LodWater::new(&[quad(0, 0, 450.0)], "w", None);
        let range = RANGE * UNITS_TO_ENGINE;
        let inside = Vec3::new(-range + 400.0, 0.0, -200.0);
        let outside = Vec3::new(-range - 1.0, 0.0, -200.0);
        assert_eq!(water.within_range(inside, RANGE).count(), 1);
        assert_eq!(water.within_range(outside, RANGE).count(), 0);
    }

    #[test]
    fn a_quad_exactly_at_the_range_is_in_range() {
        let water = LodWater::new(&[quad(0, 0, 450.0)], "w", None);
        let camera = Vec3::new(-RANGE * UNITS_TO_ENGINE, 0.0, -200.0);
        assert_eq!(water.within_range(camera, RANGE).count(), 1);
    }

    #[test]
    fn the_range_is_the_one_passed_in() {
        let water = LodWater::new(&[quad(0, 0, 450.0)], "w", None);
        let camera = Vec3::new(-RANGE * UNITS_TO_ENGINE, 0.0, -200.0);
        assert_eq!(water.within_range(camera, RANGE * 0.5).count(), 0);
        assert_eq!(water.within_range(camera, RANGE * 2.0).count(), 1);
    }

    #[test]
    fn the_range_is_measured_horizontally() {
        let water = LodWater::new(&[quad(0, 0, 450.0)], "w", None);
        let high_above = Vec3::new(200.0, RANGE * UNITS_TO_ENGINE * 4.0, -200.0);
        assert_eq!(water.within_range(high_above, RANGE).count(), 1);
    }

    #[test]
    fn a_camera_over_the_quad_is_in_range() {
        let water = LodWater::new(&[quad(0, 0, 450.0)], "w", None);
        let camera = Vec3::new(10.0, 500.0, -10.0);
        assert_eq!(water.within_range(camera, RANGE).count(), 1);
    }

    #[test]
    fn raising_lifts_every_quad_by_the_game_units_in_engine_units() {
        let water =
            LodWater::new(&[quad(0, 0, 450.0), quad(1, 0, 0.0)], "w", Some(450.0)).raised(100.0);
        let heights: Vec<f32> = water.planes.iter().map(|plane| plane.height).collect();
        assert!((heights[0] - 55.0).abs() < 1e-4 && (heights[1] - 10.0).abs() < 1e-4);
        assert!((water.plane_height.unwrap() - 45.0).abs() < 1e-4);
    }

    #[test]
    fn the_plane_height_is_converted_to_engine_units() {
        let water = LodWater::new(&[], "w", Some(450.0));
        assert!((water.plane_height.unwrap() - 45.0).abs() < 1e-4);
        assert!(water.is_empty());
    }
}
