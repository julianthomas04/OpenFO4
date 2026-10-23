use crate::assets::ini::{GameIni, IniError, IniKey};

const WATER_REFLECTIONS: IniKey = IniKey::new("Water", "bUseWaterReflections");
const LOD_DRAW_RANGE: IniKey = IniKey::new("TerrainManager", "fBlockMaximumDistance");

/// The water quality settings from the game's INI cascade.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct WaterConfig {
    /// `bUseWaterReflections`: whether the distant (LOD) water reflects.
    pub reflections: bool,
    /// `fBlockMaximumDistance`: how far from the camera distant (LOD) water is drawn, in game
    /// units.
    pub lod_range_game_units: f32,
}

impl WaterConfig {
    pub fn from_ini(ini: &GameIni) -> Result<Self, IniError> {
        Ok(Self {
            reflections: ini.require_flag(WATER_REFLECTIONS)?,
            lod_range_game_units: ini.require_positive(LOD_DRAW_RANGE)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::test_support::ini_with;

    #[test]
    fn reflections_come_from_the_ini() {
        for (text, reflections) in [
            ("bUseWaterReflections=1\n", true),
            ("bUseWaterReflections=0\n", false),
        ] {
            let config = WaterConfig::from_ini(&ini_with(text)).unwrap();
            assert_eq!(config.reflections, reflections, "{text}");
        }
    }

    #[test]
    fn the_lod_range_comes_from_the_ini() {
        for range in [1.0, 90_000.0, 123_456.0] {
            let text = format!("fBlockMaximumDistance={range}\n");
            let config = WaterConfig::from_ini(&ini_with(&text)).unwrap();
            assert!((config.lod_range_game_units - range).abs() < 1e-3, "{text}");
        }
    }

    #[test]
    fn a_missing_or_invalid_reflections_setting_is_an_error_naming_it() {
        for text in ["", "bUseWaterReflections=2\n", "bUseWaterReflections=yes\n"] {
            let error = WaterConfig::from_ini(&GameIni::from_texts([text])).unwrap_err();
            assert!(
                error.to_string().contains("bUseWaterReflections"),
                "{text}: {error}"
            );
        }
    }

    #[test]
    fn a_missing_lod_range_is_a_fatal_error_without_a_fallback() {
        let ini = GameIni::from_texts(["bUseWaterReflections=1\n"]);
        let error = WaterConfig::from_ini(&ini).unwrap_err();
        assert!(
            matches!(&error, IniError::MissingKey { key, section, .. }
                if *key == "fBlockMaximumDistance" && *section == "TerrainManager"),
            "{error}"
        );
    }

    #[test]
    fn an_unusable_lod_range_is_an_error_naming_it() {
        for text in [
            "fBlockMaximumDistance=0\n",
            "fBlockMaximumDistance=-90000\n",
            "fBlockMaximumDistance=NaN\n",
            "fBlockMaximumDistance=inf\n",
            "fBlockMaximumDistance=far\n",
            "fBlockMaximumDistance=\n",
        ] {
            let error = WaterConfig::from_ini(&ini_with(text)).unwrap_err();
            assert!(
                error.to_string().contains("fBlockMaximumDistance"),
                "{text}: {error}"
            );
        }
    }
}
