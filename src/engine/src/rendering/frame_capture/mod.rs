//! What the screenshot modes share: the window size they render at and writing one finished
//! frame to a PNG file.

mod readback;
mod size;

use std::path::PathBuf;

use readback::{FrameReadback, ReadbackError};
pub use size::{DEFAULT_CAPTURE_SIZE, InvalidCaptureSize, SIZE_ENV, capture_size, size_from_env};

#[derive(Debug, thiserror::Error)]
pub enum FrameCaptureError {
    #[error("cannot record the copy of the frame for {path}: {source}")]
    Record {
        path: PathBuf,
        source: ReadbackError,
    },
    #[error("cannot read back the frame for {path}: {source}")]
    Read {
        path: PathBuf,
        source: ReadbackError,
    },
    #[error("cannot write {path}: {source}")]
    Save {
        path: PathBuf,
        source: image::ImageError,
    },
}

/// A frame whose copy is recorded and whose file is yet to be written.
pub struct PendingFrame {
    readback: FrameReadback,
    path: PathBuf,
}

impl PendingFrame {
    /// Records the copy of `frame` (which needs `COPY_SRC`) into `encoder`, to be written to
    /// `path` once the encoder is submitted.
    pub fn record(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::Texture,
        path: PathBuf,
    ) -> Result<Self, FrameCaptureError> {
        match FrameReadback::encode(device, encoder, frame) {
            Ok(readback) => Ok(Self { readback, path }),
            Err(source) => Err(FrameCaptureError::Record { path, source }),
        }
    }

    /// Waits for the submitted copy and writes it; returns the path written.
    pub fn write(self, device: &wgpu::Device) -> Result<PathBuf, FrameCaptureError> {
        let Self { readback, path } = self;
        let image = match readback.finish(device) {
            Ok(image) => image,
            Err(source) => return Err(FrameCaptureError::Read { path, source }),
        };
        match image.save(&path) {
            Ok(()) => Ok(path),
            Err(source) => Err(FrameCaptureError::Save { path, source }),
        }
    }
}
