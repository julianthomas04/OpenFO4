//! Textures of the water shader: the three WATR normal layers loaded from the
//! asset root, and the 1x1 stand-ins for the resources that do not exist yet.

use std::collections::HashMap;

use wgpu::util::DeviceExt;

use crate::assets::layout::{TEXTURES_DIR, texture_key};
use crate::utils::warn_once;

/// The wading displacement field at rest, bound as `t2` for every surface
/// without the wading permutation and while the field is idle: all four
/// channels at the 0.5 bias.
const NEUTRAL_RIPPLE_TEXEL: [u8; 4] = [128, 128, 128, 128];
/// Zero coverage: the shader falls back to the procedural sky gradient.
const EMPTY_REFLECTION_TEXEL: [u8; 4] = [0, 0, 0, 0];
/// The decoded normal (0, 0, 1): both channels at the middle of the range.
const FLAT_NORMAL_TEXEL: [u8; 2] = [128, 128];

const PLUGIN_DATA_PREFIX: &str = "data\\";

/// The texture cache key and relative path of a WATR texture: the plugin
/// stores `data\Textures\Water\x.dds`, the asset root holds `textures\Water\x.dds`.
/// The plugin's casing of the textures root is replaced by the layout's.
pub fn asset_path(plugin_path: &str) -> String {
    let trimmed = plugin_path.trim();
    let has_prefix = trimmed
        .get(..PLUGIN_DATA_PREFIX.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(PLUGIN_DATA_PREFIX));
    let relative = if has_prefix {
        &trimmed[PLUGIN_DATA_PREFIX.len()..]
    } else {
        trimmed
    }
    .replace('/', "\\");
    match relative.split_once('\\') {
        Some((root, below_root)) if root.eq_ignore_ascii_case(TEXTURES_DIR) => {
            texture_key(below_root)
        }
        _ => relative,
    }
}

fn texel_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    format: wgpu::TextureFormat,
    texel: &[u8],
) -> wgpu::TextureView {
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            texel,
        )
        .create_view(&wgpu::TextureViewDescriptor::default())
}

/// The fixed textures bound next to the frame constants.
pub struct StandInTextures {
    pub ripple: wgpu::TextureView,
    pub reflection_sharp: wgpu::TextureView,
    pub reflection_blurred: wgpu::TextureView,
}

/// The `t9` (sharp) and `t10` (blurred) SSR textures the water shader samples:
/// the SSR chain's sharp and blurred reflections, or the empty stand-ins while it does not run.
#[derive(Clone)]
pub struct ReflectionViews {
    pub sharp: wgpu::TextureView,
    pub blurred: wgpu::TextureView,
}

impl StandInTextures {
    pub fn reflection(&self) -> ReflectionViews {
        ReflectionViews {
            sharp: self.reflection_sharp.clone(),
            blurred: self.reflection_blurred.clone(),
        }
    }

    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let rgba = |label, texel: &[u8; 4]| {
            texel_texture(device, queue, label, wgpu::TextureFormat::Rgba8Unorm, texel)
        };
        Self {
            ripple: rgba("water_ripple_neutral", &NEUTRAL_RIPPLE_TEXEL),
            reflection_sharp: rgba("water_ssr_sharp_empty", &EMPTY_REFLECTION_TEXEL),
            reflection_blurred: rgba("water_ssr_blurred_empty", &EMPTY_REFLECTION_TEXEL),
        }
    }
}

/// Format of the `t11` water mask: one byte per pixel, the tag in `x`.
pub const MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// The `t11` water mask the refraction test reads: cleared to tag 0, then every water surface that
/// survives the depth test against the opaque scene writes its tag (2, or 3
/// for wading) through the constant-colour passes `0x12xxx` / `0x14xxx`. The
/// refraction accepts a refracted pixel only where it carries tag 2 or 3, i.e.
/// where water is also drawn.
///
/// The tag passes are depth-tested like the shading draws and run before them, so the mask
/// belongs to the same frame. The engine has no stencil buffer, so no stencil-derived tags are
/// produced.
pub struct WaterMask {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: (u32, u32),
}

impl WaterMask {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let size = (width.max(1), height.max(1));
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("water_mask"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: MASK_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Self {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            texture,
            size,
        }
    }

