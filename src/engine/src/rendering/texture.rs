use image::GenericImageView;

pub struct Texture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
}

impl Texture {
    pub fn from_bytes(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bytes: &[u8],
        label: &str,
    ) -> anyhow::Result<Self> {
        let img = image::load_from_memory(bytes)?;
        Self::from_image(device, queue, &img, Some(label))
    }

    pub fn from_image(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        img: &image::DynamicImage,
        label: Option<&str>,
    ) -> anyhow::Result<Self> {
        let rgba = img.to_rgba8();
        let dimensions = img.dimensions();

        let size = wgpu::Extent3d {
            width: dimensions.0,
            height: dimensions.1,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                aspect: wgpu::TextureAspect::All,
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * dimensions.0),
                rows_per_image: Some(dimensions.1),
            },
            size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        Ok(Self {
            texture,
            view,
            sampler,
        })
    }
}

impl Texture {
    pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

    pub fn create_depth_texture(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
        label: &str,
    ) -> Self {
        let size = wgpu::Extent3d {
            width: config.width.max(1),
            height: config.height.max(1),
            depth_or_array_layers: 1,
        };
        let desc = wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let texture = device.create_texture(&desc);

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        // Comparison direction must match the reverse-Z pipelines: a fragment
        // is lit/visible when its depth is >= the stored depth.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::GreaterEqual),
            lod_min_clamp: 0.0,
            lod_max_clamp: 100.0,
            ..Default::default()
        });

        Self {
            texture,
            view,
            sampler,
        }
    }
}

/// Loads the DDS file `path` below `asset_root` with every mip into a sampled texture view in the
/// file's own format (gamma-encoded colour stays as stored).
pub fn load_dds_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    asset_root: &str,
    path: &str,
) -> Result<wgpu::TextureView, crate::assets::types::texture::TextureError> {
    load_dds_view_as(device, queue, asset_root, path, false)
}

/// Like [`load_dds_view`], but the view decodes sRGB-encoded colour to linear when sampled.
pub fn load_dds_view_srgb(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    asset_root: &str,
    path: &str,
) -> Result<wgpu::TextureView, crate::assets::types::texture::TextureError> {
    load_dds_view_as(device, queue, asset_root, path, true)
}

fn load_dds_view_as(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    asset_root: &str,
    path: &str,
    srgb: bool,
) -> Result<wgpu::TextureView, crate::assets::types::texture::TextureError> {
    use crate::assets::types::texture;
    let full_path = format!("{asset_root}/{path}");
    let layout = std::sync::Arc::new(texture::read_layout(&full_path)?);
    let payload = texture::read_mips(&full_path, &layout, 0..layout.mip_count)?;
    let gpu = texture::create_texture(device, &layout, 0, srgb, path);
    texture::write_payload(queue, &gpu, 0, &payload)?;
    Ok(gpu.create_view(&wgpu::TextureViewDescriptor::default()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::gpu_test::test_device;

    fn write_dds(dir: &std::path::Path) -> &'static str {
        const NAME: &str = "texel.dds";
        let mut dds = ddsfile::Dds::new_dxgi(ddsfile::NewDxgiParams {
            height: 4,
            width: 4,
            depth: None,
            format: ddsfile::DxgiFormat::R8G8B8A8_UNorm,
            mipmap_levels: Some(1),
            array_layers: None,
            caps2: None,
            is_cubemap: false,
            resource_dimension: ddsfile::D3D10ResourceDimension::Texture2D,
            alpha_mode: ddsfile::AlphaMode::Unknown,
        })
        .unwrap();
        dds.data.fill(200);
        let mut file = std::fs::File::create(dir.join(NAME)).unwrap();
        dds.write(&mut file).unwrap();
        NAME
    }

    #[test]
    fn the_srgb_loader_decodes_colour_and_the_plain_loader_keeps_it_as_stored() {
        let Some((device, queue)) =
            test_device("the_srgb_loader_decodes_colour_and_the_plain_loader_keeps_it_as_stored")
        else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let name = write_dds(dir.path());
        let root = dir.path().to_str().unwrap();

        let plain = load_dds_view(&device, &queue, root, name).unwrap();
        let srgb = load_dds_view_srgb(&device, &queue, root, name).unwrap();

        assert_eq!(plain.texture().format(), wgpu::TextureFormat::Rgba8Unorm);
        assert_eq!(srgb.texture().format(), wgpu::TextureFormat::Rgba8UnormSrgb);
    }
}
