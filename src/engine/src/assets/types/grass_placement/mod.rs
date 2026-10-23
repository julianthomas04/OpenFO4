//! Procedural grass placement of one exterior cell. Pure CPU: explicit inputs
//! in, blades out.
//!
//! The acceptance test draws from an MT19937 stream. One stream shared by all
//! cells would make a cell's grass depend on the order cells load in, so every
//! cell gets its own generator seeded with [`ACCEPTANCE_SEED`]; every other
//! draw is taken in a fixed order. Blade positions keep `f32` precision and
//! are not rounded to `f16`.

pub mod basis;
mod mt19937;
mod scatter;
mod slots;
mod surface;
mod tables;

use std::collections::{BTreeMap, HashMap};

use crate::assets::db::grass::{GrassRow, LtexGrassRow};
use crate::assets::ini::{GameIni, IniError, IniKey};
use crate::assets::types::grass::{CellGrass, GrassBlade, GrassFlags};
use crate::assets::types::terrain::{
    ALPHA_GRID, CELL_SIZE, HeightGrid, LandscapeLayer, TERRAIN_QUAD_SIZE,
};

use mt19937::Mt19937;
use scatter::{PATCH_SIDE, ScatterContext, scatter_slot};
use slots::{QuadrantLayers, gather_slots};
pub use surface::{SurfacePoint, TerrainSurface};

/// The default MT19937 seed.
pub const ACCEPTANCE_SEED: u32 = 5489;

/// Vertices per quadrant side; patches are centred on the odd ones.
const QUADRANT_VERTS: usize = ALPHA_GRID;
const QUADRANT_SIDE: f32 = TERRAIN_QUAD_SIZE * (QUADRANT_VERTS - 1) as f32;
const QUADRANTS: u8 = 4;

const MIN_GRASS_SIZE: IniKey = IniKey::new("Grass", "iMinGrassSize");
/// No shipped or generated INI sets these `[Grass]` keys, so the engine has no INI value to read
/// for them and uses the built-in values below; an INI that does set one is reported as ignored.
const IGNORED_KEYS: [IniKey; 3] = [
    IniKey::new("Grass", "bDrawShaderGrass"),
    IniKey::new("Grass", "iMaxGrassTypesPerTexure"),
    IniKey::new("Grass", "fTexturePctThreshold"),
];
const MAX_TYPES_PER_TEXTURE: usize = 2;
const TEXTURE_PCT_THRESHOLD: f32 = 0.005;
/// Replacement for a threshold above 1.
const OVERSIZED_THRESHOLD: f32 = 0.9;

/// What the placement reads of the `[Grass]` settings: the minimum grass size from the INI
/// cascade and built-in values for the rest.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GrassSettings {
    pub min_grass_size: i32,
    /// A texture grows at most this many grass types plus one.
    pub max_types_per_texture: usize,
    pub texture_pct_threshold: f32,
}

impl GrassSettings {
    pub fn from_ini(ini: &GameIni) -> Result<Self, IniError> {
        for (key, file) in ignored_settings(ini) {
            tracing::warn!(
                "INI setting {key} in {file} is ignored: the engine uses a built-in value for it"
            );
        }
        Ok(Self::with_min_grass_size(ini.require(MIN_GRASS_SIZE)?))
    }

    fn with_min_grass_size(min_grass_size: i32) -> Self {
        Self {
            min_grass_size,
            max_types_per_texture: MAX_TYPES_PER_TEXTURE,
            texture_pct_threshold: TEXTURE_PCT_THRESHOLD,
        }
    }

    /// The coverage a vertex needs to grow grass: the INI value clamped to
    /// non-negative, 0.9 when above 1.
    pub fn coverage_threshold(&self) -> f32 {
        let threshold = self.texture_pct_threshold.max(0.0);
        if threshold > 1.0 {
            OVERSIZED_THRESHOLD
        } else {
            threshold
        }
    }
}

