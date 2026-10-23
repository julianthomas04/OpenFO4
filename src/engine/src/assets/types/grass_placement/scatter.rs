//! Scattering one grass slot over one 256 x 256 patch:
//! lattice, density-driven acceptance, jitter, terrain filters and the
//! per-blade variation draws.

use crate::assets::types::grass::GrassBlade;
use crate::bethesda_space::position_to_engine;

use super::basis::blade_rotation;
use super::mt19937::Mt19937;
use super::slots::Slot;
use super::surface::{SurfacePoint, TerrainSurface};
use super::tables::{cosine_of_degrees, random_at, unit_float};
use super::{GrassSettings, GrassType};

pub const PATCH_SIDE: f32 = 256.0;
const ACCEPTANCE_SCALE: f32 = 32768.0;
/// `randint(0, 0x7fff)`: the span of the acceptance draw.
const ACCEPTANCE_SPAN: f32 = 32767.0;
const LUMA_WEIGHTS: [f32; 3] = [0.31, 0.37, 0.32];
const BRIGHTNESS_CEILING: f32 = 0.99;

/// Everything a patch scatter reads or advances.
pub struct ScatterContext<'a> {
    /// Landscape cell grid coordinates.
    pub cell: (i32, i32),
    /// South-west corner of the cell in Bethesda units.
    pub cell_origin: (f32, f32),
    pub surface: &'a TerrainSurface<'a>,
    /// Effective water height in Bethesda units; `-f32::MAX` without water.
    pub water_height: f32,
    pub settings: &'a GrassSettings,
    /// Acceptance draws; one generator per cell.
    pub acceptance: &'a mut Mt19937,
}

/// Float to `u32` conversion: values beyond the `i64` range and NaN give 0, and
/// anything else keeps the low 32 bits of the truncated integer.
fn truncate_to_u32(value: f32) -> u32 {
    if value.is_finite() && value.abs() < 9.0e18 {
        (value as i64) as u32
    } else {
        0
    }
}

/// Lattice cells per patch side: limited by the grass type's position range
/// and the minimum grass size; 0 grows nothing.
pub fn lattice_size(position_range: f32, min_grass_size: i32) -> u32 {
    truncate_to_u32(PATCH_SIDE / position_range)
        .min(truncate_to_u32(PATCH_SIDE / min_grass_size as f32))
}

/// Whether terrain height `z` passes the grass type's water rule (`W` water
/// height, `D` units from water, all in Bethesda units).
pub fn passes_water_filter(rule: u32, z: f32, water: f32, distance: f32) -> bool {
    match rule {
        // Rule 0 compares against W - D, unlike the other "above" rules, which use W + D.
        0 => z >= water - distance,
        1 => z >= water && z <= water + distance,
        2 => z <= water - distance,
        3 => z <= water && z >= water - distance,
        4 => z >= water + distance || z <= water - distance,
        5 => z >= water - distance && z <= water + distance,
        _ => true,
    }
}

