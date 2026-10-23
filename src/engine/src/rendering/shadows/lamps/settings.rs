use crate::assets::ini::{GameIni, IniError, IniKey};
use crate::rendering::shadows::settings::{ShadowConfig, ShadowFilter};
use crate::rendering::world_streamer::SCALE;

/// Upper bound of the lamps shadowed at once; the panel override is clamped to it and a larger
/// `iMaxFocusShadows` is clamped to it.
pub const MAX_SHADOWED_LAMPS: u32 = 16;

const MAX_LAMPS: IniKey = IniKey::new("Display", "iMaxFocusShadows");
const FILTER_QUALITY: IniKey = IniKey::new("Display", "uiShadowFilter");
const HALVE_DISTANCE: IniKey = IniKey::new("Display", "fLocalShadowMapHalveEveryXUnit");
const FADE_START: IniKey = IniKey::new("Display", "fShadowLODMaxStartFade");
/// Fixed guess like the placed-light fade range: no INI key is known; calibrate.
const FADE_RANGE_GUESS_BETH: f32 = 500.0;

/// Lamp-shadow quality from the game's INI cascade; distances in engine units.
///
/// The cap reads `iMaxFocusShadows`: no shipped key is documented as the
/// number of shadowed placed lights, and community guides describe focus
/// shadows as high-resolution shadows of what the player looks at, so this
/// interpretation is uncertain.
#[derive(Debug, Copy, Clone)]
pub struct LampShadowConfig {
    pub max_lamps: u32,
    pub filter: ShadowFilter,
    /// A lamp's tile halves its resolution every this many units between the
    /// camera and the lamp's range sphere (`fLocalShadowMapHalveEveryXUnit`).
    pub halve_distance: f32,
    /// Receiver distance from the camera at which lamp shadows are gone.
    pub fade_end: f32,
    /// Side of the square shadow atlas (`iShadowMapResolution`).
    pub atlas_resolution: u32,
    pub bias_scale: f32,
}

impl LampShadowConfig {
    pub fn from_ini(ini: &GameIni, sun: &ShadowConfig) -> Result<Self, IniError> {
        let max_lamps = ini.require_capped(MAX_LAMPS, MAX_SHADOWED_LAMPS)?;
        let filter = ShadowFilter::from_ini(ini, FILTER_QUALITY)?;
        let halve_distance = ini.require_positive(HALVE_DISTANCE)?;
        let fade_start = ini.require_non_negative(FADE_START)?;

        let config = Self {
            max_lamps,
            filter,
            halve_distance: halve_distance * SCALE,
            fade_end: (fade_start + FADE_RANGE_GUESS_BETH) * SCALE,
            atlas_resolution: sun.resolution,
            bias_scale: sun.bias_scale,
        };
        tracing::info!(
            "lamp shadows: up to {max_lamps} lamps in a {}² atlas, {:?} filter, tiles halve every {halve_distance} BU, gone {} BU from the camera",
            config.atlas_resolution,
            config.filter,
            fade_start + FADE_RANGE_GUESS_BETH,
        );
        Ok(config)
    }
}

/// Live, non-persisted debug-panel controls. Biases are in texels of the
/// receiving tile at the receiver's distance from the lamp.
#[derive(Debug, Copy, Clone)]
pub struct LampShadowSettings {
    pub enabled: bool,
    /// Overrides the INI cap.
    pub max_lamps: u32,
    pub constant_bias: f32,
    pub slope_bias: f32,
    pub normal_offset: f32,
    pub freeze_selection: bool,
    /// Set by the panel, consumed by the renderer: re-render every tile.
    pub rerender_requested: bool,
    pub show_atlas: bool,
}

impl LampShadowSettings {
    pub fn new(config: &LampShadowConfig) -> Self {
        Self {
            enabled: true,
            max_lamps: config.max_lamps,
            constant_bias: 1.0,
            slope_bias: 1.5,
            normal_offset: 1.5,
            freeze_selection: false,
            rerender_requested: false,
            show_atlas: false,
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact sentinels and copied values"
)]
mod tests {
    use super::*;
    use crate::rendering::test_support::ini_with;

