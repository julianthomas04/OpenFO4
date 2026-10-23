use crate::assets::ini::{GameIni, IniError, IniKey};

const ENABLE: IniKey = IniKey::new("General", "bEnableMainMenuBink");

/// The video the game plays behind the main menu. No INI setting names it: the game keeps the
/// file name as a built-in value (Fallout4.exe).
pub const MAIN_MENU_VIDEO_SOURCE: &str = "MainMenuLoop.bk2";

/// No shipped INI sets the switch; the game's built-in default applies (Fallout4.exe).
const DEFAULT_ENABLED: bool = true;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuVideoSettings {
    pub enabled: bool,
}

impl MenuVideoSettings {
    pub fn from_ini(ini: &GameIni) -> Result<Self, IniError> {
        Ok(Self {
            enabled: if ini.defined_in(ENABLE).is_some() {
                ini.require_flag(ENABLE)?
            } else {
                DEFAULT_ENABLED
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled(ini_text: &str) -> Result<bool, IniError> {
        MenuVideoSettings::from_ini(&GameIni::from_texts([ini_text])).map(|s| s.enabled)
    }

    #[test]
    fn the_video_is_on_unless_an_ini_turns_it_off() {
        assert!(enabled("[General]\n").unwrap());
        assert!(enabled("[General]\nbEnableMainMenuBink=1\n").unwrap());
        assert!(!enabled("[General]\nbEnableMainMenuBink=0\n").unwrap());
    }

    #[test]
    fn a_malformed_switch_is_an_error() {
        assert!(enabled("[General]\nbEnableMainMenuBink=yes\n").is_err());
    }
}