/// The ignored keys an INI sets, with the file whose value wins.
fn ignored_settings(ini: &GameIni) -> Vec<(&'static str, &str)> {
    IGNORED_KEYS
        .into_iter()
        .filter_map(|key| ini.defined_in(key).map(|file| (key.name, file)))
        .collect()
}

/// One grass type: the `GRAS` fields placement reads and the resolved model.
#[derive(Debug, Clone, PartialEq)]
pub struct GrassType {
    /// Asset key of the model's GLB; `None` when it cannot be shown. Such a
    /// type still occupies its slots (so the per-texture cap stays exact) but
    /// yields no blades.
    pub model_path: Option<String>,
    pub flags: GrassFlags,
    /// Percent, 0 to 100.
    pub density: u8,
    pub min_slope: u8,
    pub max_slope: u8,
    pub units_from_water: u16,
    pub units_from_water_type: u32,
    pub position_range: f32,
    pub height_range: f32,
    pub color_range: f32,
    pub wave_period: f32,
}

/// Every grass type and which landscape textures grow which, in `GNAM` order.
#[derive(Debug, Default)]
pub struct GrassCatalog {
    types: Vec<GrassType>,
    by_texture: HashMap<String, Vec<usize>>,
}

impl GrassCatalog {
    /// `resolve_model` maps a `GRAS` model path to its asset key, `None` when
    /// the asset is unavailable. List entries naming an unknown `GRAS` are
    /// reported through `on_unknown_grass` and dropped.
    pub fn build(
        grasses: &[GrassRow],
        texture_lists: &[LtexGrassRow],
        resolve_model: impl Fn(&GrassRow) -> Option<String>,
        mut on_unknown_grass: impl FnMut(&LtexGrassRow),
    ) -> Self {
        let mut index_of: HashMap<&str, usize> = HashMap::new();
        let mut types = Vec::with_capacity(grasses.len());
        for row in grasses {
            index_of.insert(row.form_key.as_str(), types.len());
            types.push(GrassType {
                model_path: resolve_model(row),
                flags: GrassFlags(row.flags),
                density: row.density,
                min_slope: row.min_slope,
                max_slope: row.max_slope,
                units_from_water: row.units_from_water,
                units_from_water_type: row.units_from_water_type,
                position_range: row.position_range,
                height_range: row.height_range,
                color_range: row.color_range,
                wave_period: row.wave_period,
            });
        }

        let mut ordered: Vec<&LtexGrassRow> = texture_lists.iter().collect();
        ordered.sort_by(|a, b| (&a.ltex_form_key, a.ord).cmp(&(&b.ltex_form_key, b.ord)));
        let mut by_texture: HashMap<String, Vec<usize>> = HashMap::new();
        for entry in ordered {
            match index_of.get(entry.grass_form_key.as_str()) {
                Some(&index) => by_texture
                    .entry(entry.ltex_form_key.clone())
                    .or_default()
                    .push(index),
                None => on_unknown_grass(entry),
            }
        }
        Self { types, by_texture }
    }

    pub fn is_empty(&self) -> bool {
        self.by_texture.is_empty()
    }

    pub fn get(&self, index: usize) -> &GrassType {
        &self.types[index]
    }

    /// The grass types of a landscape texture, in list order.
    fn types_of(&self, texture_key: &str) -> &[usize] {
        self.by_texture
            .get(texture_key)
            .map_or(&[], std::vec::Vec::as_slice)
    }
}

/// The terrain facts of one exterior cell.
pub struct CellTerrainInput<'a> {
    /// Landscape cell grid coordinates.
    pub grid: (i32, i32),
    pub layers: &'a [LandscapeLayer],
    pub heights: &'a HeightGrid,
    /// `landscapes.vertex_colors`, if the land has any.
    pub vertex_colours: Option<&'a [u8]>,
    /// Effective water height in Bethesda units; `-f32::MAX` without water.
    pub water_height: f32,
}

