pub mod db;
pub mod handle;
pub mod ini;
pub mod layout;
pub mod pipboy_art;
pub mod pipboy_catalog;
pub mod pipboy_playback;
pub mod resolve;
pub mod sky_game_settings;
pub mod types;
pub mod world_clock_start;
pub mod world_settings;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use layout::{ASSETS_DIR, DATABASE_FILE};

pub const INSTALL_DIR_VAR: &str = "OPENFO4_INSTALL_DIR";
const LEGACY_ASSET_ROOT_VAR: &str = "OPENFO4_ASSET_ROOT";

#[derive(Debug, thiserror::Error)]
pub enum AssetRootError {
    #[error(
        "{LEGACY_ASSET_ROOT_VAR} is no longer supported: set {INSTALL_DIR_VAR} to the folder \
         that contains `{ASSETS_DIR}` and `cache` (the parent of the old value)"
    )]
    LegacyVariableSet,
    #[error("{INSTALL_DIR_VAR}={0:?} is not an existing directory")]
    InstallDirNotADirectory(PathBuf),
    #[error("{INSTALL_DIR_VAR}={0:?} does not contain an `{ASSETS_DIR}` directory")]
    InstallDirWithoutAssets(PathBuf),
    #[error(
        "no asset root found: {INSTALL_DIR_VAR} is unset and {0:?} has no `{ASSETS_DIR}` \
         directory next to the executable"
    )]
    NotFound(PathBuf),
}

/// Environment overrides read at startup, injected so resolution stays testable.
#[derive(Debug, Default, Clone)]
pub struct AssetRootEnv {
    pub install_dir: Option<OsString>,
    pub legacy_asset_root: Option<OsString>,
}

impl AssetRootEnv {
    pub fn from_process() -> Self {
        Self {
            install_dir: std::env::var_os(INSTALL_DIR_VAR),
            legacy_asset_root: std::env::var_os(LEGACY_ASSET_ROOT_VAR),
        }
    }
}

/// Resolves the asset root, in order:
/// 1. `OPENFO4_INSTALL_DIR` (the folder holding `assets` and `cache`); an invalid
///    value is an error, never a fallback. The retired `OPENFO4_ASSET_ROOT` is an error.
/// 2. `<exe_dir>/assets` (the installed layout).
pub fn resolve_asset_root(env: &AssetRootEnv, exe_dir: &Path) -> Result<PathBuf, AssetRootError> {
    if env.legacy_asset_root.is_some() {
        return Err(AssetRootError::LegacyVariableSet);
    }
    if let Some(value) = &env.install_dir {
        let install_dir = PathBuf::from(value);
        if !install_dir.is_dir() {
            return Err(AssetRootError::InstallDirNotADirectory(install_dir));
        }
        let assets = install_dir.join(ASSETS_DIR);
        return if assets.is_dir() {
            Ok(assets)
        } else {
            Err(AssetRootError::InstallDirWithoutAssets(install_dir))
        };
    }
    let beside_exe = exe_dir.join(ASSETS_DIR);
    if beside_exe.is_dir() {
        Ok(beside_exe)
    } else {
        Err(AssetRootError::NotFound(exe_dir.to_path_buf()))
    }
}

/// The install root: the folder holding `assets` and `cache`, i.e. the parent of the asset root.
pub fn install_dir_from_environment() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let exe_dir = exe.parent().unwrap_or_else(|| Path::new("."));
    let assets = resolve_asset_root(&AssetRootEnv::from_process(), exe_dir)?;
    assets
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow::anyhow!("asset root {} has no parent folder", assets.display()))
}

/// Mesh parsing and DDS pre-reads live on the streaming worker thread
/// (`world_streamer.rs`); the asset server just resolves relative paths under
/// the asset root. It deliberately has no texture cache: GPU residency (and
/// the worker's pre-read dedup) is tracked by `TextureArray`'s refcounts, and a
/// CPU-side view cache in lockstep with it was pure redundancy.
#[derive(Clone)]
pub struct AssetServer {
    asset_location: String,
}

impl AssetServer {
    pub fn new(asset_location: String) -> Self {
        AssetServer { asset_location }
    }

    /// Resolves the asset root from the environment and the running
    /// executable's location (see [`resolve_asset_root`]).
    pub fn from_environment() -> anyhow::Result<Self> {
        let exe = std::env::current_exe()?;
        let exe_dir = exe.parent().unwrap_or_else(|| Path::new("."));
        let root = resolve_asset_root(&AssetRootEnv::from_process(), exe_dir)?;
        let root = root.into_os_string().into_string().map_err(|root| {
            anyhow::anyhow!("asset root {} is not valid UTF-8", root.to_string_lossy())
        })?;
        Ok(Self::new(root))
    }

