use crate::assets::ini::{GameIni, IniError, IniKey};

const MIP_SKIP: IniKey = IniKey::new("Display", "iTexMipMapSkip");
/// A larger `iTexMipMapSkip` is clamped to this.
const MAX_MIP_SKIP: u32 = 4;

const MAX_ANISOTROPY: IniKey = IniKey::new("Display", "iMaxAnisotropy");
/// A larger `iMaxAnisotropy` is clamped to this, the highest sampler anisotropy clamp.
const MAX_SAMPLER_ANISOTROPY: u16 = 16;

/// Texture quality from the game's INI cascade.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct TextureStreamingConfig {
    /// `iTexMipMapSkip`: top mips dropped from every texture.
    pub mip_skip: u32,
    /// `iMaxAnisotropy`: sampler anisotropy clamp, 1 (off) to 16. The INI may write 0 for off.
    pub max_anisotropy: u16,
}

impl TextureStreamingConfig {
    pub fn from_ini(ini: &GameIni) -> Result<Self, IniError> {
        let mip_skip = ini.require_capped(MIP_SKIP, MAX_MIP_SKIP)?;
        let max_anisotropy = ini
            .require_capped(MAX_ANISOTROPY, MAX_SAMPLER_ANISOTROPY)?
            .max(1);

        let config = Self {
            mip_skip,
            max_anisotropy,
        };
        tracing::info!(
            "texture streaming: skip {} top mips, {}x anisotropy",
            config.mip_skip,
            config.max_anisotropy,
        );
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::test_support::ini_with;

    #[test]
    fn both_settings_come_from_the_ini() {
        let config = TextureStreamingConfig::from_ini(&ini_with("")).unwrap();
        assert_eq!((config.mip_skip, config.max_anisotropy), (1, 8));
    }

    fn config(overrides: &str) -> Result<TextureStreamingConfig, IniError> {
        TextureStreamingConfig::from_ini(&ini_with(overrides))
    }

    #[test]
    fn mip_skip_accepts_zero_to_four_and_clamps_above() {
        for (value, expected) in [(0, 0), (1, 1), (2, 2), (3, 3), (4, 4), (5, 4), (9, 4)] {
            let config = config(&format!("iTexMipMapSkip={value}\n")).unwrap();
            assert_eq!(config.mip_skip, expected, "{value}");
        }
    }

    #[test]
    fn anisotropy_is_taken_as_written_from_one_to_sixteen() {
        for value in [1, 2, 4, 8, 12, 16] {
            let config = config(&format!("iMaxAnisotropy={value}\n")).unwrap();
            assert_eq!(config.max_anisotropy, value, "{value}");
        }
    }

    #[test]
    fn anisotropy_zero_means_off_and_one_is_the_sampler_spelling_of_it() {
        assert_eq!(config("iMaxAnisotropy=0\n").unwrap().max_anisotropy, 1);
    }

    #[test]
    fn anisotropy_above_the_sampler_limit_is_clamped() {
        for value in [17, 32, 4096] {
            let config = config(&format!("iMaxAnisotropy={value}\n")).unwrap();
            assert_eq!(config.max_anisotropy, 16, "{value}");
        }
    }

    #[test]
    fn negative_whole_numbers_are_errors_naming_the_key() {
        for (text, key) in [
            ("iTexMipMapSkip=-1\n", "iTexMipMapSkip"),
            ("iMaxAnisotropy=-1\n", "iMaxAnisotropy"),
        ] {
            let error = config(text).unwrap_err();
            assert!(error.to_string().contains(key), "{text}: {error}");
        }
    }

    #[test]
    fn unparsable_values_are_errors_naming_the_key() {
        for (text, key) in [
            ("iTexMipMapSkip=two\n", "iTexMipMapSkip"),
            ("iMaxAnisotropy=high\n", "iMaxAnisotropy"),
        ] {
            let error = TextureStreamingConfig::from_ini(&ini_with(text)).unwrap_err();
            assert!(error.to_string().contains(key), "{text}: {error}");
        }
    }

    #[test]
    fn a_later_ini_in_the_cascade_wins_over_an_earlier_one() {
        let dir = tempfile::tempdir().unwrap();
        let aniso = "iMaxAnisotropy=4\n";
        std::fs::write(
            dir.path().join("Fallout4_Default.ini"),
            format!("iTexMipMapSkip=1\n{aniso}"),
        )
        .unwrap();
        std::fs::write(dir.path().join("Fallout4Prefs.ini"), aniso).unwrap();
        std::fs::write(dir.path().join("Fallout4Custom.ini"), "iTexMipMapSkip=3\n").unwrap();
        let ini = GameIni::load(dir.path()).unwrap();
        assert_eq!(TextureStreamingConfig::from_ini(&ini).unwrap().mip_skip, 3);
    }
}
