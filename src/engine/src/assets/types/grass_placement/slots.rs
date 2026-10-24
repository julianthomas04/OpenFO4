//! Per-patch grass slot gathering: which grass types grow around a patch
//! centre, and how dense, from the base and overlay layers of one quadrant.

use crate::assets::types::terrain::{ALPHA_GRID, LandscapeLayer, base_layer};

use super::{GrassCatalog, GrassSettings};

/// Overlay layers counted per quadrant.
pub const MAX_OVERLAY_LAYERS: usize = 12;
/// Grass slots one patch can hold.
pub const MAX_SLOTS_PER_PATCH: usize = 16;
/// Opacity bytes at or below this on all nine vertices skip an overlay layer.
const OVERLAY_PREFILTER_BYTE: u8 = 25;
const NEIGHBOURS: usize = 9;
const BYTE_TO_UNIT: f32 = 1.0 / 255.0;
const NINTH: f32 = 1.0 / 9.0;
const PERCENT: f32 = 0.01;

type OpacityBytes = [[u8; ALPHA_GRID]; ALPHA_GRID];

pub struct OverlayLayer<'a> {
    pub texture_key: Option<&'a str>,
    pub opacity: OpacityBytes,
}

/// The layers of one landscape quadrant in the shape the gathering reads.
pub struct QuadrantLayers<'a> {
    pub base_texture_key: Option<&'a str>,
    pub overlays: Vec<OverlayLayer<'a>>,
}

impl<'a> QuadrantLayers<'a> {
    /// A quadrant has at most one base layer (the last record wins) and keeps
    /// its first [`MAX_OVERLAY_LAYERS`] overlays in record order.
    pub fn of(layers: &'a [LandscapeLayer], quadrant: u8) -> Self {
        let in_quadrant = || layers.iter().filter(|layer| layer.quadrant == quadrant);
        let base_texture_key =
            base_layer(layers, quadrant).and_then(|layer| layer.texture_key.as_deref());
        let overlays = in_quadrant()
            .filter(|layer| !layer.is_base)
            .take(MAX_OVERLAY_LAYERS)
            .map(|layer| OverlayLayer {
                texture_key: layer.texture_key.as_deref(),
                opacity: layer
                    .alpha_grid
                    .as_ref()
                    .map_or([[0; ALPHA_GRID]; ALPHA_GRID], |grid| {
                        grid.map(|row| row.map(opacity_byte))
                    }),
            })
            .collect();
        Self {
            base_texture_key,
            overlays,
        }
    }

    /// The share of the base texture at a vertex: what the overlays leave.
    fn base_weight(&self, col: usize, row: usize) -> f32 {
        let overlay_sum: u32 = self
            .overlays
            .iter()
            .map(|overlay| u32::from(overlay.opacity[row][col]))
            .sum();
        1.0 - overlay_sum as f32 * BYTE_TO_UNIT
    }
}

