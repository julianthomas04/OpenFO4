//! Folder and file names of the installed asset layout, the single source of truth on the
//! engine side. The installer writes the folders and files under the names of
//! `InstallLayout` in `src/installer/install_layout.cs`; tests below compare the two sides, so a
//! rename on one side alone fails the build. Only the installer's `staging` folder and the
//! engine's `cache` and `physics` folders are private to one side.
//!
//! Everything is lowercase. Folders below these (`Actors`, `Terrain`, ...) keep the casing of the
//! game's own archive paths and are never compared literally.

/// The file by which the installer claims a folder as an install, below the install root.
pub const OWNERSHIP_MARKER_FILE: &str = ".openfo4-install";
/// The installer's log, below the install root.
pub const INSTALL_LOG_FILE: &str = "install.log";
/// The folder below the install root that holds the converted game assets.
pub const ASSETS_DIR: &str = "assets";
pub const MESHES_DIR: &str = "meshes";
pub const TEXTURES_DIR: &str = "textures";
pub const INTERFACE_DIR: &str = "interface";
pub const FONTS_DIR: &str = "fonts";
/// The converted game audio (FLAC), keyed by the lowercased archive path.
pub const AUDIO_DIR: &str = "audio";
/// The converted game videos (MP4), named by the lowercased base name of the source.
pub const VIDEO_DIR: &str = "video";
/// The Pip-Boy art below the interface folder: the manifest and the frame files it names.
pub const PIPBOY_DIR: &str = "pipboy";
pub const PIPBOY_ART_MANIFEST_FILE: &str = "art_manifest.json";
/// The UI layout documents (one per game movie) and the installed language's translate table and
/// font configuration, below the interface folder.
pub const LAYOUT_DIR: &str = "layout";
pub const UI_TRANSLATE_FILE: &str = "translate.json";
pub const UI_FONT_CONFIG_FILE: &str = "fontconfig.json";
/// The database and the game INI cascade.
pub const DATA_DIR: &str = "data";
pub const DATABASE_FILE: &str = "data.db";
/// The WGSL the installer translates from the game's shaders.
pub const SHADERS_DIR: &str = "shaders";
/// Runtime caches the engine bakes itself; a sibling of [`ASSETS_DIR`], never written by the installer.
pub const CACHE_DIR: &str = "cache";
pub const PHYSICS_CACHE_DIR: &str = "physics";

pub fn data_dir(asset_root: &str) -> String {
    format!("{asset_root}/{DATA_DIR}")
}

pub fn audio_dir(asset_root: &str) -> String {
    format!("{asset_root}/{AUDIO_DIR}")
}

pub fn fonts_dir(asset_root: &str) -> String {
    format!("{asset_root}/{INTERFACE_DIR}/{FONTS_DIR}")
}

/// The converted file of the game video `source` (`MainMenuLoop.bk2`): the lowercased base name
/// with an `.mp4` extension, below the video folder of `asset_root`.
pub fn video_path(asset_root: &str, source: &str) -> String {
    let stem = source
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(source)
        .rsplit_once('.')
        .map_or(source, |(stem, _)| stem)
        .to_lowercase();
    format!("{asset_root}/{VIDEO_DIR}/{stem}.mp4")
}

pub fn pipboy_dir(asset_root: &str) -> String {
    format!("{asset_root}/{INTERFACE_DIR}/{PIPBOY_DIR}")
}

pub fn pipboy_art_manifest_path(asset_root: &str) -> String {
    format!("{}/{PIPBOY_ART_MANIFEST_FILE}", pipboy_dir(asset_root))
}

pub fn ui_layout_dir(asset_root: &str) -> String {
    format!("{asset_root}/{INTERFACE_DIR}/{LAYOUT_DIR}")
}

/// The asset-root-relative key of a mesh file: `meshes\<path>`.
pub fn mesh_key(path_below_meshes: &str) -> String {
    format!("{MESHES_DIR}\\{path_below_meshes}")
}

/// The part of a [`mesh_key`] below the meshes folder, or the whole key when it has another root.
pub fn below_meshes(key: &str) -> &str {
    key.strip_prefix(MESHES_DIR)
        .and_then(|rest| rest.strip_prefix('\\'))
        .unwrap_or(key)
}

/// The asset-root-relative key of a texture file: `textures\<path>`.
pub fn texture_key(path_below_textures: &str) -> String {
    format!("{TEXTURES_DIR}\\{path_below_textures}")
}

