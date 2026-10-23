//! Copies a rendered frame back to the CPU as an RGBA image.

use image::RgbaImage;

#[derive(Debug, thiserror::Error)]
pub enum ReadbackError {
    #[error("surface format {0:?} cannot be written as a PNG; expected an 8-bit RGBA or BGRA one")]
    UnsupportedFormat(wgpu::TextureFormat),
    #[error("the frame readback failed to map: {0}")]
    Map(#[from] wgpu::BufferAsyncError),
    #[error("the frame readback mapped range is unavailable: {0}")]
    MappedRange(#[from] wgpu::MapRangeError),
    #[error("the frame readback poll failed: {0}")]
    Poll(#[from] wgpu::PollError),
    #[error("the frame readback callback was dropped without a result")]
    CallbackDropped,
}

const BYTES_PER_PIXEL: u32 = 4;

/// Whether the format stores its channels as blue, green, red, alpha.
fn channel_order(format: wgpu::TextureFormat) -> Result<bool, ReadbackError> {
    match format {
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => Ok(false),
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => Ok(true),
        other => Err(ReadbackError::UnsupportedFormat(other)),
    }
}

fn padded_row_bytes(width: u32) -> u32 {
    (width * BYTES_PER_PIXEL).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
}

/// Drops the row padding of a mapped copy and returns tightly packed RGBA pixels.
fn pixels_from_rows(mapped: &[u8], width: u32, height: u32, bgra: bool) -> Vec<u8> {
    let row_bytes = (width * BYTES_PER_PIXEL) as usize;
    let mut pixels = Vec::with_capacity(row_bytes * height as usize);
    for row in mapped
        .chunks_exact(padded_row_bytes(width) as usize)
        .take(height as usize)
    {
        pixels.extend_from_slice(&row[..row_bytes]);
    }
    if bgra {
        for texel in pixels.chunks_exact_mut(BYTES_PER_PIXEL as usize) {
            texel.swap(0, 2);
        }
    }
    pixels
}

/// A texture copy recorded into a command encoder and read once the encoder is submitted.
pub struct FrameReadback {
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    bgra: bool,
}

impl FrameReadback {
    /// Records the copy of `texture` (which needs `COPY_SRC`) into `encoder`.
    pub fn encode(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) -> Result<Self, ReadbackError> {
        let bgra = channel_order(texture.format())?;
        let (width, height) = (texture.width(), texture.height());
        let bytes_per_row = padded_row_bytes(width);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pipboy_capture_readback"),
            size: u64::from(bytes_per_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: None,
                },
            },
            texture.size(),
        );
        Ok(Self {
            buffer,
            width,
            height,
            bgra,
        })
    }

    /// Waits for the submitted copy and returns the frame.
    pub fn finish(self, device: &wgpu::Device) -> Result<RgbaImage, ReadbackError> {
        let (sender, receiver) = std::sync::mpsc::channel();
        self.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                // The receiver lives until the poll below returns; a failed send means it is gone.
                let _ = sender.send(result);
            });
        device.poll(wgpu::PollType::wait_indefinitely())?;
        receiver
            .recv()
            .map_err(|_| ReadbackError::CallbackDropped)??;
        let pixels = {
            let mapped = self.buffer.slice(..).get_mapped_range()?;
            pixels_from_rows(&mapped, self.width, self.height, self.bgra)
        };
        self.buffer.unmap();
        Ok(RgbaImage::from_raw(self.width, self.height, pixels)
            .expect("the pixel buffer is exactly width * height * 4 bytes"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_padded_to_the_copy_alignment() {
        assert_eq!(padded_row_bytes(1920), 7680);
        assert_eq!(padded_row_bytes(65), 512);
        assert_eq!(padded_row_bytes(64), 256);
    }

    #[test]
    fn padding_is_dropped_and_bgra_becomes_rgba() {
        let (width, height) = (2, 2);
        let stride = padded_row_bytes(width) as usize;
        let mut mapped = vec![0xEE; stride * height as usize];
        mapped[..8].copy_from_slice(&[3, 2, 1, 255, 6, 5, 4, 255]);
        mapped[stride..stride + 8].copy_from_slice(&[9, 8, 7, 255, 12, 11, 10, 255]);
        assert_eq!(
            pixels_from_rows(&mapped, width, height, true),
            vec![1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255]
        );
        assert_eq!(
            pixels_from_rows(&mapped, width, height, false)[..4],
            [3, 2, 1, 255]
        );
    }

    #[test]
    fn only_eight_bit_rgba_and_bgra_formats_are_supported() {
        assert!(!channel_order(wgpu::TextureFormat::Rgba8UnormSrgb).unwrap());
        assert!(channel_order(wgpu::TextureFormat::Bgra8UnormSrgb).unwrap());
        assert!(matches!(
            channel_order(wgpu::TextureFormat::Rgba16Float),
            Err(ReadbackError::UnsupportedFormat(_))
        ));
    }
}