/// The grass of one exterior cell, one entry per grass type that grew at
/// least one blade, ordered by catalog index.
pub fn place_cell_grass(
    cell: &CellTerrainInput,
    catalog: &GrassCatalog,
    settings: &GrassSettings,
) -> Vec<CellGrass> {
    let surface = TerrainSurface::new(cell.heights, cell.vertex_colours);
    let mut acceptance = Mt19937::new(ACCEPTANCE_SEED);
    let cell_origin = (
        cell.grid.0 as f32 * CELL_SIZE,
        cell.grid.1 as f32 * CELL_SIZE,
    );
    let mut context = ScatterContext {
        cell: cell.grid,
        cell_origin,
        surface: &surface,
        water_height: cell.water_height,
        settings,
        acceptance: &mut acceptance,
    };

    let mut blades_by_type: BTreeMap<usize, Vec<GrassBlade>> = BTreeMap::new();
    for quadrant in 0..QUADRANTS {
        let layers = QuadrantLayers::of(cell.layers, quadrant);
        let quadrant_origin = (
            cell_origin.0 + f32::from(quadrant & 1) * QUADRANT_SIDE,
            cell_origin.1 + f32::from(quadrant >> 1) * QUADRANT_SIDE,
        );
        for row in (1..QUADRANT_VERTS - 1).step_by(2) {
            for col in (1..QUADRANT_VERTS - 1).step_by(2) {
                let patch_origin = (
                    quadrant_origin.0 + col as f32 * TERRAIN_QUAD_SIZE - PATCH_SIDE * 0.5,
                    quadrant_origin.1 + row as f32 * TERRAIN_QUAD_SIZE - PATCH_SIDE * 0.5,
                );
                for slot in gather_slots(&layers, catalog, settings, col, row) {
                    let grass = catalog.get(slot.grass);
                    if grass.model_path.is_none() {
                        continue;
                    }
                    scatter_slot(
                        &mut context,
                        &slot,
                        grass,
                        patch_origin,
                        blades_by_type.entry(slot.grass).or_default(),
                    );
                }
            }
        }
    }

    blades_by_type
        .into_iter()
        .filter(|(_, blades)| !blades.is_empty())
        .filter_map(|(index, blades)| {
            let grass = catalog.get(index);
            Some(CellGrass {
                model_path: grass.model_path.clone()?,
                flags: grass.flags,
                wave_period: grass.wave_period,
                blades,
            })
        })
        .collect()
}

#[cfg(test)]
mod test_support {
    use super::*;
    use crate::assets::layout::mesh_key;

    /// The minimum grass size the placement tests are written against.
    const MIN_GRASS_SIZE: i32 = 20;

    pub fn grass_settings() -> GrassSettings {
        GrassSettings::with_min_grass_size(MIN_GRASS_SIZE)
    }

    /// Heights blob: offset, then 33 x 33 deltas in rows of constant y.
    pub fn height_blob(offset: f32, delta_at: impl Fn(usize, usize) -> i8) -> Vec<u8> {
        let mut bytes = offset.to_le_bytes().to_vec();
        for y in 0..33 {
            for x in 0..33 {
                bytes.push(delta_at(x, y) as u8);
            }
        }
        bytes
    }