    /// Recreates the mask when the target size changed. Returns whether it
    /// did, so the bind group holding the view can be rebuilt.
    pub fn ensure_size(&mut self, device: &wgpu::Device, width: u32, height: u32) -> bool {
        let wanted = (width.max(1), height.max(1));
        if wanted == self.size {
            return false;
        }
        *self = Self::new(device, wanted.0, wanted.1);
        true
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }
}

/// Format of the underwater surface depth: the raw reverse-Z depth value as a
/// colour, because the live depth buffer feeds the Hi-Z pyramid and must not
/// see the water surface from below.
pub const SURFACE_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// The depth of the underwater water surfaces. Cleared to 0 (reverse-Z far:
/// no surface), then every underwater surface that passes the depth test
/// against the opaque scene writes its depth. Combined with the scene depth by
/// taking the nearer value, it gives the depth buffer the underwater fog needs.
pub struct WaterSurfaceDepth {
    view: wgpu::TextureView,
    size: (u32, u32),
}

impl WaterSurfaceDepth {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let size = (width.max(1), height.max(1));
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("water_surface_depth"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SURFACE_DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        Self {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            size,
        }
    }

    /// Recreates the texture when the target size changed; returns whether it
    /// did.
    pub fn ensure_size(&mut self, device: &wgpu::Device, width: u32, height: u32) -> bool {
        let wanted = (width.max(1), height.max(1));
        if wanted == self.size {
            return false;
        }
        *self = Self::new(device, wanted.0, wanted.1);
        true
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }
}

/// Normal-layer textures by asset path, loaded on first use. A texture that
/// cannot be read is reported once and replaced by a flat normal, so one bad
/// path costs that layer's detail, not the whole water.
pub struct NormalTextures {
    asset_root: String,
    loaded: HashMap<String, Option<wgpu::TextureView>>,
    flat: wgpu::TextureView,
}

impl NormalTextures {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, asset_root: &str) -> Self {
        Self {
            asset_root: asset_root.to_owned(),
            loaded: HashMap::new(),
            flat: texel_texture(
                device,
                queue,
                "water_flat_normal",
                wgpu::TextureFormat::Rg8Unorm,
                &FLAT_NORMAL_TEXEL,
            ),
        }
    }

    pub fn view(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        plugin_path: Option<&str>,
    ) -> wgpu::TextureView {
        let Some(plugin_path) = plugin_path.filter(|path| !path.trim().is_empty()) else {
            return self.flat.clone();
        };
        let path = asset_path(plugin_path);
        let entry = self.loaded.entry(path.clone()).or_insert_with(|| {
            match crate::rendering::texture::load_dds_view(device, queue, &self.asset_root, &path) {
                Ok(view) => Some(view),
                Err(error) => {
                    warn_once(&format!("water-texture:{path}"), || {
                        format!(
                            "water normal texture {path} unusable, using a flat normal: {error}"
                        )
                    });
                    None
                }
            }
        });
        entry.clone().unwrap_or_else(|| self.flat.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_paths_lose_their_data_prefix_in_any_case() {
        assert_eq!(
            asset_path("data\\Textures\\Water\\Wave_n.dds"),
            "textures\\Water\\Wave_n.dds"
        );
        assert_eq!(
            asset_path("Data\\Textures\\Water\\Wave_n.dds"),
            "textures\\Water\\Wave_n.dds"
        );
    }

    #[test]
    fn paths_without_the_prefix_and_with_forward_slashes_are_normalised() {
        assert_eq!(
            asset_path("Textures/Water/Wave_n.dds"),
            "textures\\Water\\Wave_n.dds"
        );
    }

    #[test]
    fn the_textures_root_takes_the_layout_case_whatever_the_plugin_wrote() {
        for plugin_path in [
            "data\\TEXTURES\\Water\\x.dds",
            "data\\textures\\Water\\x.dds",
        ] {
            assert_eq!(asset_path(plugin_path), "textures\\Water\\x.dds");
        }
    }

    #[test]
    fn a_folder_that_merely_starts_with_textures_is_not_the_root() {
        assert_eq!(
            asset_path("data\\texturesextra\\x.dds"),
            "texturesextra\\x.dds"
        );
    }

    #[test]
    fn short_or_non_ascii_paths_do_not_panic() {
        assert_eq!(asset_path("da"), "da");
        assert_eq!(asset_path("dätä\\x.dds"), "dätä\\x.dds");
    }
}
