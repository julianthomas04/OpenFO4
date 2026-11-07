//! Builds the weather audio from an installation: reads the weather sound catalog from the asset
//! database and seeds the generator from the clock. The only place where `WeatherAudio` meets the
//! database and the system clock.

use std::path::Path;

use super::table::WeatherSoundTable;
use super::{WeatherAudio, WeatherAudioError, report_disabled};
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
            report_disabled(&error);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;
    use crate::audio::test_support::SharedBuffer;
    use crate::audio::test_support::weather::table;

    const DISABLED: &str = "weather sounds are disabled";

    fn disabled_lines(log: &SharedBuffer) -> Vec<String> {
        log.drain()
            .lines()
            .filter(|line| line.contains(DISABLED))
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn a_database_that_cannot_be_opened_leaves_the_game_without_weather_sounds_and_says_why() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("absent.db");
        let log = SharedBuffer::default();
        let _guard = log.capture("openfo4=warn");

        let loaded = load_or_report(missing.to_str().unwrap(), table(), root.path());

        assert!(loaded.is_none());
        let lines = disabled_lines(&log);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].contains("cannot read the weather sound data"),
            "{lines:?}"
        );
    }

    #[test]
    fn a_database_without_the_sound_tables_leaves_the_game_without_weather_sounds_and_says_why() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("converted.db");
        Connection::open(&database)
            .unwrap()
            .execute_batch("CREATE TABLE unrelated (key TEXT);")
            .unwrap();
        let log = SharedBuffer::default();
        let _guard = log.capture("openfo4=warn");

        let loaded = load_or_report(database.to_str().unwrap(), table(), root.path());

        assert!(loaded.is_none());
        let lines = disabled_lines(&log);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].contains("cannot read the weather sound data"),
            "{lines:?}"
        );
    }
}