    #[allow(
        clippy::large_types_passed_by_value,
        reason = "a test helper taking the layer field's own type"
    )]
    pub fn layer(
        texture: &str,
        quadrant: u8,
        is_base: bool,
        alpha_grid: Option<crate::assets::types::terrain::AlphaGrid>,
    ) -> LandscapeLayer {
        LandscapeLayer {
            texture_key: Some(texture.to_owned()),
            quadrant,
            is_base,
            alpha_grid,
            coverage: 0.0,
        }
    }

    pub fn grass_row(form_key: &str, density: u8) -> GrassRow {
        GrassRow {
            form_key: form_key.to_owned(),
            editor_id: None,
            model_path: Some(format!("landscape\\grass\\{form_key}.nif")),
            flags: 0,
            density,
            min_slope: 0,
            max_slope: 90,
            units_from_water: 0,
            units_from_water_type: 6,
            position_range: 32.0,
            height_range: 0.0,
            color_range: 0.0,
            wave_period: 10.0,
        }
    }

    /// A catalog from `(texture, [(grass, density)])` lists; grass types are
    /// numbered in order of first appearance.
    pub fn catalog_of(textures: &[(&str, &[(&str, u8)])]) -> GrassCatalog {
        let mut rows: Vec<GrassRow> = Vec::new();
        let mut lists = Vec::new();
        for (texture, grasses) in textures {
            for (ord, (name, density)) in grasses.iter().enumerate() {
                if !rows.iter().any(|row| row.form_key == *name) {
                    rows.push(grass_row(name, *density));
                }
                lists.push(LtexGrassRow {
                    ltex_form_key: (*texture).to_owned(),
                    grass_form_key: (*name).to_owned(),
                    ord: ord as u32,
                });
            }
        }
        catalog_from(&rows, &lists)
    }

    pub fn catalog_from(rows: &[GrassRow], lists: &[LtexGrassRow]) -> GrassCatalog {
        GrassCatalog::build(
            rows,
            lists,
            |row| row.model_path.as_ref().map(|path| mesh_key(path)),
            |entry| panic!("unexpected unknown grass {}", entry.grass_form_key),
        )
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals or exact table entries"
)]
mod tests {
    use glam::Vec3;

    use super::test_support::{
        catalog_from, catalog_of, grass_row, grass_settings, height_blob, layer,
    };
    use super::*;

    const GRID: (i32, i32) = (2, -3);
    /// 4 quadrants x 8 x 8 patches.
    const PATCHES: usize = 256;

    struct Terrain {
        heights: HeightGrid,
        layers: Vec<LandscapeLayer>,
        colours: Option<Vec<u8>>,
        water_height: f32,
    }

    impl Terrain {
        /// Flat ground at height 0 with `texture` as the base of all four quadrants.
        fn flat(texture: &str) -> Self {
            Self::sloped(texture, |_, _| 0)
        }

        fn sloped(texture: &str, delta_at: impl Fn(usize, usize) -> i8) -> Self {
            Self {
                heights: HeightGrid::decode(&height_blob(0.0, delta_at)).unwrap(),
                layers: (0..4).map(|q| layer(texture, q, true, None)).collect(),
                colours: None,
                water_height: -f32::MAX,
            }
        }

        fn place(&self, catalog: &GrassCatalog) -> Vec<CellGrass> {
            place_cell_grass(
                &CellTerrainInput {
                    grid: GRID,
                    layers: &self.layers,
                    heights: &self.heights,
                    vertex_colours: self.colours.as_deref(),
                    water_height: self.water_height,
                },
                catalog,
                &grass_settings(),
            )
        }
    }

    fn blade_count(grass: &[CellGrass]) -> usize {
        grass.iter().map(|g| g.blades.len()).sum()
    }

    fn one_type(adjust: impl FnOnce(&mut GrassRow)) -> GrassCatalog {
        let mut row = grass_row("g", 100);
        adjust(&mut row);
        catalog_from(
            &[row],
            &[LtexGrassRow {
                ltex_form_key: "L".into(),
                grass_form_key: "g".into(),
                ord: 0,
            }],
        )
    }

    #[test]
    fn full_density_fills_every_lattice_point_of_every_patch() {
        // position range 32 -> 256/32 = 8 per side; 8 x 8 points x 256 patches.
        let grass = Terrain::flat("L").place(&one_type(|_| {}));
        assert_eq!(grass.len(), 1);
        assert_eq!(blade_count(&grass), PATCHES * 64);
    }

    #[test]
    fn the_lattice_follows_the_position_range_and_minimum_size() {
        let place = |range: f32| {
            blade_count(&Terrain::flat("L").place(&one_type(|g| g.position_range = range)))
        };
        // 256/100 = 2 per side.
        assert_eq!(place(100.0), PATCHES * 4);
        // 256/10 = 25 but the minimum size 20 caps it at 12.
        assert_eq!(place(10.0), PATCHES * 144);
        assert_eq!(place(0.0), 0);
    }