/// Blades of `slot` in the patch whose south-west corner is `patch_origin`
/// (world Bethesda units), appended to `blades`.
pub fn scatter_slot(
    context: &mut ScatterContext,
    slot: &Slot,
    grass: &GrassType,
    patch_origin: (f32, f32),
    blades: &mut Vec<GrassBlade>,
) {
    let n = lattice_size(grass.position_range, context.settings.min_grass_size);
    if n == 0 {
        return;
    }
    let lattice = n as f32;
    let step = PATCH_SIDE / lattice;
    let half_step = step * 0.5;
    let slope_max_cosine = cosine_of_degrees(grass.max_slope);
    let slope_min_cosine = cosine_of_degrees(grass.min_slope);
    let mut counter = context.cell.0.wrapping_mul(context.cell.1);
    let mut next_table_value = || {
        let value = random_at(counter);
        counter = counter.wrapping_add(1);
        value
    };

    for i in 0..n {
        for j in 0..n {
            let u = 2.0 * (i as f32 + 0.5) / lattice;
            let v = 2.0 * (j as f32 + 0.5) / lattice;
            let density = bilinear_density(&slot.density, u, v);
            let threshold = (density * ACCEPTANCE_SCALE) as i32;
            if threshold == 0 {
                continue;
            }
            let draw = (unit_float(context.acceptance.next_u32()) * ACCEPTANCE_SPAN) as i32;
            if draw >= threshold {
                continue;
            }

            let jitter_x = (2.0 * next_table_value() - 1.0) * half_step;
            let jitter_y = (2.0 * next_table_value() - 1.0) * half_step;
            let x = patch_origin.0 + (i as f32 + 0.5) * step + jitter_x;
            let y = patch_origin.1 + (j as f32 + 0.5) * step + jitter_y;
            let point = context
                .surface
                .sample(x - context.cell_origin.0, y - context.cell_origin.1);

            let slope = point.slope_cosine();
            if slope < slope_max_cosine || slope > slope_min_cosine {
                continue;
            }
            let water_distance = f32::from(grass.units_from_water);
            if !passes_water_filter(
                grass.units_from_water_type,
                point.height,
                context.water_height,
                water_distance,
            ) {
                continue;
            }

            let tint = tint_of(&point, grass.color_range, next_table_value());
            let yaw_cos = 2.0 * next_table_value() - 1.0;
            let height_k = (2.0 * next_table_value() - 1.0) * grass.height_range;
            let frame = blade_rotation(yaw_cos, point.normal, grass.flags.fit_to_slope());
            blades.push(GrassBlade::new(
                position_to_engine(x, y, point.height).to_array(),
                height_k,
                frame.x_axis.to_array(),
                tint,
                frame.y_axis.to_array(),
            ));
        }
    }
}

/// Density at lattice-cell centre `(u, v)` in `[0, 2)` of the 3 x 3 vertex
/// densities, bilinearly interpolated.
fn bilinear_density(densities: &[f32; 9], u: f32, v: f32) -> f32 {
    let (ix, iy) = (u as usize, v as usize);
    let (fx, fy) = (u - ix as f32, v - iy as f32);
    let at = |x: usize, y: usize| densities[x + 3 * y];
    let lower = (1.0 - fx) * at(ix, iy) + fx * at(ix + 1, iy);
    let upper = (1.0 - fx) * at(ix, iy + 1) + fx * at(ix + 1, iy + 1);
    lower * (1.0 - fy) + upper * fy
}

/// Terrain luma scaled by the colour-range variation, capped just below 1.
fn tint_of(point: &SurfacePoint, color_range: f32, variation: f32) -> f32 {
    let luma: f32 = point
        .colour
        .iter()
        .zip(LUMA_WEIGHTS)
        .map(|(channel, weight)| channel * weight)
        .sum();
    let brightness = luma * ((1.0 - color_range) + (color_range * 0.5) * (variation - 0.5));
    if brightness >= 1.0 {
        BRIGHTNESS_CEILING
    } else {
        brightness.max(0.0)
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals or exact table entries"
)]
mod tests {
    use glam::Vec3;

    use super::*;

    #[test]
    fn lattice_is_limited_by_position_range_and_minimum_size() {
        // 256/32 = 8 < 256/20 = 12
        assert_eq!(lattice_size(32.0, 20), 8);
        // 256/12 = 21 > 12
        assert_eq!(lattice_size(12.0, 20), 12);
        // 256/18 = 14.2 -> min(14, 12)
        assert_eq!(lattice_size(18.0, 20), 12);
        // 256/100 = 2.56 -> 2
        assert_eq!(lattice_size(100.0, 20), 2);
    }

    #[test]
    fn zero_position_range_grows_nothing() {
        assert_eq!(lattice_size(0.0, 20), 0);
    }

    #[test]
    fn negative_range_defers_to_the_minimum_size() {
        assert_eq!(lattice_size(-5.0, 20), 12);
    }

    #[test]
    fn minimum_size_zero_still_bounded_by_range() {
        // 256/0 overflows to 0 in the conversion, so nothing grows.
        assert_eq!(lattice_size(32.0, 0), 0);
    }

    const W: f32 = 100.0;
    const D: f32 = 10.0;

