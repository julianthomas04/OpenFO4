//! The startup database reads that need no GPU, run beside the GPU construction.

use std::collections::HashMap;

use super::{
    InteriorLightingCatalog, InteriorOption, MaterialBuoyancy, PlayerMovement, ResolvedTarget,
    State, WorldspaceOption, material_buoyancy_table, world_streamer,
};
use crate::assets::db::DbClient;
use crate::game_flow::{SpawnLocation, StartMode, spawn_locations::resolve_spawn_locations};
use crate::startup::StartupError;

pub(super) struct StartupDbReads {
    pub spawn_locations: Vec<SpawnLocation>,
    pub worldspaces: Vec<WorldspaceOption>,
    pub interior_lighting: Option<InteriorLightingCatalog>,
    pub interiors: Vec<InteriorOption>,
    pub teleports: HashMap<String, ResolvedTarget>,
    pub player_movement: PlayerMovement,
    pub buoyancy_table: Option<MaterialBuoyancy>,
}

impl StartupDbReads {
    /// Every read except the spawn locations degrades to an empty or default value on failure and
    /// reports it; the engine still runs.
    pub(super) fn load(db_path: &str, start_mode: StartMode) -> Result<Self, StartupError> {
        let spawn_locations = match start_mode {
            StartMode::Menu => {
                resolve_spawn_locations(&DbClient::new(db_path).map_err(StartupError::Database)?)?
            }
            StartMode::Direct => Vec::new(),
        };
        Ok(Self {
            spawn_locations,
            worldspaces: Self::load_worldspaces(db_path),
            interior_lighting: InteriorLightingCatalog::load_or_report(db_path),
            interiors: State::build_interiors(db_path),
            teleports: State::build_teleports(db_path),
            player_movement: Self::load_player_movement(db_path),
            buoyancy_table: material_buoyancy_table(db_path),
        })
    }

    /// Debug worldspace dropdown. Open and query failures are reported distinctly; either yields
    /// an empty dropdown and the engine still runs against `WORLDSPACE`.
    fn load_worldspaces(db_path: &str) -> Vec<WorldspaceOption> {
        match DbClient::new(db_path) {
            Ok(db) => match db.get_worldspaces() {
                Ok(rows) => rows
                    .into_iter()
                    .filter_map(|w| WorldspaceOption::from_worldspace(w, &db))
                    .collect(),
                Err(e) => {
                    tracing::warn!("worldspace dropdown query failed: {e}");
                    Vec::new()
                }
            },
            Err(e) => {
                tracing::warn!("worldspace dropdown DB open failed: {e}");
                Vec::new()
            }
        }
    }

    /// Player ground speeds come from the `Player_Default_MT` MOVT record (Beth u/s in the DB ->
    /// engine u/s via SCALE). A missing row or pre-movt-extraction DB falls back to the same
    /// vanilla values baked into Default.
    fn load_player_movement(db_path: &str) -> PlayerMovement {
        let movement = DbClient::new(db_path)
            .and_then(|db| db.get_movement_type("Player_Default_MT"))
            .map_or_else(
                |e| {
                    tracing::warn!("Player_Default_MT speeds not loaded ({e}); using defaults");
                    PlayerMovement::default()
                },
                |mt| PlayerMovement {
                    run_speed: mt.forward_run * world_streamer::SCALE,
                    sprint_speed: mt.forward_sprint * world_streamer::SCALE,
                },
            );
        tracing::info!(
            "player movement: run {:.2} u/s, sprint {:.2} u/s",
            movement.run_speed,
            movement.sprint_speed
        );
        movement
    }
}
