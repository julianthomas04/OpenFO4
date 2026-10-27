//! Dev-only screenshot mode: `OPENFO4_SHOT=<file.png>` starts in the world with the camera parked
//! at the spawn, waits until the world around it is ready, writes one frame without any
//! interface and quits.

mod config;
mod driver;
mod gate;

use std::time::{Duration, Instant};

use winit::dpi::PhysicalSize;

pub use config::{SHOT_ENV, ShotConfig, ShotConfigError, shot_requested};
pub use gate::Blocker;

use crate::rendering::frame_capture::FrameCaptureError;
use gate::ShotGate;

/// A failure that ends the shot and the game with an error.
#[derive(Debug, thiserror::Error)]
pub enum ShotError {
    #[error("the world was not ready after {}s: {blocker}", .after.as_secs())]
    TimedOut { blocker: Blocker, after: Duration },
    #[error(
        "the frame is {}x{} but {}x{} was asked for; the window could not take the capture size",
        .actual.width, .actual.height, .expected.width, .expected.height
    )]
    SizeMismatch {
        expected: PhysicalSize<u32>,
        actual: PhysicalSize<u32>,
    },
    #[error(transparent)]
    Capture(#[from] FrameCaptureError),
}

/// Where the shot stands.
enum Progress {
    Waiting,
    /// The next rendered frame is the one to write.
    Due,
    /// The frame is written; the game is quitting.
    Written,
    /// The shot failed while rendering; the next update ends the game with it.
    Failed(ShotError),
    /// The failure has been handed to the event loop.
    Reported,
}

pub struct ShotSession {
    config: ShotConfig,
    gate: ShotGate,
    started: Instant,
    progress: Progress,
}

impl ShotSession {
    pub fn new(config: ShotConfig) -> Self {
        Self {
            gate: ShotGate::new(config.settle_frames),
            config,
            started: Instant::now(),
            progress: Progress::Waiting,
        }
    }

    /// The failure of the render side, handed out once.
    fn take_failure(&mut self) -> Option<ShotError> {
        match std::mem::replace(&mut self.progress, Progress::Reported) {
            Progress::Failed(error) => Some(error),
            other => {
                self.progress = other;
                None
            }
        }
    }
}
