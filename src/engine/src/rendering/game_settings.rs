//! Every game INI setting the renderer and the streamer read, gathered and validated once
//! before the window opens, so a missing or invalid setting stops the engine at startup.

use std::path::Path;

use crate::assets::ini::{GameIni, IniError, IniKey};
use crate::assets::layout;
use crate::assets::types::grass_placement::GrassSettings;
use crate::assets::world_settings::LoadedGrid;
use crate::audio::MenuMusicSettings;
use crate::rendering::grass::FadeLimits;
use crate::rendering::local_lights::LightFade;
use crate::rendering::post::depth_of_field::BokehGate;
use crate::rendering::shadows::lamps::LampShadowConfig;
use crate::rendering::shadows::settings::ShadowConfig;
use crate::rendering::texture_streaming::TextureStreamingConfig;
use crate::rendering::ui_colour::UiColours;
use crate::rendering::water::WaterConfig;
use crate::rendering::world_streamer::MAX_GRIDS_TO_LOAD;
use crate::video::MenuVideoSettings;

const WORLD_FOV: IniKey = IniKey::new("Interface", "fDefaultWorldFOV");
const SAFE_ZONE_X: IniKey = IniKey::new("Interface", "fSafeZoneXWide");
/// The vertical safe-zone margin the game uses: its INI key is misspelled in every shipped INI
/// ("fSafeZoneYWid"), so the game's built-in value applies.
const SAFE_ZONE_Y: f32 = 36.0;

/// The margins interface movies keep from the screen edges, in pixels of a 1280 x 720 movie.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SafeZone {
    pub x: f32,
    pub y: f32,
}

impl SafeZone {
    fn from_ini(ini: &GameIni) -> Result<Self, IniError> {
        Ok(Self {
            x: ini.require_non_negative(SAFE_ZONE_X)?,
            y: SAFE_ZONE_Y,
        })
    }
}

#[derive(Debug, Clone)]
pub struct GameSettings {
    /// Horizontal field of view in Bethesda degrees at 16:9.
    pub world_fov_beth: f32,
    pub ui_colours: UiColours,
    pub safe_zone: SafeZone,
    pub shadows: ShadowConfig,
    pub lamp_shadows: LampShadowConfig,
    pub light_fade: LightFade,
    pub texture_streaming: TextureStreamingConfig,
    pub loaded_grid: LoadedGrid,
    pub grass_fade_limits: FadeLimits,
    pub grass_placement: GrassSettings,
    pub water: WaterConfig,
    /// The depth of field gate in normal gameplay.
    pub bokeh_gate: BokehGate,
    pub menu_music: MenuMusicSettings,
    pub menu_video: MenuVideoSettings,
}

impl GameSettings {
    /// Reads the INI cascade the installer copied into `<asset_root>/data`.
    pub fn load(asset_root: &Path) -> Result<Self, IniError> {
        Self::from_ini(&GameIni::load(&asset_root.join(layout::DATA_DIR))?)
    }