    fn passes(rule: u32, z: f32) -> bool {
        passes_water_filter(rule, z, W, D)
    }

    #[test]
    fn rule_0_accepts_from_water_minus_distance_up() {
        assert!(passes(0, 90.0));
        assert!(!passes(0, 89.9));
        assert!(passes(0, 500.0));
    }

    #[test]
    fn rule_1_accepts_the_band_above_the_water() {
        assert!(passes(1, 100.0));
        assert!(passes(1, 110.0));
        assert!(!passes(1, 99.9));
        assert!(!passes(1, 110.1));
    }

    #[test]
    fn rule_2_accepts_at_or_below_water_minus_distance() {
        assert!(passes(2, 90.0));
        assert!(!passes(2, 90.1));
    }

    #[test]
    fn rule_3_accepts_the_band_below_the_water() {
        assert!(passes(3, 100.0));
        assert!(passes(3, 90.0));
        assert!(!passes(3, 100.1));
        assert!(!passes(3, 89.9));
    }

    #[test]
    fn rule_4_rejects_only_the_inner_band() {
        assert!(passes(4, 110.0));
        assert!(passes(4, 90.0));
        assert!(!passes(4, 100.0));
        assert!(!passes(4, 109.9));
    }

    #[test]
    fn rule_5_accepts_the_band_around_the_water() {
        assert!(passes(5, 90.0));
        assert!(passes(5, 110.0));
        assert!(!passes(5, 89.9));
        assert!(!passes(5, 110.1));
    }

    #[test]
    fn rules_from_6_on_apply_no_filter() {
        for rule in [6, 7, 8, u32::MAX] {
            assert!(passes(rule, -1.0e9), "rule {rule}");
            assert!(passes(rule, 1.0e9), "rule {rule}");
        }
    }

    #[test]
    fn without_water_every_rule_but_below_ones_sees_land_above_it() {
        let none = -f32::MAX;
        assert!(passes_water_filter(0, 0.0, none, D));
        assert!(!passes_water_filter(2, 0.0, none, D));
        assert!(!passes_water_filter(3, 0.0, none, D));
    }

    #[test]
    fn bilinear_density_interpolates_the_3x3_grid() {
        let mut grid = [0.0; 9];
        grid[4] = 1.0; // centre vertex only
        // At (1,1) the sample sits exactly on the centre.
        assert_eq!(bilinear_density(&grid, 1.0, 1.0), 1.0);
        // Halfway to the next vertex in x loses half.
        assert!((bilinear_density(&grid, 1.5, 1.0) - 0.5).abs() < 1e-6);
        // Halfway in both axes: a quarter.
        assert!((bilinear_density(&grid, 1.5, 1.5) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn tint_follows_the_documented_formula() {
        let point = SurfacePoint {
            height: 0.0,
            normal: Vec3::Y,
            colour: [0.5, 0.5, 0.5],
        };
        // luma = 0.5 * (0.31 + 0.37 + 0.32) = 0.5
        // no colour range -> luma unchanged regardless of the variation draw
        assert!((tint_of(&point, 0.0, 0.9) - 0.5).abs() < 1e-6);
        // cr 0.2, draw 0.9: 0.5 * (0.8 + 0.1 * 0.4) = 0.42
        assert!((tint_of(&point, 0.2, 0.9) - 0.42).abs() < 1e-6);
        // cr 0.2, draw 0.1: 0.5 * (0.8 - 0.04) = 0.38
        assert!((tint_of(&point, 0.2, 0.1) - 0.38).abs() < 1e-6);
    }

    #[test]
    fn tint_is_capped_below_one_and_floored_at_zero() {
        let white = SurfacePoint {
            height: 0.0,
            normal: Vec3::Y,
            colour: [1.0; 3],
        };
        // luma 1.0 with cr 0 -> exactly 1.0 -> 0.99
        assert_eq!(tint_of(&white, 0.0, 0.5), 0.99);
        // cr 3: factor (1-3) + 1.5*(draw-0.5) is negative for a low draw
        assert_eq!(tint_of(&white, 3.0, 0.0), 0.0);
    }
}
