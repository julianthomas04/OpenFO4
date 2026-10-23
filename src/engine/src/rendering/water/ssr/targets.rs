//! The render targets and the Hi-Z pyramid of the SSR chain: ray
//! anchor uv, view z, raytrace result, blur scratch, blurred result, Hi-Z.

use glam::UVec2;

use crate::rendering::water::textures::ReflectionViews;

/// The ray anchor uv, `R16G16_FLOAT` at half resolution.
pub const ANCHOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
/// View-space z of the reflecting pixel, `R32_FLOAT` at half resolution.
pub const VIEW_Z_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
/// Raytrace result, blur scratch and blurred result: `RGBA8_UNORM_SRGB` at half resolution.
pub const REFLECTION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// Hi-Z pyramid: `R32_FLOAT`.
pub const HIZ_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// Mips of the Hi-Z pyramid: mip 0 is the full-resolution linear depth, mips 1 to 4 are the
/// pyramid the raytracer reads. Texture mip `n` is pyramid level `n`, because the translated
/// shaders address the pyramid by mip number.
///
/// The translated camera pass stores mip 0, so the full-resolution
/// `R32Float` write (4 bytes per pixel, about 8 MB per frame at 1080p) is paid every frame the
/// chain runs. The raytracer fetches the pyramid at a computed level, so nothing proves that
/// level 0 is never read; the write is kept rather than bound to a throwaway view.
pub const HIZ_MIP_COUNT: u32 = 5;
/// The smallest half-resolution extent that holds every Hi-Z mip; a guard for
/// degenerate target sizes (no valid render is smaller).
const MIN_HALF_EXTENT: u32 = 1 << (HIZ_MIP_COUNT - 1);

fn extent(size: UVec2) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: size.x,
        height: size.y,
        depth_or_array_layers: 1,
    }
}

fn target(
    device: &wgpu::Device,
    label: &str,
    size: UVec2,
    format: wgpu::TextureFormat,
    mip_level_count: u32,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: extent(size),
        mip_level_count,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

/// Copies let the tests (and captures) seed and read every target.
const COPIES: wgpu::TextureUsages =
    wgpu::TextureUsages::COPY_SRC.union(wgpu::TextureUsages::COPY_DST);
const RENDER_AND_SAMPLE: wgpu::TextureUsages = wgpu::TextureUsages::RENDER_ATTACHMENT
    .union(wgpu::TextureUsages::TEXTURE_BINDING)
    .union(COPIES);

/// The extent of the Hi-Z texture (pyramid level 0): twice the half-resolution extent.
pub fn hiz_extent(half: UVec2) -> UVec2 {
    half.max(UVec2::splat(MIN_HALF_EXTENT)) << 1
}

/// The extent of pyramid level `level` of the Hi-Z pyramid (`W >> level`, at least 1).
pub fn hiz_level_extent(half: UVec2, level: u32) -> UVec2 {
    (hiz_extent(half) >> level).max(UVec2::ONE)
}

/// The textures behind [`SsrTargets`]' views.
pub struct SsrTextures {
    pub anchor: wgpu::Texture,
    pub view_z: wgpu::Texture,
    pub sharp: wgpu::Texture,
    pub scratch: wgpu::Texture,
    pub blurred: wgpu::Texture,
    pub hiz: wgpu::Texture,
}

/// One set of SSR targets for a given half-resolution size.
pub struct SsrTargets {
    half: UVec2,
    pub textures: SsrTextures,
    pub anchor: wgpu::TextureView,
    pub view_z: wgpu::TextureView,
    pub sharp: wgpu::TextureView,
    pub scratch: wgpu::TextureView,
    pub blurred: wgpu::TextureView,
    /// The whole pyramid, for sampling.
    pub hiz: wgpu::TextureView,
    /// One single-mip view per pyramid level, for the compute passes.
    pub hiz_levels: Vec<wgpu::TextureView>,
}

impl SsrTargets {
    pub fn new(device: &wgpu::Device, half: UVec2) -> Self {
        let view =
            |texture: &wgpu::Texture| texture.create_view(&wgpu::TextureViewDescriptor::default());
        let hiz = target(
            device,
            "water_ssr_hiz",
            hiz_extent(half),
            HIZ_FORMAT,
            HIZ_MIP_COUNT,
            wgpu::TextureUsages::STORAGE_BINDING
                .union(wgpu::TextureUsages::TEXTURE_BINDING)
                .union(COPIES),
        );
        let hiz_levels = (0..HIZ_MIP_COUNT)
            .map(|mip| {
                hiz.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("water_ssr_hiz_level"),
                    base_mip_level: mip,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let half_target = |label, format| target(device, label, half, format, 1, RENDER_AND_SAMPLE);
        let textures = SsrTextures {
            anchor: half_target("water_ssr_anchor", ANCHOR_FORMAT),
            view_z: half_target("water_ssr_view_z", VIEW_Z_FORMAT),
            sharp: half_target("water_ssr_sharp", REFLECTION_FORMAT),
            scratch: half_target("water_ssr_blur_scratch", REFLECTION_FORMAT),
            blurred: half_target("water_ssr_blurred", REFLECTION_FORMAT),
            hiz,
        };
        Self {
            half,
            anchor: view(&textures.anchor),
            view_z: view(&textures.view_z),
            sharp: view(&textures.sharp),
            scratch: view(&textures.scratch),
            blurred: view(&textures.blurred),
            hiz: view(&textures.hiz),
            hiz_levels,
            textures,
        }
    }

    pub fn half(&self) -> UVec2 {
        self.half
    }

    /// The sharp and blurred reflections as the water shader's `t9` and `t10`.
    pub fn reflection_views(&self) -> ReflectionViews {
        ReflectionViews {
            sharp: self.sharp.clone(),
            blurred: self.blurred.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hiz_levels_halve_from_the_half_resolution_target() {
        let half = UVec2::new(960, 540);
        assert_eq!(hiz_level_extent(half, 0), half * 2);
        assert_eq!(hiz_level_extent(half, 1), half);
        assert_eq!(hiz_level_extent(half, 2), UVec2::new(480, 270));
        assert_eq!(hiz_level_extent(half, 3), UVec2::new(240, 135));
        assert_eq!(hiz_level_extent(half, 4), UVec2::new(120, 67));
    }

    #[test]
    fn the_coarsest_level_matches_the_raytracers_start_grid() {
        // The raytracer starts at `floor(size * 0.125)` cells on level 4.
        for (w, h) in [(1920_u32, 1080_u32), (2561, 1441), (1000, 700)] {
            let half = UVec2::new(w >> 1, h >> 1);
            let grid = (half.as_vec2() * 0.125).floor().as_uvec2();
            assert_eq!(hiz_level_extent(half, 4), grid, "{w}x{h}");
        }
    }

    #[test]
    fn a_degenerate_target_still_holds_every_mip() {
        assert_eq!(
            hiz_level_extent(UVec2::new(2, 3), HIZ_MIP_COUNT - 1),
            UVec2::splat(2)
        );
    }
}
