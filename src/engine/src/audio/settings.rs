use std::time::Duration;

use kira::Decibels;

use crate::assets::ini::{GameIni, IniError, IniKey};
use crate::assets::layout;

const PLAY_MUSIC: IniKey = IniKey::new("General", "bPlayMainMenuMusic");
const MUSIC_FILE: IniKey = IniKey::new("General", "sMainMenuMusic");
const FADE_TIME_MS: IniKey = IniKey::new("General", "uMainMenuMusicFadeTimeMS");
const ATTENUATION_MB: IniKey = IniKey::new("General", "uMainMenuMusicAttnmB");

/// No shipped INI sets these four keys; the game's built-in defaults apply (Fallout4.exe).
const DEFAULT_PLAY_MUSIC: bool = true;
const DEFAULT_MUSIC_FILE: &str = "\\Data\\Music\\Special\\MUS_MainTheme.xwm";
const DEFAULT_FADE_TIME_MS: u32 = 1500;
const DEFAULT_ATTENUATION_MB: u32 = 300;

const MILLIBELS_PER_DECIBEL: f32 = 100.0;

#[derive(Debug, Clone, PartialEq)]
pub struct MenuMusicSettings {
    pub enabled: bool,
    /// The music file as the INI spells it, an archive path such as `\Data\Music\...xwm`.
    pub file: String,
    pub fade: Duration,
    pub attenuation_millibels: u32,
}

impl MenuMusicSettings {
    pub fn from_ini(ini: &GameIni) -> Result<Self, IniError> {
        let fade_ms = if ini.defined_in(FADE_TIME_MS).is_some() {
            ini.require::<u32>(FADE_TIME_MS)?
        } else {
            DEFAULT_FADE_TIME_MS
        };
        Ok(Self {
            enabled: if ini.defined_in(PLAY_MUSIC).is_some() {
                ini.require_flag(PLAY_MUSIC)?
            } else {
                DEFAULT_PLAY_MUSIC
            },
            file: if ini.defined_in(MUSIC_FILE).is_some() {
                ini.require_valid::<String>(
                    MUSIC_FILE,
                    "a file path without empty, \".\" or \"..\" segments",
                    |path| layout::audio_key(path).is_ok(),
                )?
            } else {
                DEFAULT_MUSIC_FILE.to_string()
            },
            fade: Duration::from_millis(u64::from(fade_ms)),
            attenuation_millibels: if ini.defined_in(ATTENUATION_MB).is_some() {
                ini.require::<u32>(ATTENUATION_MB)?
            } else {
                DEFAULT_ATTENUATION_MB
            },
        })
    }

    /// The music plays this much quieter than the file's own level.
    pub fn attenuation(&self) -> Decibels {
        Decibels(-(self.attenuation_millibels as f32) / MILLIBELS_PER_DECIBEL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(text: &str) -> Result<MenuMusicSettings, IniError> {
        MenuMusicSettings::from_ini(&GameIni::from_texts([text]))
    }

    #[test]
    fn absent_keys_give_the_built_in_defaults() {
        let settings = settings("[General]\nsLanguage=EN\n").unwrap();
        assert!(settings.enabled);
        assert_eq!(settings.file, "\\Data\\Music\\Special\\MUS_MainTheme.xwm");
        assert_eq!(settings.fade, Duration::from_millis(1500));
        assert_eq!(settings.attenuation_millibels, 300);
    }

    #[test]
    fn present_keys_override_the_defaults() {
        let settings = settings(
            "[General]\nbPlayMainMenuMusic=0\nsMainMenuMusic=\\Data\\Music\\Other.xwm\n\
             uMainMenuMusicFadeTimeMS=250\nuMainMenuMusicAttnmB=600\n",
        )
        .unwrap();
        assert!(!settings.enabled);
        assert_eq!(settings.file, "\\Data\\Music\\Other.xwm");
        assert_eq!(settings.fade, Duration::from_millis(250));
        assert_eq!(settings.attenuation_millibels, 600);
    }

    #[test]
    fn attenuation_is_millibels_below_the_file_level() {
        let default = settings("").unwrap();
        assert!((default.attenuation().0 + 3.0).abs() < 1e-6);
        let none = settings("[General]\nuMainMenuMusicAttnmB=0\n").unwrap();
        assert_eq!(none.attenuation(), Decibels::IDENTITY);
    }

    #[test]
    fn an_invalid_present_value_names_its_key() {
        for (text, key) in [
            ("[General]\nbPlayMainMenuMusic=yes\n", "bPlayMainMenuMusic"),
            ("[General]\nsMainMenuMusic=\n", "sMainMenuMusic"),
            (
                "[General]\nsMainMenuMusic=\\Data\\..\\..\\a.xwm\n",
                "sMainMenuMusic",
            ),
            (
                "[General]\nuMainMenuMusicFadeTimeMS=-5\n",
                "uMainMenuMusicFadeTimeMS",
            ),
            (
                "[General]\nuMainMenuMusicAttnmB=loud\n",
                "uMainMenuMusicAttnmB",
            ),
        ] {
            let error = settings(text).unwrap_err();
            assert!(error.to_string().contains(key), "{text}: {error}");
        }
    }
}