    #[test]
    fn half_density_accepts_about_half_the_candidates() {
        let all = PATCHES * 64;
        let grass = Terrain::flat("L").place(&one_type(|g| g.density = 50));
        let accepted = blade_count(&grass);
        assert!(
            accepted > all * 45 / 100 && accepted < all * 55 / 100,
            "{accepted} of {all}"
        );
    }

    #[test]
    fn zero_density_grows_nothing() {
        assert!(
            Terrain::flat("L")
                .place(&one_type(|g| g.density = 0))
                .is_empty()
        );
    }

    #[test]
    fn blades_stay_inside_their_cell_on_the_ground() {
        let terrain = Terrain {
            heights: HeightGrid::decode(&height_blob(3.0, |_, _| 0)).unwrap(),
            ..Terrain::flat("L")
        };
        let grass = terrain.place(&one_type(|_| {}));
        let (x0, z0) = (GRID.0 as f32 * 409.6, -(GRID.1 as f32) * 409.6);
        for blade in &grass[0].blades {
            let [x, y, z] = blade.position;
            assert!(x >= x0 - 1e-3 && x <= x0 + 409.6 + 1e-3, "x {x}");
            // Cell y grows towards -z in engine space.
            assert!(z <= z0 + 1e-3 && z >= z0 - 409.6 - 1e-3, "z {z}");
            assert!((y - 2.4).abs() < 1e-4, "height 3 * 8 * 0.1 = 2.4, got {y}");
        }
    }

    #[test]
    fn blade_variation_stays_in_its_documented_ranges() {
        let grass = Terrain::flat("L").place(&one_type(|g| g.height_range = 0.3));
        let blades = &grass[0].blades;
        // Without fit-to-slope the frame is a plain yaw: right = (c, 0, -s) in
        // engine space with c in [-1, 1) and s >= 0.
        assert!(blades.iter().all(|b| (-1.0..1.0).contains(&b.right[0])));
        assert!(
            blades
                .iter()
                .all(|b| b.right[1] == 0.0 && b.right[2] <= 0.0)
        );
        assert!(blades.iter().all(|b| {
            let [x, _, z] = b.right;
            (x * x + z * z - 1.0).abs() < 1e-5
        }));
        assert!(blades.iter().all(|b| b.height_k.abs() <= 0.3));
        assert!(
            blades.iter().any(|b| b.height_k > 0.2) && blades.iter().any(|b| b.height_k < -0.2)
        );
        assert!(blades.iter().all(|b| b.up == [0.0, 1.0, 0.0]));
    }

    #[test]
    fn tint_is_the_vertex_colour_luma_when_the_colour_range_is_zero() {
        let mut terrain = Terrain::flat("L");
        terrain.colours = Some([255, 0, 0].repeat(33 * 33));
        let grass = terrain.place(&one_type(|_| {}));
        // 1.0 * 0.31 + 0 + 0
        assert!(grass[0].blades.iter().all(|b| (b.tint - 0.31).abs() < 1e-5));
    }

    #[test]
    fn colour_range_spreads_the_tint_around_the_luma() {
        let mut terrain = Terrain::flat("L");
        terrain.colours = Some([100, 100, 100].repeat(33 * 33));
        let grass = terrain.place(&one_type(|g| g.color_range = 0.5));
        let luma = 100.0 / 255.0;
        // luma * (0.5 + 0.25 * (r - 0.5)), r in [0, 1): between 0.375 and 0.625 of luma.
        for blade in &grass[0].blades {
            assert!(blade.tint >= luma * 0.375 - 1e-5 && blade.tint <= luma * 0.625 + 1e-5);
        }
        let spread = grass[0]
            .blades
            .iter()
            .map(|b| b.tint)
            .fold((f32::MAX, f32::MIN), |(lo, hi), t| (lo.min(t), hi.max(t)));
        assert!(spread.1 - spread.0 > 0.05, "{spread:?}");
    }

