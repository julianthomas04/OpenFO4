//! The large loading screen's background image and the grid overlay added over it, loaded
//! together once when first needed.

use std::path::PathBuf;

use crate::assets::layout::texture_key;
use crate::rendering::texture::load_texture_asset;

/// Below the textures folder; the game's own image for every large screen.
const BACKGROUND_TEXTURE: &str = r"Interface\LoadingMenuBG.dds";

/// Below the textures folder; the grid the game adds over the image.
const OVERLAY_TEXTURE: &str = r"Interface\Objects\LoadScreen\Overlay01_d.dds";

/// The views of one large screen's background.
#[derive(Debug, Clone, Copy)]
pub struct BackgroundImages<'a> {
    pub background: &'a wgpu::TextureView,
    /// `None` when the install lacks the grid: the background is then drawn without it.
    pub overlay: Option<&'a wgpu::TextureView>,
}

enum State {
    NotLoaded,
    Ready {
        background: wgpu::TextureView,
        overlay: Option<wgpu::TextureView>,
    },
    /// The background failed to load and was reported; the screen stays black rather than
    /// trying every frame.
    Unavailable,
}

pub struct LoadingBackground {
    asset_root: PathBuf,
    state: State,
}

impl LoadingBackground {
    pub fn new(asset_root: PathBuf) -> Self {
        Self {
            asset_root,
            state: State::NotLoaded,
        }
    }

    /// The images, loaded on the first call; `None`, after a warning logged once, when the
    /// install has no usable background. A missing overlay is reported the same way but only
    /// costs the grid.
    pub fn images(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Option<BackgroundImages<'_>> {
        if matches!(self.state, State::NotLoaded) {
            self.state = self.load(device, queue);
        }
        match &self.state {
            State::Ready {
                background,
                overlay,
            } => Some(BackgroundImages {
                background,
                overlay: overlay.as_ref(),
            }),
            State::NotLoaded | State::Unavailable => None,
        }
    }

    /// Whether loading the background has failed, which is not retried.
    pub fn is_unavailable(&self) -> bool {
        matches!(self.state, State::Unavailable)
    }

    fn load(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> State {
        let background = match self.load_texture(device, queue, BACKGROUND_TEXTURE) {
            Ok(view) => view,
            Err(error) => {
                crate::utils::warn_once("loading-background", || {
                    format!("{error}; large loading screens draw on black")
                });
                return State::Unavailable;
            }
        };
        let overlay = match self.load_texture(device, queue, OVERLAY_TEXTURE) {
            Ok(view) => Some(view),
            Err(error) => {
                crate::utils::warn_once("loading-background-overlay", || {
                    format!("{error}; large loading screens draw without their grid")
                });
                None
            }
        };
        State::Ready {
            background,
            overlay,
        }
    }

    fn load_texture(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        name: &str,
    ) -> Result<wgpu::TextureView, crate::rendering::texture::TextureAssetError> {
        let key = texture_key(name);
        load_texture_asset(device, queue, &self.asset_root.to_string_lossy(), &key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::gpu_test::test_device;
    use crate::rendering::loading_screen::test_support::write_solid_texture;

    #[test]
    fn the_image_is_found_whatever_the_casing_of_its_file() {
        let Some((device, queue)) = test_device("loading background found") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        write_solid_texture(dir.path(), "Interface/LoadingMenuBG.DDS", [255; 4]);
        let mut background = LoadingBackground::new(dir.path().to_path_buf());

        assert!(background.images(&device, &queue).is_some());
    }

    #[test]
    fn a_missing_image_is_none_and_stays_none() {
        let Some((device, queue)) = test_device("loading background missing") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let mut background = LoadingBackground::new(dir.path().to_path_buf());

        assert!(background.images(&device, &queue).is_none());
        write_solid_texture(dir.path(), "Interface/LoadingMenuBG.DDS", [255; 4]);
        assert!(
            background.images(&device, &queue).is_none(),
            "a failed load is not retried every frame"
        );
    }

    #[test]
    fn the_overlay_is_found_whatever_the_casing_of_its_file() {
        let Some((device, queue)) = test_device("loading overlay found") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        write_solid_texture(dir.path(), "Interface/LoadingMenuBG.DDS", [255; 4]);
        write_solid_texture(
            dir.path(),
            "interface/objects/loadscreen/OVERLAY01_D.DDS",
            [255; 4],
        );
        let mut background = LoadingBackground::new(dir.path().to_path_buf());

        let images = background.images(&device, &queue).expect("the images load");
        assert!(images.overlay.is_some());
    }

    #[test]
    fn a_missing_overlay_leaves_the_background_without_a_grid() {
        let Some((device, queue)) = test_device("loading overlay missing") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        write_solid_texture(dir.path(), "Interface/LoadingMenuBG.DDS", [255; 4]);
        let mut background = LoadingBackground::new(dir.path().to_path_buf());

        let images = background
            .images(&device, &queue)
            .expect("the background loads");
        assert!(images.overlay.is_none());
    }

    #[test]
    fn an_overlay_without_a_background_is_not_enough() {
        let Some((device, queue)) = test_device("loading overlay alone") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        write_solid_texture(
            dir.path(),
            "Interface/Objects/LoadScreen/Overlay01_d.DDS",
            [255; 4],
        );
        let mut background = LoadingBackground::new(dir.path().to_path_buf());

        assert!(background.images(&device, &queue).is_none());
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the installed interface textures"]
    fn the_installed_images_load() {
        let Some((device, queue)) = test_device("loading background installed") else {
            return;
        };
        let install = std::env::var(crate::assets::INSTALL_DIR_VAR).expect("OPENFO4_INSTALL_DIR");
        let root = std::path::Path::new(&install).join(crate::assets::layout::ASSETS_DIR);
        let mut background = LoadingBackground::new(root);

        let images = background
            .images(&device, &queue)
            .expect("the background loads");
        assert!(images.overlay.is_some(), "the install has the grid overlay");
    }
}