    pub fn asset_location(&self) -> &str {
        &self.asset_location
    }

    pub fn db_path(&self) -> String {
        format!("{}/{DATABASE_FILE}", layout::data_dir(&self.asset_location))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install_with_assets() -> (tempfile::TempDir, PathBuf) {
        let install = tempfile::tempdir().unwrap();
        let assets = install.path().join(ASSETS_DIR);
        std::fs::create_dir(&assets).unwrap();
        (install, assets)
    }

    fn install_env(value: impl Into<OsString>) -> AssetRootEnv {
        AssetRootEnv {
            install_dir: Some(value.into()),
            legacy_asset_root: None,
        }
    }

    #[test]
    fn the_database_sits_in_the_data_folder_of_the_asset_root() {
        let server = AssetServer::new("root".to_owned());
        assert_eq!(server.db_path(), "root/data/data.db");
    }

    #[test]
    fn install_dir_env_resolves_to_its_assets_folder() {
        let (install, assets) = install_with_assets();
        let resolved =
            resolve_asset_root(&install_env(install.path()), Path::new("nowhere")).unwrap();
        assert_eq!(resolved, assets);
    }

    #[test]
    fn install_dir_env_beats_assets_beside_the_exe() {
        let (exe_dir, _) = install_with_assets();
        let (install, assets) = install_with_assets();
        let resolved = resolve_asset_root(&install_env(install.path()), exe_dir.path()).unwrap();
        assert_eq!(resolved, assets);
    }

    #[test]
    fn install_dir_env_pointing_at_a_missing_directory_is_an_error_not_a_fallback() {
        let (exe_dir, _) = install_with_assets();
        let missing = exe_dir.path().join("missing");
        let error = resolve_asset_root(&install_env(missing.clone()), exe_dir.path()).unwrap_err();
        assert!(matches!(error, AssetRootError::InstallDirNotADirectory(p) if p == missing));
    }

    #[test]
    fn install_dir_env_pointing_at_a_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        std::fs::write(&file, b"").unwrap();
        let error = resolve_asset_root(&install_env(file.clone()), dir.path()).unwrap_err();
        assert!(matches!(error, AssetRootError::InstallDirNotADirectory(p) if p == file));
    }

    #[test]
    fn install_dir_env_without_an_assets_folder_is_an_error_not_a_fallback() {
        let (exe_dir, _) = install_with_assets();
        let empty = tempfile::tempdir().unwrap();
        let error = resolve_asset_root(&install_env(empty.path()), exe_dir.path()).unwrap_err();
        assert!(matches!(error, AssetRootError::InstallDirWithoutAssets(p) if p == empty.path()));
    }

    #[test]
    fn empty_install_dir_env_is_an_error_not_unset() {
        let (exe_dir, _) = install_with_assets();
        let error = resolve_asset_root(&install_env(OsString::new()), exe_dir.path()).unwrap_err();
        assert!(matches!(error, AssetRootError::InstallDirNotADirectory(_)));
    }

    #[test]
    fn the_retired_asset_root_variable_is_an_error_naming_the_replacement() {
        let (install, _) = install_with_assets();
        let env = AssetRootEnv {
            install_dir: Some(install.path().into()),
            legacy_asset_root: Some("old".into()),
        };
        let error = resolve_asset_root(&env, install.path()).unwrap_err();
        assert!(matches!(error, AssetRootError::LegacyVariableSet));
        assert!(error.to_string().contains(INSTALL_DIR_VAR));
    }

    #[test]
    fn assets_beside_the_exe_are_used_by_default() {
        let (exe_dir, assets) = install_with_assets();
        let resolved = resolve_asset_root(&AssetRootEnv::default(), exe_dir.path()).unwrap();
        assert_eq!(resolved, assets);
    }

    #[test]
    fn an_install_above_the_exe_directory_is_not_searched_for() {
        let (install, _) = install_with_assets();
        let exe_dir = install.path().join("target").join("debug");
        std::fs::create_dir_all(&exe_dir).unwrap();
        let error = resolve_asset_root(&AssetRootEnv::default(), &exe_dir).unwrap_err();
        assert!(matches!(error, AssetRootError::NotFound(p) if p == exe_dir));
    }

    #[test]
    fn nothing_found_reports_everything_that_was_tried() {
        let empty = tempfile::tempdir().unwrap();
        let error = resolve_asset_root(&AssetRootEnv::default(), empty.path()).unwrap_err();
        assert!(matches!(error, AssetRootError::NotFound(ref p) if p == empty.path()));
        let message = error.to_string();
        assert!(message.contains(INSTALL_DIR_VAR));
        assert!(message.contains(ASSETS_DIR));
    }
}