    /// Rises 128 units (one cell side's 1/32) per vertex along x: a 45 degree slope.
    fn forty_five_degrees() -> Terrain {
        Terrain::sloped("L", |x, _| if x == 0 { 0 } else { 16 })
    }

    #[test]
    fn slope_limits_use_the_quantised_cosine_table() {
        let blades = |min: u8, max: u8| {
            blade_count(&forty_five_degrees().place(&one_type(|g| {
                g.min_slope = min;
                g.max_slope = max;
            })))
        };
        let all = PATCHES * 64;
        // 45 degrees quantises to table index 63, cos 0.7157 > the true 0.7071.
        assert_eq!(blades(0, 46), all, "max 46 allows 45 degrees");
        assert_eq!(blades(0, 45), 0, "max 45 rejects it through the table");
        assert_eq!(blades(45, 90), all, "min 45 allows it");
        assert_eq!(blades(46, 90), 0, "min 46 rejects it");
    }

    #[test]
    fn flat_ground_is_inside_the_default_slope_window() {
        assert_eq!(
            blade_count(&Terrain::flat("L").place(&one_type(|g| {
                g.min_slope = 0;
                g.max_slope = 1;
            }))),
            PATCHES * 64
        );
    }

    #[test]
    fn fit_to_slope_blades_stand_on_the_terrain_normal() {
        let grass = forty_five_degrees().place(&one_type(|g| g.flags = GrassFlags::FIT_TO_SLOPE));
        let slope = std::f32::consts::FRAC_1_SQRT_2;
        for blade in &grass[0].blades {
            assert!((blade.up[0] + slope).abs() < 1e-3, "{:?}", blade.up);
            assert!((blade.up[1] - slope).abs() < 1e-3, "{:?}", blade.up);
            let right = Vec3::from(blade.right);
            assert!(right.dot(Vec3::from(blade.up)).abs() < 1e-4);
            assert!((right.length() - 1.0).abs() < 1e-4);
        }
        let upright = forty_five_degrees().place(&one_type(|_| {}));
        assert!(upright[0].blades.iter().all(|b| b.up == [0.0, 1.0, 0.0]));
    }

    #[test]
    fn water_rules_apply_to_the_terrain_height() {
        let mut terrain = Terrain::flat("L");
        // Terrain at 0, water at 1000: rule 0 wants z >= W - D (1000).
        terrain.water_height = 1000.0;
        let rule = |kind: u32, distance: u16| {
            blade_count(&terrain.place(&one_type(|g| {
                g.units_from_water_type = kind;
                g.units_from_water = distance;
            })))
        };
        let all = PATCHES * 64;
        assert_eq!(rule(0, 0), 0);
        assert_eq!(rule(0, 1000), all, "z >= 1000 - 1000");
        assert_eq!(rule(2, 0), all, "below the water");
        assert_eq!(rule(1, 50), 0, "not above the water");
        assert_eq!(rule(7, 0), all, "rules from 6 on are off");
        assert_eq!(rule(3, 1000), all, "within 1000 below the water");
        assert_eq!(rule(3, 999), 0, "not within 999 below the water");
        assert_eq!(
            rule(4, 500),
            all,
            "outside the band of 500 around the water"
        );
        assert_eq!(rule(4, 1500), 0, "inside the band of 1500 around the water");
        assert_eq!(
            rule(5, 1000),
            all,
            "inside the band of 1000 around the water"
        );
        assert_eq!(rule(5, 999), 0, "outside the band of 999 around the water");
    }

    #[test]
    fn a_cell_without_layers_grows_nothing() {
        let terrain = Terrain {
            layers: Vec::new(),
            ..Terrain::flat("L")
        };
        assert!(terrain.place(&one_type(|_| {})).is_empty());
    }

