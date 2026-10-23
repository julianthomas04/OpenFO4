//! The terrain surface a blade lands on: height, normal and vertex colour at
//! an arbitrary point of the cell, interpolated over the same triangles and
//! per-vertex normals the rendered terrain mesh uses.

use glam::Vec3;

use crate::assets::types::terrain::{HeightGrid, TERRAIN_QUAD_SIZE, TERRAIN_QUADS, TERRAIN_VERTS};
use crate::bethesda_space::axes_from_engine;

const COLOUR_BYTES: usize = TERRAIN_VERTS * TERRAIN_VERTS * 3;

/// What lies at one point of the terrain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfacePoint {
    /// Height in Bethesda units.
    pub height: f32,
    /// Unit normal in engine space (Y up).
    pub normal: Vec3,
    /// Vertex colour, each channel in `[0, 1]`.
    pub colour: [f32; 3],
}

impl SurfacePoint {
    /// Vertical component of the normal in Bethesda space (Z up), the value
    /// the slope filter compares.
    pub fn slope_cosine(&self) -> f32 {
        axes_from_engine(self.normal).z
    }
}

pub struct TerrainSurface<'a> {
    heights: &'a HeightGrid,
    colours: Option<&'a [u8]>,
}

impl<'a> TerrainSurface<'a> {
    /// `colours` is the `landscapes.vertex_colors` column (33 x 33 RGB in rows
    /// of constant y). Absent or short data renders as white, the default
    /// vertex colour of a land record without `VCLR`.
    pub fn new(heights: &'a HeightGrid, colours: Option<&'a [u8]>) -> Self {
        let colours = colours.filter(|bytes| {
            let complete = bytes.len() >= COLOUR_BYTES;
            if !complete {
                crate::utils::warn_once("grass-short-vertex-colours", || {
                    format!(
                        "grass: a landscape vertex colour blob has {} bytes, expected {COLOUR_BYTES}; \
                         treating such cells as white",
                        bytes.len()
                    )
                });
            }
            complete
        });
        Self { heights, colours }
    }

    /// The surface at `(x, y)` Bethesda units from the cell's south-west
    /// corner; points outside the cell clamp to its edge.
    pub fn sample(&self, x: f32, y: f32) -> SurfacePoint {
        let (qx, fx) = split_coordinate(x);
        let (qy, fy) = split_coordinate(y);

        // The mesh splits each quad along the (x+1, y) - (x, y+1) diagonal.
        let corners_and_weights: [((usize, usize), f32); 3] = if fx + fy <= 1.0 {
            [
                ((qx, qy), 1.0 - fx - fy),
                ((qx + 1, qy), fx),
                ((qx, qy + 1), fy),
            ]
        } else {
            [
                ((qx + 1, qy), 1.0 - fy),
                ((qx + 1, qy + 1), fx + fy - 1.0),
                ((qx, qy + 1), 1.0 - fx),
            ]
        };

        let mut height = 0.0;
        let mut normal = Vec3::ZERO;
        let mut colour = [0.0; 3];
        for ((vx, vy), weight) in corners_and_weights {
            height += weight * self.heights.height(vx, vy);
            normal += weight * Vec3::from(self.heights.normal(vx, vy, 1));
            let vertex_colour = self.colour_at(vx, vy);
            for (channel, value) in colour.iter_mut().zip(vertex_colour) {
                *channel += weight * value;
            }
        }
        SurfacePoint {
            height,
            normal: normal.normalize(),
            colour,
        }
    }

    fn colour_at(&self, x: usize, y: usize) -> [f32; 3] {
        let Some(bytes) = self.colours else {
            return [1.0; 3];
        };
        let at = (y * TERRAIN_VERTS + x) * 3;
        [bytes[at], bytes[at + 1], bytes[at + 2]].map(|byte| f32::from(byte) / 255.0)
    }
}

/// Quad index and the fraction inside it for a coordinate in Bethesda units.
fn split_coordinate(coordinate: f32) -> (usize, f32) {
    let scaled = (coordinate / TERRAIN_QUAD_SIZE).clamp(0.0, TERRAIN_QUADS as f32);
    let quad = (scaled as usize).min(TERRAIN_QUADS - 1);
    (quad, scaled - quad as f32)
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals or exact table entries"
)]
mod tests {
    use super::super::test_support::height_blob as blob;
    use super::*;

    fn flat_grid(offset: f32) -> HeightGrid {
        HeightGrid::decode(&blob(offset, |_, _| 0)).unwrap()
    }

