//! The two textures the rain is drawn with: the streak texture the weather names and the splash
//! atlas. A file that is missing or unusable is a typed error, logged once; what it belongs to
//! is then not drawn, and [`RainAssetStatus`] says why.

use crate::assets::layout::{named_texture_key, texture_key};
use crate::rendering::texture::{TextureAssetError, load_texture_asset_srgb};

/// The splash atlas, below the textures folder.
const SPLASH_TEXTURE: &str = "effects\\rainscene\\rain_rainsplash_a_16x1_d.dds";
/// The frames the splash atlas holds, side by side.
pub(super) const SPLASH_FRAMES: u32 = 16;

/// Whether one texture of the rain is in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RainTextureState {
    NotRequested,
    Loaded,
    /// The message of the [`TextureAssetError`].
    Failed(String),
}

/// What the rain has loaded, for the dev menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RainAssetStatus {
    pub streaks: RainTextureState,
    pub splashes: RainTextureState,
}

pub(super) fn splash_texture_key() -> String {
    texture_key(SPLASH_TEXTURE)
}

/// Logs a failed load once per file, so a missing texture does not repeat with every weather
/// change.
fn report(error: &TextureAssetError) {
    crate::utils::warn_once(&format!("rain:{error}"), || {
        format!("{error}; what it belongs to is not drawn")
    });
}

/// The view of the texture `key`, or the state that says why there is none.
fn load(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    asset_root: &str,
    key: &str,
) -> (Option<wgpu::TextureView>, RainTextureState) {
    match load_texture_asset_srgb(device, queue, asset_root, key) {
        Ok(view) => (Some(view), RainTextureState::Loaded),
        Err(error) => {
            report(&error);
            (None, RainTextureState::Failed(error.to_string()))
        }
    }
}

/// What the weather asked the streak texture to be.
enum Requested {
    /// No weather has asked yet.
    Nothing,
    /// A weather without a texture.
    NoTexture,
    Texture(String),
}

impl Requested {
    fn of(path: Option<&str>) -> Self {
        path.map_or(Self::NoTexture, |path| Self::Texture(path.to_owned()))
    }

    fn is(&self, path: Option<&str>) -> bool {
        match (self, path) {
            (Self::NoTexture, None) => true,
            (Self::Texture(requested), Some(path)) => requested == path,
            _ => false,
        }
    }
}

/// The textures the rain draws with and why one is missing.
pub(super) struct RainTextures {
    asset_root: String,
    /// The texture the weather named last, loaded or not.
    requested: Requested,
    streaks: Option<wgpu::TextureView>,
    splashes: Option<wgpu::TextureView>,
    status: RainAssetStatus,
}

impl RainTextures {
    /// Loads the splash atlas; the streak texture follows with [`Self::request_streaks`].
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, asset_root: &str) -> Self {
        let (splashes, splashes_state) = load(device, queue, asset_root, &splash_texture_key());
        Self {
            asset_root: asset_root.to_owned(),
            requested: Requested::Nothing,
            streaks: None,
            splashes,
            status: RainAssetStatus {
                streaks: RainTextureState::NotRequested,
                splashes: splashes_state,
            },
        }
    }

    /// Makes `path` the streak texture, loading it unless it already is; `None` is a
    /// weather without one. Returns whether the texture in use changed.
    pub fn request_streaks(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        path: Option<&str>,
    ) -> bool {
        if self.requested.is(path) {
            return false;
        }
        self.requested = Requested::of(path);
        (self.streaks, self.status.streaks) = match path {
            None => (None, RainTextureState::NotRequested),
            Some(path) => load(device, queue, &self.asset_root, &named_texture_key(path)),
        };
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