    #[test]
    fn overlay_coverage_takes_grass_from_the_base_of_its_quadrant_only() {
        let mut terrain = Terrain::flat("L");
        terrain.layers.push(layer(
            "bare",
            0,
            false,
            Some(
                [[1.0; crate::assets::types::terrain::ALPHA_GRID];
                    crate::assets::types::terrain::ALPHA_GRID],
            ),
        ));
        let grass = terrain.place(&one_type(|_| {}));
        assert_eq!(blade_count(&grass), PATCHES * 3 / 4 * 64);
    }

    #[test]
    fn overlay_grass_grows_where_the_overlay_is_painted() {
        let mut terrain = Terrain::flat("none");
        terrain.layers.push(layer(
            "L",
            3,
            false,
            Some(
                [[1.0; crate::assets::types::terrain::ALPHA_GRID];
                    crate::assets::types::terrain::ALPHA_GRID],
            ),
        ));
        let grass = terrain.place(&one_type(|_| {}));
        assert_eq!(blade_count(&grass), PATCHES / 4 * 64);
    }

    #[test]
    fn at_most_max_plus_one_types_grow_per_texture() {
        let catalog = catalog_of(&[(
            "L",
            &[("a", 50), ("b", 50), ("c", 50), ("d", 50), ("e", 50)],
        )]);
        let grass = Terrain::flat("L").place(&catalog);
        let models: Vec<_> = grass.iter().map(|g| g.model_path.as_str()).collect();
        assert_eq!(
            models,
            [
                "meshes\\landscape\\grass\\a.nif",
                "meshes\\landscape\\grass\\b.nif",
                "meshes\\landscape\\grass\\c.nif"
            ]
        );
    }

    #[test]
    fn a_type_without_a_model_grows_nothing_but_still_counts_towards_the_cap() {
        let rows = ["a", "b", "c", "d"].map(|name| grass_row(name, 50));
        let lists: Vec<LtexGrassRow> = rows
            .iter()
            .enumerate()
            .map(|(ord, row)| LtexGrassRow {
                ltex_form_key: "L".into(),
                grass_form_key: row.form_key.clone(),
                ord: ord as u32,
            })
            .collect();
        let catalog = GrassCatalog::build(
            &rows,
            &lists,
            |row| (row.form_key != "a").then(|| row.form_key.clone()),
            |_| unreachable!(),
        );
        let grass = Terrain::flat("L").place(&catalog);
        // a (no model), b, c fit under the cap; d does not.
        assert_eq!(
            grass
                .iter()
                .map(|g| g.model_path.as_str())
                .collect::<Vec<_>>(),
            ["b", "c"]
        );
    }

    #[test]
    fn unknown_grass_in_a_texture_list_is_reported_and_skipped() {
        let mut reported = Vec::new();
        let catalog = GrassCatalog::build(
            &[grass_row("known", 50)],
            &[
                LtexGrassRow {
                    ltex_form_key: "L".into(),
                    grass_form_key: "ghost".into(),
                    ord: 0,
                },
                LtexGrassRow {
                    ltex_form_key: "L".into(),
                    grass_form_key: "known".into(),
                    ord: 1,
                },
            ],
            |_| Some("m".into()),
            |entry| reported.push(entry.grass_form_key.clone()),
        );
        assert_eq!(reported, ["ghost"]);
        assert_eq!(catalog.types_of("L"), [0]);
    }

    #[test]
    fn texture_lists_keep_gnam_order_whatever_the_row_order() {
        let rows = [grass_row("a", 1), grass_row("b", 1)];
        let catalog = GrassCatalog::build(
            &rows,
            &[
                LtexGrassRow {
                    ltex_form_key: "L".into(),
                    grass_form_key: "b".into(),
                    ord: 1,
                },
                LtexGrassRow {
                    ltex_form_key: "L".into(),
                    grass_form_key: "a".into(),
                    ord: 0,
                },
            ],
            |_| None,
            |_| unreachable!(),
        );
        assert_eq!(catalog.types_of("L"), [0, 1]);
    }

