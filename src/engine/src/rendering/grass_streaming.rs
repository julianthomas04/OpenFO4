//! The streaming worker's side of procedural grass: loads the grass catalog
//! once and places the grass of each exterior cell from its landscape.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use tracing::{info, info_span, warn};

use crate::assets::db::DbClient;
use crate::assets::db::landscapes::Landscape;
use crate::assets::types::glb_key::glb_key;
use crate::assets::types::grass::CellGrass;
use crate::assets::types::grass_placement::{
    CellTerrainInput, GrassCatalog, GrassSettings, place_cell_grass,
};
use crate::assets::types::terrain::{HeightGrid, decode_layers};
use crate::water_world::height::CellWaterState;
use crate::water_world::worldspace_default_water_height;

/// The game's `-FLT_MAX` for "this cell has no water".
const NO_WATER: f32 = -f32::MAX;

pub struct GrassWorkerData {
    catalog: GrassCatalog,
    settings: GrassSettings,
    /// Worldspace form key -> default water height (game units), resolved on
    /// first use.
    default_water_heights: Mutex<HashMap<String, f32>>,
}

impl GrassWorkerData {
    /// Reads the grass records and pairs them with the `[Grass]` INI settings. A database
    /// without grass data yields a catalog that places nothing; the cause
    /// is logged.
    pub fn load(db: &DbClient, asset_root: &str, settings: GrassSettings) -> Self {
        let catalog = match (db.get_grasses(), db.get_ltex_grasses()) {
            (Ok(grasses), Ok(texture_lists)) => {
                if grasses.is_empty() && !texture_lists.is_empty() {
                    warn!(
                        "grass: {} landscape texture grass entries but no GRAS records; \
                         the grasses table is empty and needs reloading (esm-only --tables grasses)",
                        texture_lists.len()
                    );
                }
                GrassCatalog::build(
                    &grasses,
                    &texture_lists,
                    |grass| {
                        resolve_model(
                            asset_root,
                            grass.editor_id.as_deref(),
                            grass.model_path.as_deref(),
                        )
                    },
                    |entry| {
                        warn!(
                            "landscape texture {} lists unknown grass {}; ignoring it",
                            entry.ltex_form_key, entry.grass_form_key
                        );
                    },
                )
            }
            (grasses, texture_lists) => {
                for error in [grasses.err(), texture_lists.err()].into_iter().flatten() {
                    warn!("grass data unavailable (pre-grass DB?): {error:#}");
                }
                GrassCatalog::default()
            }
        };
        info!(
            "Grass: {settings:?}, placement {}",
            if catalog.is_empty() {
                "off (no grass data)"
            } else {
                "on"
            }
        );
        Self {
            catalog,
            settings,
            default_water_heights: Mutex::new(HashMap::new()),
        }
    }

    /// The grass of one full-detail exterior cell; empty when grass is off or
    /// the landscape cannot be decoded.
    pub fn place(
        &self,
        db: &DbClient,
        worldspace: &str,
        cell_form_key: &str,
        landscape: &Landscape,
    ) -> Vec<CellGrass> {
        if self.catalog.is_empty() {
            return Vec::new();
        }
        let (gx, gy) = (landscape.grid_x, landscape.grid_y);
        let span = info_span!("grass_place", gx, gy, blades = tracing::field::Empty).entered();

        let Some(layers) = landscape.layers.as_deref().and_then(decode_layers) else {
            return Vec::new();
        };
        let heights = match HeightGrid::decode(&landscape.vertex_height_map) {
            Ok(heights) => heights,
            Err(error) => {
                warn!("grass for cell ({gx}, {gy}): {error:#}");
                return Vec::new();
            }
        };
        let cell = CellTerrainInput {
            grid: (gx, gy),
            layers: &layers,
            heights: &heights,
            vertex_colours: landscape.vertex_colors.as_deref(),
            water_height: self.water_height(db, worldspace, cell_form_key),
        };
        let grass = place_cell_grass(&cell, &self.catalog, &self.settings);
        span.record(
            "blades",
            grass.iter().map(|g| g.blades.len()).sum::<usize>(),
        );
        grass
    }

    /// The effective water height of an exterior cell, in Bethesda units.
    fn water_height(&self, db: &DbClient, worldspace: &str, cell_form_key: &str) -> f32 {
        let water = match db.get_cell_water(cell_form_key) {
            Ok(water) => water,
            Err(error) => {
                crate::utils::warn_once("grass-cell-water", || {
                    format!("grass: cell water unreadable, placing as if dry: {error:#}")
                });
                return NO_WATER;
            }
        };
        CellWaterState::of_exterior(&water)
            .effective_height(self.default_water_height(db, worldspace))
            .unwrap_or(NO_WATER)
    }

    /// The worldspace's default water height; a failed lookup is reported once
    /// and not cached, so a later call retries.
    fn default_water_height(&self, db: &DbClient, worldspace: &str) -> f32 {
        if let Some(height) = self.cached_default_height(worldspace) {
            return height;
        }
        match worldspace_default_water_height(db, worldspace) {
            Ok(height) => {
                self.cache_default_height(worldspace, height);
                height
            }
            Err(error) => {
                crate::utils::warn_once(&format!("grass-default-water-{worldspace}"), || {
                    format!(
                        "grass: default water height of {worldspace} unavailable, using 0: {error:#}"
                    )
                });
                0.0
            }
        }
    }

    // The map only memoises database reads, so a poisoned lock still holds valid entries.
    fn cached_default_height(&self, worldspace: &str) -> Option<f32> {
        self.default_water_heights
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(worldspace)
            .copied()
    }

    fn cache_default_height(&self, worldspace: &str, height: f32) {
        self.default_water_heights
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(worldspace.to_owned(), height);
    }
}

/// The GLB asset key of a grass model (see [`glb_key`]), matching the
/// placed-object mesh keys; `None` with one warning when
/// the record has no model or the converted asset is missing.
fn resolve_model(
    asset_root: &str,
    editor_id: Option<&str>,
    model_path: Option<&str>,
) -> Option<String> {
    let name = editor_id.unwrap_or("?");
    let Some(model_path) = model_path.filter(|path| !path.is_empty()) else {
        warn!("grass {name} has no model; it grows nothing");
        return None;
    };
    let key = glb_key(model_path);
    if Path::new(&format!("{asset_root}/{key}")).is_file() {
        Some(key)
    } else {
        warn!("grass {name}: model {key} not found; it grows nothing");
        None
    }
}
