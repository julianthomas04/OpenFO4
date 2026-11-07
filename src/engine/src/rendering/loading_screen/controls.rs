//! How the game's INI settings shape the viewer's control of a loading screen's model.
//!
//! The game's own INIs leave these settings out, so each falls back to its documented hidden
//! default; a setting an INI does set overrides it.

use crate::assets::ini::{GameIni, IniError, IniKey};

const DELAY_BEFORE_AUTO_SPIN: IniKey =
    IniKey::new("Interface", "uLoadingModel_DelayBeforeAutoSpin");

const VANILLA_DELAY_BEFORE_AUTO_SPIN_MS: u32 = 1000;

const MILLISECONDS_PER_SECOND: f32 = 1000.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelControls {
    /// How long after a drag ends the model starts to spin by itself again.
    pub spin_resume_delay_seconds: f32,
}

impl ModelControls {
    pub const VANILLA: Self = Self::with_delay_ms(VANILLA_DELAY_BEFORE_AUTO_SPIN_MS);

    const fn with_delay_ms(milliseconds: u32) -> Self {
        Self {
            spin_resume_delay_seconds: milliseconds as f32 / MILLISECONDS_PER_SECOND,
        }
    }

    /// A setting no INI sets takes its [`Self::VANILLA`] value, with one note in the log.
    pub fn from_ini(ini: &GameIni) -> Result<Self, IniError> {
        let delay_ms: Option<u32> = ini.optional_valid(
            DELAY_BEFORE_AUTO_SPIN,
            "a whole number of milliseconds, 0 or more",
            |_| true,
        )?;
        let Some(delay_ms) = delay_ms else {
            tracing::info!(
                "{} is not set in any game INI; the vanilla default applies",
                DELAY_BEFORE_AUTO_SPIN.name
            );
            return Ok(Self::VANILLA);
        };
        Ok(Self::with_delay_ms(delay_ms))
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the expected values are exact for these inputs"
)]
mod tests {
    use super::*;

    fn ini(text: &str) -> GameIni {
        GameIni::from_texts([text])
    }

    #[test]
    fn without_a_setting_the_spin_resumes_a_second_after_a_drag() {
        let controls = ModelControls::from_ini(&ini("")).unwrap();
        assert_eq!(controls, ModelControls::VANILLA);
        assert_eq!(controls.spin_resume_delay_seconds, 1.0);
    }

    #[test]
    fn a_setting_in_an_ini_gives_the_delay_in_milliseconds() {
        let controls =
            ModelControls::from_ini(&ini("[Interface]\nuLoadingModel_DelayBeforeAutoSpin=250\n"))
                .unwrap();
        assert_eq!(controls.spin_resume_delay_seconds, 0.25);
    }

    #[test]
    fn a_setting_that_is_not_a_whole_number_is_an_error() {
        let error =
            ModelControls::from_ini(&ini("[Interface]\nuLoadingModel_DelayBeforeAutoSpin=-5\n"))
                .unwrap_err();
        assert!(matches!(error, IniError::Unparsable { .. }), "{error}");
    }
}