    fn sun() -> ShadowConfig {
        ShadowConfig::from_ini(&ini_with(
            "iShadowMapResolution=1024\nfShadowBiasScale=0.25\n",
        ))
        .unwrap()
    }

    fn lamps(overrides: &str) -> Result<LampShadowConfig, IniError> {
        LampShadowConfig::from_ini(&ini_with(overrides), &sun())
    }

    #[test]
    fn every_setting_comes_from_the_ini() {
        let config = lamps("").unwrap();
        assert_eq!(config.max_lamps, 6);
        assert_eq!(config.filter, ShadowFilter::SingleTap);
        assert!((config.halve_distance - 90.0).abs() < 1e-3);
        // (1200 + 500 guessed fade range) Bethesda units.
        assert!((config.fade_end - 170.0).abs() < 1e-3);
    }

    #[test]
    fn the_atlas_and_bias_follow_the_sun_shadow_config() {
        let config = lamps("").unwrap();
        assert_eq!(config.atlas_resolution, 1024);
        assert_eq!(config.bias_scale, 0.25);
    }

    #[test]
    fn the_lamp_cap_accepts_zero_up_to_the_maximum_and_clamps_above() {
        // 1, 2 and 4 are what the launcher's quality presets write.
        for (value, expected) in [(0, 0), (1, 1), (2, 2), (4, 4), (16, 16), (17, 16), (32, 16)] {
            let config = lamps(&format!("iMaxFocusShadows={value}\n")).unwrap();
            assert_eq!(config.max_lamps, expected, "{value}");
        }
    }

    #[test]
    fn a_negative_or_unparsable_lamp_cap_is_an_error_naming_the_key() {
        for value in ["-1", "many"] {
            let error = lamps(&format!("iMaxFocusShadows={value}\n")).unwrap_err();
            let message = error.to_string();
            assert!(message.contains("iMaxFocusShadows"), "{value}: {message}");
            assert!(
                message.contains("a whole number, 0 or more"),
                "{value}: {message}"
            );
        }
    }

    #[test]
    fn the_filter_quality_accepts_zero_to_the_maximum_and_clamps_above() {
        for (quality, expected) in [
            (0, ShadowFilter::SingleTap),
            (ShadowFilter::MAX_QUALITY, ShadowFilter::Grid3x3),
            (ShadowFilter::MAX_QUALITY + 1, ShadowFilter::Grid3x3),
        ] {
            let config = lamps(&format!("uiShadowFilter={quality}\n")).unwrap();
            assert_eq!(config.filter, expected, "{quality}");
        }
    }

    #[test]
    fn ini_distances_become_engine_units() {
        let config = lamps(
            "fLocalShadowMapHalveEveryXUnit=1000\nfShadowLODMaxStartFade=2000\nuiShadowFilter=2\n",
        )
        .unwrap();
        assert!((config.halve_distance - 100.0).abs() < 1e-3);
        assert!((config.fade_end - 250.0).abs() < 1e-3);
        assert_eq!(config.filter, ShadowFilter::Grid2x2);
    }

    #[test]
    fn invalid_ini_values_are_errors_naming_the_key() {
        for (overrides, key) in [
            (
                "fLocalShadowMapHalveEveryXUnit=0\n",
                "fLocalShadowMapHalveEveryXUnit",
            ),
            (
                "fLocalShadowMapHalveEveryXUnit=-5\n",
                "fLocalShadowMapHalveEveryXUnit",
            ),
            ("fShadowLODMaxStartFade=-1\n", "fShadowLODMaxStartFade"),
            ("uiShadowFilter=-1\n", "uiShadowFilter"),
            ("uiShadowFilter=high\n", "uiShadowFilter"),
        ] {
            let error = lamps(overrides).unwrap_err();
            assert!(error.to_string().contains(key), "{overrides}: {error}");
        }
    }

    #[test]
    fn the_panel_settings_start_from_the_ini_cap() {
        let config = lamps("iMaxFocusShadows=7\n").unwrap();
        let settings = LampShadowSettings::new(&config);
        assert_eq!(settings.max_lamps, 7);
        assert!(settings.enabled && !settings.freeze_selection && !settings.rerender_requested);
    }
}