/// An audio path that cannot be placed below the audio folder.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("audio path {path:?} has an empty, \".\" or \"..\" segment")]
pub struct AudioKeyError {
    pub path: String,
}

/// The top-level audio folders the installer fills, named by archive paths.
const AUDIO_ROOTS: [&str; 2] = ["sound", "music"];
/// Sound descriptors list paths relative to `Data\Sound\`, such as `fx\fst\dirt\walk.wav`.
const BARE_PATH_ROOT: &str = "sound";

/// The path below the audio folder of a game audio file, from its archive path: lowercase,
/// forward slashes, a leading `\` or `Data\` dropped, and `.xwm` or `.wav` replaced by `.flac`.
/// A path that does not start with `Sound\` or `Music\` (after the `Data\`) is relative to
/// `Data\Sound\`, so it gets the `sound/` root.
/// A path with an empty, `.` or `..` segment is rejected, as the installer rejects it.
pub fn audio_key(archive_path: &str) -> Result<String, AudioKeyError> {
    let lowered = archive_path.to_lowercase().replace('\\', "/");
    let relative = lowered.trim_start_matches('/');
    let relative = relative.strip_prefix("data/").unwrap_or(relative);
    if relative
        .split('/')
        .any(|segment| matches!(segment, "" | "." | ".."))
    {
        return Err(AudioKeyError {
            path: archive_path.to_string(),
        });
    }
    let relative = if AUDIO_ROOTS
        .iter()
        .any(|root| relative.starts_with(&format!("{root}/")))
    {
        relative.to_string()
    } else {
        format!("{BARE_PATH_ROOT}/{relative}")
    };
    let relative = relative.as_str();
    Ok(
        match relative
            .strip_suffix(".xwm")
            .or_else(|| relative.strip_suffix(".wav"))
        {
            Some(stem) => format!("{stem}.flac"),
            None => relative.to_string(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installer_source;

    const INSTALLER_LAYOUT: &str = include_str!("../../../installer/install_layout.cs");

    /// The installer's private intermediates folder, which the engine never reads.
    const INSTALLER_ONLY_CONSTANT: &str = "StagingFolder";

    /// The values of the constants of `InstallLayout` whose name ends with `name_suffix`,
    /// without the installer-only constant.
    fn installer_values(name_suffix: &str) -> Vec<&'static str> {
        installer_source::constants(INSTALLER_LAYOUT)
            .filter(|&(name, _)| name != INSTALLER_ONLY_CONSTANT && name.ends_with(name_suffix))
            .map(|(_, value)| value)
            .collect()
    }

    #[test]
    fn a_video_is_found_by_its_lowercased_base_name_with_the_mp4_extension() {
        assert_eq!(
            video_path("root/assets", "MainMenuLoop.bk2"),
            "root/assets/video/mainmenuloop.mp4"
        );
        assert_eq!(
            video_path("root/assets", "Data\\Video\\GameIntro_V3_B.bk2"),
            "root/assets/video/gameintro_v3_b.mp4"
        );
    }

    #[test]
    fn the_folders_the_installer_writes_are_the_ones_the_engine_reads() {
        let mut engine = vec![
            ASSETS_DIR,
            MESHES_DIR,
            TEXTURES_DIR,
            INTERFACE_DIR,
            FONTS_DIR,
            AUDIO_DIR,
            VIDEO_DIR,
            PIPBOY_DIR,
            LAYOUT_DIR,
            DATA_DIR,
            SHADERS_DIR,
        ];
        let mut installer = installer_values("Folder");
        engine.sort_unstable();
        installer.sort_unstable();
        assert_eq!(engine, installer);
    }

    #[test]
    fn the_files_the_installer_writes_are_the_ones_the_engine_reads() {
        let mut engine = vec![
            OWNERSHIP_MARKER_FILE,
            INSTALL_LOG_FILE,
            DATABASE_FILE,
            PIPBOY_ART_MANIFEST_FILE,
            UI_TRANSLATE_FILE,
            UI_FONT_CONFIG_FILE,
        ];
        let mut installer = installer_values("FileName");
        engine.sort_unstable();
        installer.sort_unstable();
        assert_eq!(engine, installer);
    }

    #[test]
    fn the_layout_names_are_lowercase() {
        assert_eq!(
            [
                ASSETS_DIR,
                MESHES_DIR,
                TEXTURES_DIR,
                INTERFACE_DIR,
                FONTS_DIR,
                AUDIO_DIR,
                VIDEO_DIR,
                PIPBOY_DIR,
                LAYOUT_DIR,
                DATA_DIR,
                SHADERS_DIR,
                CACHE_DIR,
                PHYSICS_CACHE_DIR,
            ],
            [
                "assets",
                "meshes",
                "textures",
                "interface",
                "fonts",
                "audio",
                "video",
                "pipboy",
                "layout",
                "data",
                "shaders",
                "cache",
                "physics",
            ]
        );
    }

    #[test]
    fn composed_paths_use_the_layout_names() {
        assert_eq!(data_dir("root"), "root/data");
        assert_eq!(fonts_dir("root"), "root/interface/fonts");
        assert_eq!(audio_dir("root"), "root/audio");
        assert_eq!(pipboy_dir("root"), "root/interface/pipboy");
        assert_eq!(
            pipboy_art_manifest_path("root"),
            "root/interface/pipboy/art_manifest.json"
        );
        assert_eq!(ui_layout_dir("root"), "root/interface/layout");
    }

    #[test]
    fn keys_start_with_the_layout_folder_and_a_backslash() {
        assert_eq!(mesh_key("props\\crate.glb"), "meshes\\props\\crate.glb");
        assert_eq!(texture_key("wall_d.dds"), "textures\\wall_d.dds");
    }

    #[test]
    fn below_meshes_strips_exactly_the_meshes_root() {
        assert_eq!(below_meshes("meshes\\props\\crate.glb"), "props\\crate.glb");
        assert_eq!(below_meshes("meshesextra\\a.glb"), "meshesextra\\a.glb");
        assert_eq!(below_meshes("props\\crate.glb"), "props\\crate.glb");
    }

    fn key(archive_path: &str) -> String {
        audio_key(archive_path).unwrap()
    }

    #[test]
    fn audio_keys_follow_the_installer_contract() {
        assert_eq!(
            key(r"Music\Special\MUS_MainTheme.xwm"),
            "music/special/mus_maintheme.flac"
        );
        assert_eq!(key(r"Sound\fx\UI\Click.WAV"), "sound/fx/ui/click.flac");
    }

    #[test]
    fn audio_keys_accept_the_ini_spelling_with_its_data_prefix() {
        assert_eq!(
            key(r"\Data\Music\Special\MUS_MainTheme.xwm"),
            "music/special/mus_maintheme.flac"
        );
        assert_eq!(key(r"Data\Music\a.xwm"), "music/a.flac");
    }

    #[test]
    fn audio_keys_leave_other_extensions_alone() {
        assert_eq!(key(r"Music\Dataset\a.ogg"), "music/dataset/a.ogg");
    }

    #[test]
    fn audio_keys_put_a_path_relative_to_the_sound_folder_below_sound() {
        assert_eq!(
            key(r"fx\amb\water\exterior\amb_water_river_lp.wav"),
            "sound/fx/amb/water/exterior/amb_water_river_lp.flac"
        );
        assert_eq!(key(r"\fx\ui\Click.WAV"), "sound/fx/ui/click.flac");
        assert_eq!(key(r"database\a.xwm"), "sound/database/a.flac");
    }

    #[test]
    fn audio_keys_read_the_data_sound_spelling_as_the_sound_folder() {
        assert_eq!(
            key(r"Data\Sound\FX\FST\Dirt\Walk.wav"),
            "sound/fx/fst/dirt/walk.flac"
        );
        assert_eq!(key(r"\Data\Sound\fx\a.wav"), "sound/fx/a.flac");
        assert_eq!(key(r"data\sound\fx\a.wav"), key(r"Sound\fx\a.wav"));
    }

    #[test]
    fn audio_keys_keep_the_music_root_in_every_spelling() {
        for path in [
            r"Music\Special\MUS_MainTheme.xwm",
            r"Data\Music\Special\MUS_MainTheme.xwm",
            r"\Data\Music\Special\MUS_MainTheme.xwm",
        ] {
            assert_eq!(key(path), "music/special/mus_maintheme.flac", "{path}");
        }
    }

    #[test]
    fn audio_keys_reject_paths_that_leave_the_audio_folder() {
        for path in [
            "",
            r"\Data\",
            r"..\a.xwm",
            r"Music\..\..\a.xwm",
            r"Music\.\a.xwm",
            r"Music\\a.xwm",
            r"Music\Special\",
            r"\Data\..\a.xwm",
        ] {
            assert_eq!(
                audio_key(path),
                Err(AudioKeyError {
                    path: path.to_string()
                }),
                "{path:?}"
            );
        }
    }
}
