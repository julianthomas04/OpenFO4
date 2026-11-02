//! The large loading screen's background image, loaded once when first needed.

use std::path::PathBuf;

use crate::assets::layout::texture_key;
use crate::rendering::texture::load_texture_asset;

/// Below the textures folder; the game's own image for every large screen.
const BACKGROUND_TEXTURE: &str = "Interface\\LoadingMenuBG.dds";

enum State {
    NotLoaded,
    Ready(wgpu::TextureView),
    /// The load failed and was reported; the screen stays black rather than trying every frame.
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

    /// The image's view, loading it on the first call; `None`, after a warning logged once, when
    /// the install has no usable image.
    pub fn view(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Option<&wgpu::TextureView> {
        if matches!(self.state, State::NotLoaded) {
            self.state = self.load(device, queue);
        }
        match &self.state {
            State::Ready(view) => Some(view),
            State::NotLoaded | State::Unavailable => None,
        }
    }

    fn load(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> State {
        let key = texture_key(BACKGROUND_TEXTURE);
        match load_texture_asset(device, queue, &self.asset_root.to_string_lossy(), &key) {
            Ok(view) => State::Ready(view),
            Err(error) => {
                crate::utils::warn_once("loading-background", || {
                    format!("{error}; large loading screens draw on black")
                });
                State::Unavailable
            }
        }
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

        assert!(background.view(&device, &queue).is_some());
    }

    #[test]
    fn a_missing_image_is_none_and_stays_none() {
        let Some((device, queue)) = test_device("loading background missing") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let mut background = LoadingBackground::new(dir.path().to_path_buf());

        assert!(background.view(&device, &queue).is_none());
        write_solid_texture(dir.path(), "Interface/LoadingMenuBG.DDS", [255; 4]);
        assert!(
            background.view(&device, &queue).is_none(),
            "a failed load is not retried every frame"
        );
    }
}