fn opacity_byte(opacity: f32) -> u8 {
    (opacity.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// One grass type growing in a patch, with its density at the 3 x 3 vertices
/// around the patch centre (index `dx + 3 * dy`, `dx`/`dy` in `0..3`).
#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    pub grass: usize,
    pub density: [f32; NEIGHBOURS],
}

/// The grass slots of the patch centred on vertex `(col, row)` of a quadrant
/// (`col` and `row` odd, in `1..=15`).
pub fn gather_slots(
    quadrant: &QuadrantLayers,
    catalog: &GrassCatalog,
    settings: &GrassSettings,
    col: usize,
    row: usize,
) -> Vec<Slot> {
    let threshold = settings.coverage_threshold();
    let overlay_byte_threshold = (threshold * 255.0) as u32;
    let max_types = settings.max_types_per_texture;
    let mut slots: Vec<Slot> = Vec::new();

    if let Some(base) = quadrant.base_texture_key {
        for (used, &grass) in catalog.types_of(base).iter().enumerate() {
            if used > max_types || slots.len() >= MAX_SLOTS_PER_PATCH {
                break;
            }
            let density = f32::from(catalog.get(grass).density) * PERCENT;
            let covered = neighbourhood(col, row, |c, r| quadrant.base_weight(c, r) > threshold);
            slots.push(slot_of(grass, covered, density, threshold));
        }
    }

    for overlay in &quadrant.overlays {
        let Some(texture) = overlay.texture_key else {
            continue;
        };
        let opacity_at = |c: usize, r: usize| u32::from(overlay.opacity[r][c]);
        if neighbourhood(col, row, |c, r| {
            opacity_at(c, r) > u32::from(OVERLAY_PREFILTER_BYTE)
        })
        .iter()
        .all(|&covered| !covered)
        {
            continue;
        }
        for (used, &grass) in catalog.types_of(texture).iter().enumerate() {
            if used > max_types || slots.len() >= MAX_SLOTS_PER_PATCH {
                break;
            }
            let density = f32::from(catalog.get(grass).density) * PERCENT;
            let covered = neighbourhood(col, row, |c, r| opacity_at(c, r) > overlay_byte_threshold);
            slots.push(slot_of(grass, covered, density, threshold));
        }
    }
    slots
}

/// `test` over the nine vertices around `(col, row)`, `dx + 3 * dy` order.
fn neighbourhood(
    col: usize,
    row: usize,
    test: impl Fn(usize, usize) -> bool,
) -> [bool; NEIGHBOURS] {
    std::array::from_fn(|k| test(col + k % 3 - 1, row + k / 3 - 1))
}

/// A slot whose nine densities average below the threshold keeps its index
/// but grows nothing.
fn slot_of(grass: usize, covered: [bool; NEIGHBOURS], density: f32, threshold: f32) -> Slot {
    let mut densities = covered.map(|covered| if covered { density } else { 0.0 });
    if densities.iter().sum::<f32>() * NINTH < threshold {
        densities = [0.0; NEIGHBOURS];
    }
    Slot {
        grass,
        density: densities,
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    clippy::unnecessary_wraps,
    reason = "expected values are exact literals; the helper mirrors the layer field's Option"
)]
mod tests {
    use super::super::test_support::{catalog_of, grass_settings, layer};
    use super::*;

    const CENTRE: usize = 5;

    fn gather(
        layers: &[LandscapeLayer],
        catalog: &GrassCatalog,
        settings: &GrassSettings,
    ) -> Vec<Slot> {
        gather_slots(
            &QuadrantLayers::of(layers, 0),
            catalog,
            settings,
            CENTRE,
            CENTRE,
        )
    }

    fn uniform_alpha(opacity: f32) -> Option<[[f32; ALPHA_GRID]; ALPHA_GRID]> {
        Some([[opacity; ALPHA_GRID]; ALPHA_GRID])
    }

    #[test]
    fn a_texture_yields_at_most_max_plus_one_grass_types() {
        let catalog = catalog_of(&[(
            "L",
            &[("a", 50), ("b", 50), ("c", 50), ("d", 50), ("e", 50)],
        )]);
        let layers = [layer("L", 0, true, None)];

        let slots = gather(&layers, &catalog, &grass_settings());
        assert_eq!(slots.iter().map(|s| s.grass).collect::<Vec<_>>(), [0, 1, 2]);

        let one = GrassSettings {
            max_types_per_texture: 0,
            ..grass_settings()
        };
        assert_eq!(gather(&layers, &catalog, &one).len(), 1);
    }

    #[test]
    fn a_patch_holds_at_most_sixteen_slots() {
        let catalog = catalog_of(&[("L", &[("a", 50), ("b", 50), ("c", 50)])]);
        let mut layers = vec![layer("L", 0, true, None)];
        for _ in 0..12 {
            layers.push(layer("L", 0, false, uniform_alpha(1.0)));
        }
        // 3 base + 12 overlays x 3 would be 39.
        assert_eq!(
            gather(&layers, &catalog, &grass_settings()).len(),
            MAX_SLOTS_PER_PATCH
        );
    }

    #[test]
    fn a_quadrant_counts_only_its_first_twelve_overlays() {
        let catalog = catalog_of(&[("L", &[("a", 50)]), ("late", &[("z", 50)])]);
        let mut layers: Vec<LandscapeLayer> = (0..12)
            .map(|_| layer("L", 0, false, uniform_alpha(0.0)))
            .collect();
        layers.push(layer("late", 0, false, uniform_alpha(1.0)));
        assert_eq!(QuadrantLayers::of(&layers, 0).overlays.len(), 12);
        assert_eq!(gather(&layers, &catalog, &grass_settings()), []);
    }

    #[test]
    fn overlay_at_the_prefilter_byte_is_skipped_one_above_grows() {
        let catalog = catalog_of(&[("O", &[("a", 100)])]);
        let flat = |bytes: f32| [layer("O", 0, false, uniform_alpha(bytes / 255.0))];
        assert_eq!(gather(&flat(25.0), &catalog, &grass_settings()), []);
        let slots = gather(&flat(26.0), &catalog, &grass_settings());
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].density, [1.0; 9]);
    }

    #[test]
    fn one_vertex_above_the_prefilter_is_enough_but_only_it_gets_density() {
        let catalog = catalog_of(&[("O", &[("a", 100)])]);
        let mut grid = [[0.0; ALPHA_GRID]; ALPHA_GRID];
        // Row CENTRE+1, column CENTRE-1: neighbour index k = 0 + 3 * 2.
        grid[CENTRE + 1][CENTRE - 1] = 1.0;
        let layers = [layer("O", 0, false, Some(grid))];
        // One of nine vertices at density 1.0 averages 0.111, above the threshold.
        let slots = gather(&layers, &catalog, &grass_settings());
        let mut expected = [0.0; 9];
        expected[6] = 1.0;
        assert_eq!(slots[0].density, expected);
    }

    #[test]
    fn base_weight_is_what_the_overlays_leave() {
        let catalog = catalog_of(&[("B", &[("a", 100)])]);
        let settings = grass_settings();
        // 253/255 overlay leaves 2/255 = 0.0078 > 0.005: base grows.
        let grows = [
            layer("B", 0, true, None),
            layer("none", 0, false, uniform_alpha(253.0 / 255.0)),
        ];
        assert_eq!(gather(&grows, &catalog, &settings)[0].density, [1.0; 9]);
        // 254/255 leaves 1/255 = 0.0039 < 0.005: slot kept, nothing grows.
        let bare = [
            layer("B", 0, true, None),
            layer("none", 0, false, uniform_alpha(254.0 / 255.0)),
        ];
        let slots = gather(&bare, &catalog, &settings);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].density, [0.0; 9]);
    }

    #[test]
    fn several_overlays_all_reduce_the_base() {
        let catalog = catalog_of(&[("B", &[("a", 100)])]);
        // 2 x 128/255 = 1.0039: the base is gone although each overlay alone leaves half.
        let layers = [
            layer("B", 0, true, None),
            layer("x", 0, false, uniform_alpha(128.0 / 255.0)),
            layer("y", 0, false, uniform_alpha(128.0 / 255.0)),
        ];
        assert_eq!(
            gather(&layers, &catalog, &grass_settings())[0].density,
            [0.0; 9]
        );
    }

    #[test]
    fn a_slot_averaging_below_the_threshold_is_zeroed_but_keeps_its_index() {
        let settings = grass_settings();
        let mut grid = [[0.0; ALPHA_GRID]; ALPHA_GRID];
        grid[CENTRE][CENTRE] = 1.0;
        // density 1% -> 0.01 / 9 = 0.0011 < 0.005; density 5% -> 0.0056 >= 0.005.
        let catalog = catalog_of(&[("O", &[("faint", 1), ("dense", 5)])]);
        let slots = gather(&[layer("O", 0, false, Some(grid))], &catalog, &settings);
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].density, [0.0; 9]);
        assert!((slots[1].density[4] - 0.05).abs() < 1e-6);
    }

    #[test]
    fn the_threshold_is_clamped_and_replaced_when_oversized() {
        let at = |value| GrassSettings {
            texture_pct_threshold: value,
            ..grass_settings()
        };
        assert_eq!(at(-0.3).coverage_threshold(), 0.0);
        assert_eq!(at(0.005).coverage_threshold(), 0.005);
        assert_eq!(at(1.0).coverage_threshold(), 1.0);
        assert_eq!(at(1.5).coverage_threshold(), 0.9);
    }

    #[test]
    fn edge_patches_read_the_outermost_vertices_as_neighbours() {
        let catalog = catalog_of(&[("O", &[("a", 100)])]);
        let last = ALPHA_GRID - 1;
        let mut grid = [[0.0; ALPHA_GRID]; ALPHA_GRID];
        grid[0][0] = 1.0;
        grid[last][last] = 1.0;
        let layers = [layer("O", 0, false, Some(grid))];
        let quadrant = QuadrantLayers::of(&layers, 0);
        let settings = grass_settings();

        let south_west = gather_slots(&quadrant, &catalog, &settings, 1, 1);
        let mut expected = [0.0; 9];
        expected[0] = 1.0;
        assert_eq!(south_west[0].density, expected);

        let north_east = gather_slots(&quadrant, &catalog, &settings, last - 1, last - 1);
        let mut expected = [0.0; 9];
        expected[8] = 1.0;
        assert_eq!(north_east[0].density, expected);
    }

    #[test]
    fn layers_of_other_quadrants_are_ignored() {
        let catalog = catalog_of(&[("L", &[("a", 50)])]);
        let layers = [layer("L", 2, true, None)];
        assert_eq!(gather(&layers, &catalog, &grass_settings()), []);
        assert_eq!(
            gather_slots(
                &QuadrantLayers::of(&layers, 2),
                &catalog,
                &grass_settings(),
                3,
                3
            )
            .len(),
            1
        );
    }
}
