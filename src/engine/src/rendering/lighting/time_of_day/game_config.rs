use super::sky::SkyGameSettings;

#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum SkyGameConfigError {
    #[error("fSunAlphaTransTime is {0}, it must be a finite number of hours of zero or more")]
    InvalidSunAlphaTransTime(f32),
    #[error("{setting} is {value}, it must be a finite number above zero")]
    InvalidMoonSize { setting: &'static str, value: f32 },
}

/// Every game setting the time of day and the night sky read, validated on construction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkyGameConfig {
    settings: SkyGameSettings,
    sun_alpha_trans_time: f32,
    masser_size: f32,
    secunda_size: f32,
}

impl SkyGameConfig {
    /// `sun_alpha_trans_time` is `fSunAlphaTransTime`; `masser_size` and `secunda_size` are
    /// `iMasserSize` and `iSecundaSize`.
    pub fn new(
        settings: SkyGameSettings,
        sun_alpha_trans_time: f32,
        masser_size: f32,
        secunda_size: f32,
    ) -> Result<Self, SkyGameConfigError> {
        if !sun_alpha_trans_time.is_finite() || sun_alpha_trans_time < 0.0 {
            return Err(SkyGameConfigError::InvalidSunAlphaTransTime(
                sun_alpha_trans_time,
            ));
        }
        for (setting, value) in [("iMasserSize", masser_size), ("iSecundaSize", secunda_size)] {
            if !value.is_finite() || value <= 0.0 {
                return Err(SkyGameConfigError::InvalidMoonSize { setting, value });
            }
        }
        Ok(Self {
            settings,
            sun_alpha_trans_time,
            masser_size,
            secunda_size,
        })
    }

    pub fn settings(&self) -> &SkyGameSettings {
        &self.settings
    }

    pub fn sun_alpha_trans_time(&self) -> f32 {
        self.sun_alpha_trans_time
    }

    pub fn masser_size(&self) -> f32 {
        self.masser_size
    }

    pub fn secunda_size(&self) -> f32 {
        self.secunda_size
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are copied through unchanged"
)]
mod tests {
    use super::super::test_support::vanilla_settings;
    use super::*;

    fn config(trans: f32, masser: f32, secunda: f32) -> Result<SkyGameConfig, SkyGameConfigError> {
        SkyGameConfig::new(vanilla_settings(), trans, masser, secunda)
    }

    #[test]
    fn valid_values_are_kept_next_to_the_settings() {
        let config = config(0.15, 90.0, 75.0).unwrap();
        assert_eq!(config.sun_alpha_trans_time(), 0.15);
        assert_eq!((config.masser_size(), config.secunda_size()), (90.0, 75.0));
        assert_eq!(*config.settings(), vanilla_settings());
    }

    #[test]
    fn a_zero_transition_time_is_allowed() {
        assert!(config(0.0, 90.0, 75.0).is_ok());
    }

    #[test]
    fn a_negative_or_non_finite_transition_time_is_rejected() {
        for bad in [-0.1, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(
                matches!(
                    config(bad, 90.0, 75.0),
                    Err(SkyGameConfigError::InvalidSunAlphaTransTime(_))
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_moon_size_that_is_not_positive_and_finite_is_rejected_naming_the_setting() {
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(matches!(
                config(0.15, bad, 75.0),
                Err(SkyGameConfigError::InvalidMoonSize {
                    setting: "iMasserSize",
                    ..
                })
            ));
            assert!(matches!(
                config(0.15, 90.0, bad),
                Err(SkyGameConfigError::InvalidMoonSize {
                    setting: "iSecundaSize",
                    ..
                })
            ));
        }
    }
}
