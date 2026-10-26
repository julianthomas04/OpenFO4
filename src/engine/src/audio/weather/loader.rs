//! Builds the weather audio from an installation: reads the weather sound catalog from the asset
//! database and seeds the generator from the clock. The only place where `WeatherAudio` meets the
//! database and the system clock.

use std::path::Path;

use super::table::WeatherSoundTable;
use super::{LOG_TARGET, WeatherAudio, WeatherAudioError};
use crate::assets::db::DbClient;
use crate::audio::world_music_loader::seed_from_clock;

/// Reads every sound `table` can reach from the database at `db_path` and checks the converted
/// files below `audio_dir`.
pub fn load_weather_audio(
    db_path: &str,
    table: WeatherSoundTable,
    audio_dir: &Path,
) -> Result<WeatherAudio, WeatherAudioError> {
    let roots = table.descriptors().into_iter().map(str::to_string);
    let catalog = DbClient::new(db_path)
        .and_then(|db| db.get_weather_sound_catalog(roots))
        .map_err(WeatherAudioError::Catalog)?;
    WeatherAudio::new(catalog, table, audio_dir, seed_from_clock())
}

/// Boot-time entry point: a failure is logged and the game goes on without weather sounds.
pub fn load_or_report(
    db_path: &str,
    table: WeatherSoundTable,
    audio_dir: &Path,
) -> Option<WeatherAudio> {
    match load_weather_audio(db_path, table, audio_dir) {
        Ok(audio) => Some(audio),
        Err(error) => {
            tracing::error!(target: LOG_TARGET, "weather sounds are disabled: {error}");
            None
        }
    }
}
