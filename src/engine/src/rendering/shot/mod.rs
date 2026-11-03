//! Dev-only screenshot mode: `OPENFO4_SHOT=<file.png>` starts in the world with the camera parked
//! at the spawn, waits until the world around it is ready, writes one frame without any
//! interface and quits. With `OPENFO4_SHOT_LOADSCREEN=<editor id or form key>` as well, it then
//! opens that loading screen over the world with its camera pinned, waits until its background,
//! model and panel are shown, and photographs that instead.

mod config;
mod driver;
mod gate;

use std::time::{Duration, Instant};

use winit::dpi::PhysicalSize;

pub use config::{
    SHOT_ENV, SHOT_LOADING_SCREEN_ENV, ShotConfig, ShotConfigError, shot_requested,
};
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
    #[error("cannot show the loading screen: {0}")]
    LoadingScreen(String),
    #[error(transparent)]
    Capture(#[from] FrameCaptureError),
}

/// How far the shot is with the loading screen it was asked to photograph.
#[derive(Debug, PartialEq, Eq)]
enum LoadingScreenStage {
    /// The world is photographed.
    NotWanted,
    /// To be opened once the world is ready.
    Wanted(String),
    Opened,
}

impl LoadingScreenStage {
    /// The screen to open now, which moves the stage on to `Opened`.
    fn take_wanted(&mut self) -> Option<String> {
        match std::mem::replace(self, Self::Opened) {
            Self::Wanted(name) => Some(name),
            other => {
                *self = other;
                None
            }
        }
    }
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
    loading_screen: LoadingScreenStage,
    /// When the loading screen was opened, which its own deadline counts from.
    loading_screen_opened: Option<Instant>,
    started: Instant,
    progress: Progress,
}

impl ShotSession {
    pub fn new(config: ShotConfig) -> Self {
        let loading_screen = config
            .loading_screen
            .clone()
            .map_or(LoadingScreenStage::NotWanted, LoadingScreenStage::Wanted);
        Self {
            gate: ShotGate::new(config.settle_frames),
            loading_screen,
            loading_screen_opened: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wanted_screen_is_taken_once_and_leaves_the_stage_opened() {
        let mut stage = LoadingScreenStage::Wanted("LSDog".to_owned());
        assert_eq!(stage.take_wanted().as_deref(), Some("LSDog"));
        assert_eq!(stage, LoadingScreenStage::Opened);
        assert_eq!(stage.take_wanted(), None);
        assert_eq!(stage, LoadingScreenStage::Opened);
    }

    #[test]
    fn a_shot_of_the_world_wants_no_screen() {
        let mut stage = LoadingScreenStage::NotWanted;
        assert_eq!(stage.take_wanted(), None);
        assert_eq!(stage, LoadingScreenStage::NotWanted);
    }
}
