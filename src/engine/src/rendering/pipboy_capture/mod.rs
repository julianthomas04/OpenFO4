//! Dev-only screenshot mode: `OPENFO4_PIPBOY_CAPTURE=<dir>` photographs the main menu and the HUD,
//! walks the Pip-Boy through a fixed list of screens and writes one PNG per screen, then quits.

mod config;
mod driver;
mod readback;
mod sequencer;
mod states;

use std::path::PathBuf;

pub use config::{CaptureConfig, CaptureConfigError, DIR_ENV as CAPTURE_DIR_ENV};

use self::sequencer::{CaptureEvent, Sequencer};
use self::states::{CaptureAction, Scene, capture_states};

/// What the engine has to do for the capture this frame.
pub enum CaptureStep {
    Apply(Vec<CaptureAction>),
    Finished,
}

/// The capture session: where it writes and how far it has come.
pub struct PipBoyCapture {
    directory: PathBuf,
    sequencer: Sequencer,
    pending: Option<PathBuf>,
    written: usize,
}

impl PipBoyCapture {
    pub fn new(config: &CaptureConfig) -> std::io::Result<Self> {
        std::fs::create_dir_all(&config.directory)?;
        Ok(Self {
            directory: config.directory.clone(),
            sequencer: Sequencer::new(capture_states()),
            pending: None,
            written: 0,
        })
    }

    fn awaited_scene(&self) -> Option<Scene> {
        self.sequencer.awaited_scene()
    }

    fn advance(&mut self, dt: f32) -> Option<CaptureStep> {
        match self.sequencer.advance(dt)? {
            CaptureEvent::Apply(state) => Some(CaptureStep::Apply(state.actions.clone())),
            CaptureEvent::Capture(state) => {
                self.pending = Some(self.directory.join(state.file_name()));
                None
            }
            CaptureEvent::Finished => Some(CaptureStep::Finished),
        }
    }
}
