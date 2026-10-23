use std::sync::Arc;

use super::residency::TextureResidency;
use crate::assets::types::texture::TexturePayload;
use crate::rendering::gpu_driven::{
    BLACK_SLOT, FALLBACK_SLOT, FLAT_NORMAL_SLOT, TextureArray, TextureHandle, WHITE_SLOT,
};

/// What a material texture is used for; fixes its colour space and the slot
/// shown while it is missing or still loading.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TextureRole {
    Diffuse,
    Normal,
    Specular,
    Glow,
    /// Effect environment-reflection mask, linear.
    EnvMask,
    /// Effect greyscale-to-colour gradient.
    Palette,
}

impl TextureRole {
    pub fn fallback_slot(self) -> u32 {
        match self {
            TextureRole::Diffuse => FALLBACK_SLOT,
            TextureRole::Normal => FLAT_NORMAL_SLOT,
            TextureRole::Specular | TextureRole::EnvMask | TextureRole::Palette => WHITE_SLOT,
            TextureRole::Glow => BLACK_SLOT,
        }
    }

    pub fn is_srgb(self) -> bool {
        match self {
            TextureRole::Diffuse | TextureRole::Glow | TextureRole::Palette => true,
            TextureRole::Normal | TextureRole::Specular | TextureRole::EnvMask => false,
        }
    }

    /// Slot of a role the material names no texture for. An emitting material
    /// without a glow map is lit by its emissive scalars alone, so it samples
    /// WHITE; every other role uses its failed-load fallback.
    pub fn absent_slot(self, emissive: bool) -> u32 {
        match self {
            TextureRole::Glow if emissive => WHITE_SLOT,
            _ => self.fallback_slot(),
        }
    }
}

pub enum Admission {
    Admitted {
        handle: TextureHandle,
        uploaded_bytes: u64,
    },
    /// The array is full; nothing was inserted.
    Exhausted,
    /// The cause was logged once for the path.
    Failed,
}

/// Admits `path` into the bindless array, taking a reference. A texture that is
/// already resident is just referenced; one that is not shows a placeholder of
/// the fallback slot until its pixels arrive.
pub fn admit_texture(
    array: &mut TextureArray,
    residency: &mut TextureResidency,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: &str,
    srgb: bool,
    fallback_slot: u32,
    payload: Option<&Arc<TexturePayload>>,
    upload_now: bool,
) -> Admission {
    let placeholder_slot = if fallback_slot == FALLBACK_SLOT {
        WHITE_SLOT
    } else {
        fallback_slot
    };
    match residency.admit(
        array,
        device,
        queue,
        path,
        srgb,
        payload,
        upload_now,
        placeholder_slot,
    ) {
        Ok(Some((handle, uploaded_bytes))) => Admission::Admitted {
            handle,
            uploaded_bytes,
        },
        Ok(None) => Admission::Exhausted,
        Err(error) => {
            crate::utils::warn_once(&format!("texture-admit:{path}"), || {
                format!("failed to upload texture '{path}': {error}; using fallback slot")
            });
            Admission::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_roles_fall_back_to_white() {
        assert_eq!(TextureRole::EnvMask.fallback_slot(), WHITE_SLOT);
        assert_eq!(TextureRole::Palette.fallback_slot(), WHITE_SLOT);
        assert_eq!(TextureRole::EnvMask.absent_slot(false), WHITE_SLOT);
        assert_eq!(TextureRole::Palette.absent_slot(true), WHITE_SLOT);
    }

    #[test]
    fn only_the_mask_is_linear() {
        assert!(!TextureRole::EnvMask.is_srgb());
        assert!(TextureRole::Palette.is_srgb());
        assert!(TextureRole::Diffuse.is_srgb());
    }
}
