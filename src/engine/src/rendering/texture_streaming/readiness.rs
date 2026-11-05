//! Whether a mesh using a texture would draw it as it is meant to look: loaded (or settled on
//! its stand-in because its file is missing) and sampled by the material bind group.

use crate::rendering::gpu_driven::{TextureArray, TextureHandle};
use crate::rendering::texture_streaming::residency::{TextureLoad, TextureResidency};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureReadiness {
    /// Its file is still being read, or it was found broken and the game is ending.
    Loading,
    /// Loaded, or settled on its stand-in because its file is missing, but the material bind
    /// group still samples the view the slot held before.
    Unbound,
    Ready,
}

impl TextureReadiness {
    pub fn of(load: Option<TextureLoad>, view_bound: bool) -> Self {
        match load {
            Some(TextureLoad::Resident | TextureLoad::Missing) if view_bound => Self::Ready,
            Some(TextureLoad::Resident | TextureLoad::Missing) => Self::Unbound,
            Some(TextureLoad::Pending | TextureLoad::Broken) | None => Self::Loading,
        }
    }

    pub fn of_handle(
        residency: &TextureResidency,
        array: &TextureArray,
        handle: TextureHandle,
    ) -> Self {
        Self::of(residency.load_of(handle), array.view_bound(handle.slot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TextureReadiness::{Loading, Ready, Unbound};

    #[test]
    fn a_texture_is_ready_once_loaded_or_settled_on_its_stand_in_and_bound() {
        for load in [TextureLoad::Resident, TextureLoad::Missing] {
            assert_eq!(TextureReadiness::of(Some(load), true), Ready);
            assert_eq!(TextureReadiness::of(Some(load), false), Unbound);
        }
    }

    #[test]
    fn a_pending_broken_or_untracked_texture_is_still_loading() {
        for load in [Some(TextureLoad::Pending), Some(TextureLoad::Broken), None] {
            assert_eq!(TextureReadiness::of(load, true), Loading);
        }
    }
}
