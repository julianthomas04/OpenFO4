//! Fixtures for the audio tests: a FLAC writer, a peak meter and a driver for kira's mock
//! backend. Shared with the integration tests through the `test-support` feature.

mod audio_files;
pub mod catalog;
pub mod condition;
mod flac;
pub mod footsteps;
mod log_capture;
mod meter;
mod mock_audio;
pub mod weather;

pub use audio_files::write_audio_file;
pub use flac::{sine_flac, undecodable_flac};
pub use log_capture::SharedBuffer;
pub use meter::{PeakMeterBuilder, PeakReader, peak_meter};
pub use mock_audio::{
    MockAudio, mock_manager_settings, render, render_buffer, render_manager, render_manager_buffer,
    wait_until,
};

pub const SAMPLE_RATE: u32 = 8000;
/// Half a second of audio at `SAMPLE_RATE`.
pub const HALF_SECOND_SAMPLES: u32 = 4000;
/// Buffers rendered after a fade-in so the meter reads the settled level.
pub const SETTLED_BUFFERS: usize = 8;
pub const SEED: u64 = 7;
