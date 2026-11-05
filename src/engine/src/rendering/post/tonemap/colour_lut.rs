//! A colour lookup table on the GPU: a 16 x 16 x 16 texture, filtered trilinearly between its
//! grid points.

use super::lut_table::{LUT_SIZE, LutTable, blended_texels};

const LUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

pub(crate) struct ColourLut {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
}

impl ColourLut {
    /// The table that leaves every colour as it is, until [`Self::write`].
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, label: &str) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: LUT_SIZE,
                height: LUT_SIZE,
                depth_or_array_layers: LUT_SIZE,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: LUT_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(label),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let lut = Self {
            texture,
            view,
            sampler,
        };
        lut.write(
            queue,
            &blended_texels(&[(Some(&LutTable::identity()), 1.0)]),
        );
        lut
    }

    /// `texels` as [`blended_texels`] lays them out.
    pub fn write(&self, queue: &wgpu::Queue, texels: &[[u8; 4]]) {
        queue.write_texture(
            self.texture.as_image_copy(),
            bytemuck::cast_slice(texels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(LUT_SIZE * 4),
                rows_per_image: Some(LUT_SIZE),
            },
            self.texture.size(),
        );
    }
}