    #[test]
    fn the_minimum_grass_size_comes_from_the_ini_and_the_rest_is_built_in() {
        let ini = GameIni::from_texts(["[Grass]\niMinGrassSize=40\n"]);
        assert_eq!(
            GrassSettings::from_ini(&ini).unwrap(),
            GrassSettings {
                min_grass_size: 40,
                max_types_per_texture: 2,
                texture_pct_threshold: 0.005,
            }
        );
    }

    #[test]
    fn grass_keys_without_a_shipped_ini_value_keep_the_built_in_value_when_an_ini_sets_them() {
        let ini = GameIni::from_texts([
            "iMinGrassSize=40\niMaxGrassTypesPerTexure=0\nfTexturePctThreshold=0.5\nbDrawShaderGrass=0\n",
        ]);
        let settings = GrassSettings::from_ini(&ini).unwrap();
        assert_eq!(settings.max_types_per_texture, 2);
        assert!((settings.texture_pct_threshold - 0.005).abs() < f32::EPSILON);
    }

    #[test]
    fn an_ini_that_sets_a_built_in_grass_key_is_reported_with_its_file() {
        let ini = GameIni::from_texts([
            "iMinGrassSize=40\nbDrawShaderGrass=0\n",
            "fTexturePctThreshold=0.5\n",
        ]);
        assert_eq!(
            ignored_settings(&ini),
            [
                ("bDrawShaderGrass", "text 1"),
                ("fTexturePctThreshold", "text 2")
            ]
        );
        let clean = GameIni::from_texts(["iMinGrassSize=40\n"]);
        assert_eq!(ignored_settings(&clean), []);
    }

    #[test]
    fn a_missing_or_unparsable_minimum_grass_size_is_an_error() {
        let missing = GrassSettings::from_ini(&GameIni::from_texts([""])).unwrap_err();
        assert!(matches!(
            missing,
            IniError::MissingKey {
                key: "iMinGrassSize",
                ..
            }
        ));
        let unparsable = GrassSettings::from_ini(&GameIni::from_texts(["iMinGrassSize=big\n"]));
        assert!(matches!(
            unparsable.unwrap_err(),
            IniError::Unparsable {
                key: "iMinGrassSize",
                ..
            }
        ));
    }

    #[test]
    fn the_jitter_comes_from_the_random_table_starting_at_the_cell_coordinate_product() {
        // One lattice point per patch (range 256), full density: the first blade is
        // quadrant 0, patch (1,1): centre (128, 128) +- 128 from the cell's corner.
        // Table values (MT19937 seed 1): T0 0.41701782, T1 0.99717480, T2 0.72031730.
        let first_blade = |grid: (i32, i32)| {
            let terrain = Terrain::flat("L");
            let grass = place_cell_grass(
                &CellTerrainInput {
                    grid,
                    layers: &terrain.layers,
                    heights: &terrain.heights,
                    vertex_colours: None,
                    water_height: -f32::MAX,
                },
                &one_type(|g| g.position_range = 256.0),
                &grass_settings(),
            );
            grass[0].blades[0].position
        };
        // Cell (0, 0): counter 0 -> x from T0, y from T1.
        let [x, _, z] = first_blade((0, 0));
        assert!((x - (128.0 + (2.0 * 0.417_017_82 - 1.0) * 128.0) * 0.1).abs() < 1e-3);
        assert!((z + (128.0 + (2.0 * 0.997_174_8 - 1.0) * 128.0) * 0.1).abs() < 1e-3);
        // Cell (1, 1): counter 1 -> x from T1, y from T2.
        let [x, _, z] = first_blade((1, 1));
        assert!((x - (4096.0 + 128.0 + (2.0 * 0.997_174_8 - 1.0) * 128.0) * 0.1).abs() < 1e-3);
        assert!((z + (4096.0 + 128.0 + (2.0 * 0.720_317_3 - 1.0) * 128.0) * 0.1).abs() < 1e-3);
    }
}
