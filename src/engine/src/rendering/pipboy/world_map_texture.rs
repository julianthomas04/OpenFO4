//! The world-map image as an egui native texture, loaded once at startup.

use std::io;
use std::path::Path;

use egui::{TextureId, Vec2};

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

/// The path below `root` that `key` names, with every component matched ignoring ASCII case
/// (the archives store mixed case, the contract keys are lower case) and joined with `/`.
pub fn resolve_below(root: &Path, key: &str) -> io::Result<String> {
    let mut current = root.to_path_buf();
    let mut parts = Vec::new();
    for component in key.split(['\\', '/']).filter(|part| !part.is_empty()) {
        let name = if current.join(component).exists() {
            component.to_owned()
        } else {
            find_ignoring_case(&current, component)?
        };
        current.push(&name);
        parts.push(name);
    }
    Ok(parts.join("/"))
}

fn find_ignoring_case(directory: &Path, wanted: &str) -> io::Result<String> {
    for entry in std::fs::read_dir(directory)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name.eq_ignore_ascii_case(wanted) {
            return Ok(name);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("'{wanted}' not found in {}", directory.display()),
    ))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_resolves_through_directories_of_a_different_case() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("Textures").join("Interface");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("WorldMap_d.dds"), b"dds").unwrap();

        let relative = resolve_below(dir.path(), "textures\\interface\\worldmap_d.dds").unwrap();

        assert!(dir.path().join(&relative).is_file(), "{relative}");
        assert!(relative.eq_ignore_ascii_case("textures/interface/worldmap_d.dds"));
    }

    #[test]
    fn a_missing_file_is_a_not_found_error_naming_the_component() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("textures")).unwrap();

        let error = resolve_below(dir.path(), "textures\\worldmap_d.dds").unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error.to_string().contains("worldmap_d.dds"));
    }

    #[test]
    fn a_missing_root_is_an_error_not_an_empty_path() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nowhere");
        assert!(resolve_below(&missing, "textures\\a.dds").is_err());
    }
}
