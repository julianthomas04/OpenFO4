//! The world-map image as an egui native texture, loaded once at startup.

use std::io;
use std::path::Path;

use egui::{TextureId, Vec2};

use crate::assets::resolve::resolve_below;
use crate::assets::types::texture::{TextureError, read_layout};
use crate::rendering::egui_pass::EguiPass;
use crate::rendering::texture::load_dds_view;

#[derive(Debug, thiserror::Error)]
pub enum WorldMapTextureError {
    #[error("world map texture '{key}' is not in the install: {source}")]
    NotFound {
        key: String,
        #[source]
        source: io::Error,
    },
    #[error("world map texture '{key}' is unusable: {source}")]
    Unusable {
        key: String,
        #[source]
        source: TextureError,
    },
}

/// The registered world-map image and its native size in texels, which the map page shows at
/// one texel per stage pixel before the holder's scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldMapImage {
    pub id: TextureId,
    pub size: Vec2,
}

/// Loads the map image `key` below `asset_root` with all its mips and registers it with `pass`
/// for mip-filtered sampling. Any failure is an error for the caller to stop on.
pub fn load(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pass: &mut EguiPass,
    asset_root: &str,
    key: &str,
) -> Result<WorldMapImage, WorldMapTextureError> {
    let relative = resolve_below(Path::new(asset_root), key).map_err(|source| {
        WorldMapTextureError::NotFound {
            key: key.to_owned(),
            source,
        }
    })?;
    let unusable = |source| WorldMapTextureError::Unusable {
        key: key.to_owned(),
        source,
    };
    let layout = read_layout(&format!("{asset_root}/{relative}")).map_err(unusable)?;
    let view = load_dds_view(device, queue, asset_root, &relative).map_err(unusable)?;
    Ok(WorldMapImage {
        id: pass.register_mipmapped_texture(device, &view),
        size: Vec2::new(layout.width as f32, layout.height as f32),
    })
}