    /// Height rises 1 delta step (8 units) per vertex along x, flat in y.
    fn ramp_x_grid() -> HeightGrid {
        HeightGrid::decode(&blob(0.0, |x, _| i8::from(x > 0))).unwrap()
    }

    #[test]
    fn flat_terrain_has_the_offset_height_and_an_upright_normal() {
        let grid = flat_grid(2.0);
        let point = TerrainSurface::new(&grid, None).sample(1000.0, 2000.0);
        assert_eq!(point.height, 16.0);
        assert!((point.normal - Vec3::Y).length() < 1e-6);
        assert!((point.slope_cosine() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn height_is_linear_between_vertices_along_a_ramp() {
        let grid = ramp_x_grid();
        let surface = TerrainSurface::new(&grid, None);
        // Vertex x=3 sits 24 units up; a quarter of the way to x=4 adds 2.
        let point = surface.sample(3.25 * 128.0, 500.0);
        assert!((point.height - 26.0).abs() < 1e-4, "{}", point.height);
    }

    #[test]
    fn the_diagonal_picks_the_triangle_a_point_lies_in() {
        // Only vertex (1,1) is raised: height 8 above its neighbours' rows.
        let grid = HeightGrid::decode(&blob(0.0, |x, y| match (x, y) {
            (1, 1) => 1,
            (2, 1) | (1, 2) => -1,
            _ => 0,
        }))
        .unwrap();
        let surface = TerrainSurface::new(&grid, None);
        // Heights: h(0,0)=0 ... a point on the quad (1,1)->(2,2) lies in the
        // upper triangle (10, 11, 01) when fx + fy > 1; there the height must
        // interpolate vertices (2,1), (2,2), (1,2).
        let upper = surface.sample(1.75 * 128.0, 1.75 * 128.0);
        let expected =
            0.25 * grid.height(2, 1) + 0.5 * grid.height(2, 2) + 0.25 * grid.height(1, 2);
        assert!(
            (upper.height - expected).abs() < 1e-4,
            "{} vs {expected}",
            upper.height
        );
        let lower = surface.sample(1.25 * 128.0, 1.25 * 128.0);
        let expected =
            0.5 * grid.height(1, 1) + 0.25 * grid.height(2, 1) + 0.25 * grid.height(1, 2);
        assert!(
            (lower.height - expected).abs() < 1e-4,
            "{} vs {expected}",
            lower.height
        );
    }

    #[test]
    fn a_ramp_tilts_the_normal_against_the_climb() {
        let grid = ramp_x_grid();
        let point = TerrainSurface::new(&grid, None).sample(5.5 * 128.0, 900.0);
        // Rise of 8 units per 128 -> slope 1/16 along +x: normal leans to -x.
        assert!(
            point.normal.x < -0.06 && point.normal.x > -0.063,
            "{:?}",
            point.normal
        );
        assert!(point.slope_cosine() < 1.0);
    }

    #[test]
    fn colours_interpolate_and_default_to_white() {
        let grid = flat_grid(0.0);
        assert_eq!(
            TerrainSurface::new(&grid, None).sample(10.0, 10.0).colour,
            [1.0; 3]
        );

        // Red 255 on every vertex of column x=0, 0 elsewhere.
        let mut colours = vec![0u8; COLOUR_BYTES];
        for y in 0..TERRAIN_VERTS {
            colours[y * TERRAIN_VERTS * 3] = 255;
        }
        let point = TerrainSurface::new(&grid, Some(&colours)).sample(32.0, 40.0);
        assert!((point.colour[0] - 0.75).abs() < 1e-5, "{:?}", point.colour);
        assert_eq!(point.colour[1], 0.0);
    }

    #[test]
    fn a_short_colour_blob_renders_as_white() {
        let grid = flat_grid(0.0);
        let short = vec![0u8; COLOUR_BYTES - 1];
        assert_eq!(
            TerrainSurface::new(&grid, Some(&short))
                .sample(10.0, 10.0)
                .colour,
            [1.0; 3]
        );
    }

    #[test]
    fn points_outside_the_cell_clamp_to_the_edge() {
        let grid = ramp_x_grid();
        let surface = TerrainSurface::new(&grid, None);
        assert_eq!(surface.sample(-50.0, 0.0), surface.sample(0.0, 0.0));
        assert_eq!(surface.sample(9999.0, 0.0), surface.sample(4096.0, 0.0));
    }
}
