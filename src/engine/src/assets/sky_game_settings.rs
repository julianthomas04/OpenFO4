//! The game settings the sky and the time-of-day lighting read.

use anyhow::Context;
use glam::Vec3;

use crate::assets::db::DbClient;
use crate::rendering::lighting::time_of_day::{SkyGameConfig, SkyGameSettings};

const SUN_X_EXTREME: &str = "fSunXExtreme";
const SUN_Y_EXTREME: &str = "fSunYExtreme";
const STARS_ROTATE_DAYS: &str = "fStarsRotateDays";
const STARS_ROTATE_X_AXIS: &str = "fStarsRotateXAxis";
const STARS_ROTATE_Y_AXIS: &str = "fStarsRotateYAxis";
const STARS_ROTATE_Z_AXIS: &str = "fStarsRotateZAxis";
const MASSER_ANGLE_FADE_START: &str = "fMasserAngleFadeStart";
const MASSER_ANGLE_FADE_END: &str = "fMasserAngleFadeEnd";
const SUN_ALPHA_TRANS_TIME: &str = "fSunAlphaTransTime";
const MASSER_SIZE: &str = "iMasserSize";
const SECUNDA_SIZE: &str = "iSecundaSize";

/// Boot-time entry point: a failure is logged and the exterior keeps its static sun, as it does
/// when the weather does not load.
pub fn load_sky_game_config_or_report(db_path: &str) -> Option<SkyGameConfig> {
    report(DbClient::new(db_path).and_then(|db| load_sky_game_config(&db)))
}

fn report(result: anyhow::Result<SkyGameConfig>) -> Option<SkyGameConfig> {
    match result {
        Ok(config) => Some(config),
        Err(error) => {
            tracing::error!(
                "sky game settings not loaded: {error:#}; the exterior keeps its static sun"
            );
            None
        }
    }
}

/// A database error propagates. A setting the database lacks is an error naming it, as is a
/// set of values the sky maths cannot work with.
pub fn load_sky_game_config(db: &DbClient) -> anyhow::Result<SkyGameConfig> {
    sky_game_config_from(|editor_id| db.get_game_setting_number(editor_id))
}

fn sky_game_config_from(
    lookup: impl Fn(&str) -> anyhow::Result<Option<f32>>,
) -> anyhow::Result<SkyGameConfig> {
    let setting = |editor_id: &str| -> anyhow::Result<f32> {
        lookup(editor_id)?
            .with_context(|| format!("game setting {editor_id} is missing from the database"))
    };
    let settings = SkyGameSettings::new(
        setting(SUN_X_EXTREME)?,
        setting(SUN_Y_EXTREME)?,
        setting(STARS_ROTATE_DAYS)?,
        Vec3::new(
            setting(STARS_ROTATE_X_AXIS)?,
            setting(STARS_ROTATE_Y_AXIS)?,
            setting(STARS_ROTATE_Z_AXIS)?,
        ),
        setting(MASSER_ANGLE_FADE_START)?,
        setting(MASSER_ANGLE_FADE_END)?,
    )
    .context("the game's sky settings are unusable")?;
    SkyGameConfig::new(
        settings,
        setting(SUN_ALPHA_TRANS_TIME)?,
        setting(MASSER_SIZE)?,
        setting(SECUNDA_SIZE)?,
    )
    .context("the game's sky settings are unusable")
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "values are copied through unchanged; no arithmetic is involved"
)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn vanilla() -> HashMap<&'static str, f32> {
        HashMap::from([
            (SUN_X_EXTREME, 600.0),
            (SUN_Y_EXTREME, -325.0),
            (STARS_ROTATE_DAYS, 1.0027),
            (STARS_ROTATE_X_AXIS, -0.09),
            (STARS_ROTATE_Y_AXIS, 0.35),
            (STARS_ROTATE_Z_AXIS, 1.0),
            (MASSER_ANGLE_FADE_START, 5.0),
            (MASSER_ANGLE_FADE_END, 10.0),
            (SUN_ALPHA_TRANS_TIME, 0.15),
            (MASSER_SIZE, 90.0),
            (SECUNDA_SIZE, 75.0),
        ])
    }

    fn config_from(settings: &HashMap<&str, f32>) -> anyhow::Result<SkyGameConfig> {
        sky_game_config_from(|editor_id| Ok(settings.get(editor_id).copied()))
    }

    #[test]
    fn the_vanilla_settings_load_into_a_config() {
        let config = config_from(&vanilla()).unwrap();
        let settings = config.settings();
        assert_eq!(settings.sun_x_extreme(), 600.0);
        assert_eq!(settings.sun_y_extreme(), -325.0);
        assert_eq!(settings.stars_rotate_days(), 1.0027);
        assert_eq!(settings.masser_angle_fade(), (5.0, 10.0));
        assert_eq!(config.sun_alpha_trans_time(), 0.15);
        assert_eq!((config.masser_size(), config.secunda_size()), (90.0, 75.0));
    }

    #[test]
    fn a_failed_load_is_reported_and_yields_no_config() {
        let missing = config_from(&HashMap::new());
        assert!(report(missing).is_none());
        assert!(report(config_from(&vanilla())).is_some());
    }

    #[test]
    fn a_database_that_cannot_be_opened_yields_no_config() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing.db");
        assert!(load_sky_game_config_or_report(path.to_str().unwrap()).is_none());
    }

    #[test]
    fn every_missing_setting_is_an_error_naming_it() {
        for missing in vanilla().keys() {
            let mut settings = vanilla();
            settings.remove(missing);
            let message = format!("{:#}", config_from(&settings).unwrap_err());
            assert!(message.contains(missing), "{missing}: {message}");
        }
    }

    #[test]
    fn a_database_error_propagates_with_its_cause() {
        let error = sky_game_config_from(|_| anyhow::bail!("disk on fire")).unwrap_err();
        assert!(format!("{error:#}").contains("disk on fire"));
    }

    #[test]
    fn an_unusable_value_is_an_error_naming_the_problem() {
        let cases = [
            (STARS_ROTATE_DAYS, 0.0, "fStarsRotateDays"),
            (SUN_ALPHA_TRANS_TIME, -1.0, "fSunAlphaTransTime"),
            (MASSER_ANGLE_FADE_START, 20.0, "fade angles"),
            (SUN_X_EXTREME, f32::NAN, "fSunXExtreme"),
            (MASSER_SIZE, 0.0, "iMasserSize"),
        ];
        for (name, value, expected) in cases {
            let mut settings = vanilla();
            settings.insert(name, value);
            let message = format!("{:#}", config_from(&settings).unwrap_err());
            assert!(message.contains(expected), "{name}: {message}");
        }
    }
}
