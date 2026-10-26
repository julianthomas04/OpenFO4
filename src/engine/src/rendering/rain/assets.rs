//! The two textures the rain is drawn with: the streak texture the weather names and the splash
//! atlas. A file that is missing or unusable is a typed error, logged once; what it belongs to
//! is then not drawn, and [`RainAssetStatus`] says why.

use std::io;
use std::path::Path;

use crate::assets::layout::texture_key;
use crate::assets::resolve::resolve_below;
use crate::assets::types::texture::TextureError;
use crate::rendering::texture::load_dds_view_srgb;

/// The splash atlas, below the textures folder: 16 frames side by side.
const SPLASH_TEXTURE: &str = "effects\\rainscene\\rain_rainsplash_a_16x1_d.dds";

#[derive(Debug, thiserror::Error)]
pub enum RainAssetError {
    #[error("rain texture '{key}' is not in the install: {source}")]
    NotFound {
        key: String,
        #[source]
        source: io::Error,
    },
    #[error("rain texture '{key}' cannot be read: {source}")]
    Unreadable {
        key: String,
        #[source]
        source: io::Error,
    },
    #[error("rain texture '{key}' is unusable: {source}")]
    Texture {
        key: String,
        #[source]
        source: TextureError,
    },
}

/// Whether one texture of the rain is in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RainTextureState {
    NotRequested,
    Loaded,
    /// The message of the [`RainAssetError`].
    Failed(String),
}

impl RainTextureState {
    fn of(loaded: &Result<wgpu::TextureView, RainAssetError>) -> Self {
        match loaded {
            Ok(_) => Self::Loaded,
            Err(error) => Self::Failed(error.to_string()),
        }
    }
}

/// What the rain has loaded, for the dev menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RainAssetStatus {
    pub streaks: RainTextureState,
    pub splashes: RainTextureState,
}

/// The key of the streak texture the weather names by its path below the textures folder
/// (`Effects\FXRaindrops_D.DDS`).
pub(super) fn streak_texture_key(path: &str) -> String {
    texture_key(&path.to_ascii_lowercase().replace('/', "\\"))
}

pub(super) fn splash_texture_key() -> String {
    texture_key(SPLASH_TEXTURE)
}

/// The path below `asset_root` that `key` names. A file that is not there is told apart from
/// one that cannot be looked up (permissions, a broken disk).
fn resolve(asset_root: &str, key: &str) -> Result<String, RainAssetError> {
    resolve_below(Path::new(asset_root), key).map_err(|source| {
        let key = key.to_owned();
        if source.kind() == io::ErrorKind::NotFound {
            RainAssetError::NotFound { key, source }
        } else {
            RainAssetError::Unreadable { key, source }
        }
    })
}

/// The sRGB-decoding view of the texture `key` below `asset_root`, which may differ from the
/// stored casing.
fn load_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    asset_root: &str,
    key: &str,
) -> Result<wgpu::TextureView, RainAssetError> {
    let relative = resolve(asset_root, key)?;
    load_dds_view_srgb(device, queue, asset_root, &relative).map_err(|source| {
        RainAssetError::Texture {
            key: key.to_owned(),
            source,
        }
    })
}

/// Logs a failed load once per file, so a missing texture does not repeat with every weather
/// change.
fn report(error: &RainAssetError) {
    crate::utils::warn_once(&format!("rain:{error}"), || {
        format!("{error}; what it belongs to is not drawn")
    });
}

/// The textures the rain draws with and why one is missing.
pub(super) struct RainTextures {
    asset_root: String,
    /// The texture the weather named last, loaded or not.
    requested: Option<String>,
    streaks: Option<wgpu::TextureView>,
    splashes: Option<wgpu::TextureView>,
    status: RainAssetStatus,
}

impl RainTextures {
    /// Loads the splash atlas; the streak texture follows with [`Self::request_streaks`].
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, asset_root: &str) -> Self {
        let splashes = load_texture(device, queue, asset_root, &splash_texture_key());
        let status = RainAssetStatus {
            streaks: RainTextureState::NotRequested,
            splashes: RainTextureState::of(&splashes),
        };
        if let Err(error) = &splashes {
            report(error);
        }
        Self {
            asset_root: asset_root.to_owned(),
            requested: None,
            streaks: None,
            splashes: splashes.ok(),
            status,
        }
    }

    /// Makes `path` the streak texture, loading it unless it already is; an empty path is a
    /// weather without one. Returns whether the texture in use changed.
    pub fn request_streaks(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        path: &str,
    ) -> bool {
        if self.requested.as_deref() == Some(path) {
            return false;
        }
        self.requested = Some(path.to_owned());
        if path.is_empty() {
            self.status.streaks = RainTextureState::NotRequested;
            self.streaks = None;
            return true;
        }
        let loaded = load_texture(device, queue, &self.asset_root, &streak_texture_key(path));
        if let Err(error) = &loaded {
            report(error);
        }
        self.status.streaks = RainTextureState::of(&loaded);
        self.streaks = loaded.ok();
        true
    }

    pub fn streaks(&self) -> Option<&wgpu::TextureView> {
        self.streaks.as_ref()
    }

    pub fn splashes(&self) -> Option<&wgpu::TextureView> {
        self.splashes.as_ref()
    }

    pub fn status(&self) -> &RainAssetStatus {
        &self.status
    }
}