    pub fn from_ini(ini: &GameIni) -> Result<Self, IniError> {
        let shadows = ShadowConfig::from_ini(ini)?;
        let loaded_grid = LoadedGrid::from_ini(ini, MAX_GRIDS_TO_LOAD)?;
        Ok(Self {
            world_fov_beth: ini.require_valid::<f32>(
                WORLD_FOV,
                "a number above 0 and below 180",
                |fov| *fov > 0.0 && *fov < 180.0,
            )?,
            ui_colours: UiColours::from_ini(ini)?,
            safe_zone: SafeZone::from_ini(ini)?,
            lamp_shadows: LampShadowConfig::from_ini(ini, &shadows)?,
            shadows,
            light_fade: LightFade::from_ini(ini)?,
            texture_streaming: TextureStreamingConfig::from_ini(ini)?,
            loaded_grid,
            grass_fade_limits: FadeLimits::from_ini(ini, loaded_grid)?,
            grass_placement: GrassSettings::from_ini(ini)?,
            water: WaterConfig::from_ini(ini)?,
            bokeh_gate: BokehGate::gameplay_from_ini(ini)?,
            menu_music: MenuMusicSettings::from_ini(ini)?,
            menu_video: MenuVideoSettings::from_ini(ini)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::test_support::{FULL_INI, ini_with};

    fn without(key: &str) -> GameIni {
        let kept: Vec<_> = FULL_INI
            .lines()
            .filter(|line| line.split_once('=').is_none_or(|(name, _)| name != key))
            .collect();
        GameIni::from_texts([kept.join("\n").as_str()])
    }

    fn fixture_keys() -> Vec<&'static str> {
        FULL_INI
            .lines()
            .filter_map(|line| line.split_once('=').map(|(key, _)| key))
            .collect()
    }

    #[test]
    fn a_complete_ini_gives_the_settings() {
        let settings = GameSettings::from_ini(&ini_with("")).unwrap();
        assert!((settings.world_fov_beth - 75.0).abs() < 1e-6);
        assert_eq!(settings.loaded_grid.radius(), 3);
        assert_eq!(settings.grass_placement.min_grass_size, 30);
        assert_eq!(settings.texture_streaming.max_anisotropy, 8);
        assert!((settings.grass_fade_limits.configured_start_game_units - 3000.0).abs() < 1e-3);
        assert!(!settings.water.reflections);
        assert!((settings.water.lod_range_game_units - 120_000.0).abs() < 1e-3);
        assert!(!settings.bokeh_gate.setting_enabled);
        assert!(settings.menu_music.enabled);
        assert!(settings.menu_video.enabled);
        assert_eq!(settings.safe_zone, SafeZone { x: 50.0, y: 36.0 });
    }

    #[test]
    fn load_reads_the_cascade_from_the_data_directory_of_the_asset_root() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join(layout::DATA_DIR);
        std::fs::create_dir(&data).unwrap();
        std::fs::write(data.join("Fallout4_Default.ini"), FULL_INI).unwrap();
        std::fs::write(
            data.join("Fallout4Prefs.ini"),
            "[Display]\niMaxAnisotropy=4\n",
        )
        .unwrap();

        let settings = GameSettings::load(root.path()).unwrap();

        assert_eq!(settings.texture_streaming.max_anisotropy, 4);
        assert!((settings.world_fov_beth - 75.0).abs() < 1e-6);
    }

    #[test]
    fn load_without_a_data_directory_is_a_missing_file_error() {
        let root = tempfile::tempdir().unwrap();
        let error = GameSettings::load(root.path()).unwrap_err();
        assert!(matches!(error, IniError::MissingFile { .. }), "{error}");
    }

    #[test]
    fn dropping_any_one_setting_is_an_error_naming_exactly_that_setting() {
        for key in fixture_keys() {
            let error = GameSettings::from_ini(&without(key)).unwrap_err();
            assert!(
                matches!(&error, IniError::MissingKey { key: missing, .. } if *missing == key),
                "{key}: {error}"
            );
        }
    }

    #[test]
    fn the_field_of_view_must_be_a_usable_angle() {
        for text in [
            "fDefaultWorldFOV=0\n",
            "fDefaultWorldFOV=180\n",
            "fDefaultWorldFOV=-70\n",
            "fDefaultWorldFOV=NaN\n",
            "fDefaultWorldFOV=wide\n",
        ] {
            let error = GameSettings::from_ini(&ini_with(text)).unwrap_err();
            assert!(error.to_string().contains("fDefaultWorldFOV"), "{text}");
        }
    }

    #[test]
    fn the_field_of_view_accepts_the_open_interval() {
        for fov in [0.5, 70.0, 179.5] {
            let settings = GameSettings::from_ini(&ini_with(&format!("fDefaultWorldFOV={fov}\n")));
            assert!((settings.unwrap().world_fov_beth - fov).abs() < 1e-6);
        }
    }

    #[test]
    fn a_grid_beyond_the_streamer_is_clamped_for_every_consumer() {
        let settings = GameSettings::from_ini(&ini_with("uGridsToLoad=25\n")).unwrap();
        assert_eq!(
            settings.loaded_grid.radius(),
            (MAX_GRIDS_TO_LOAD / 2) as i32
        );
        assert!(
            (settings.grass_fade_limits.grid_cap_game_units
                - settings.loaded_grid.extent_game_units())
            .abs()
                < 1e-3
        );
    }
}
