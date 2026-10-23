//! Builds the world music from an installation: reads the catalog from the asset database and
//! seeds the generator from the clock. The only place where `WorldMusic` meets the database and
//! the system clock.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use super::world_music::{WorldMusic, WorldMusicError};
use crate::assets::db::DbClient;

pub fn load_world_music(db_path: &str, audio_dir: &Path) -> Result<WorldMusic, WorldMusicError> {
    let catalog = DbClient::new(db_path)
        .and_then(|db| db.get_music_catalog())
        .map_err(WorldMusicError::Catalog)?;
    WorldMusic::new(catalog, audio_dir, seed_from_clock())
}

/// A seed that differs between runs.
pub fn seed_from_clock() -> u64 {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|before_epoch| before_epoch.duration());
    since_epoch.as_secs() ^ u64::from(since_epoch.subsec_nanos()).rotate_left(32)
}
