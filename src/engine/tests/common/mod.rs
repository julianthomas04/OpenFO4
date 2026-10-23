//! Shared by the tests that need a converted installation.

use std::path::{Path, PathBuf};

use openfo4::assets::layout::data_dir;
use openfo4::assets::{AssetRootEnv, resolve_asset_root};

/// The asset root of the installation named by `OPENFO4_INSTALL_DIR`.
pub fn asset_root() -> PathBuf {
    resolve_asset_root(
        &AssetRootEnv::from_process(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
    )
    .expect("set OPENFO4_INSTALL_DIR to a converted installation")
}

/// `OPENFO4_DB` when set, else the database of the installation.
pub fn database_path() -> PathBuf {
    if let Some(path) = std::env::var_os("OPENFO4_DB") {
        return PathBuf::from(path);
    }
    PathBuf::from(format!(
        "{}/data.db",
        data_dir(asset_root().to_str().unwrap())
    ))
}
