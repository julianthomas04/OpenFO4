//! The engine side of the capture mode: drives the Pip-Boy and reads the frame back.

use std::path::{Path, PathBuf};

use super::readback::{FrameReadback, ReadbackError};
use super::states::{CaptureAction, Scene};
use super::{CaptureStep, PipBoyCapture};
use crate::game_flow::Phase;
use crate::rendering::State;
use crate::sim::SimCommand;
use crate::sim::pipboy::PipBoyAction;

#[derive(Debug, thiserror::Error)]
enum CaptureError {
    #[error(transparent)]
    Readback(#[from] ReadbackError),
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

impl State {
    pub(in crate::rendering) fn capture_active(&self) -> bool {
        self.capture.is_some()
    }

    /// Runs the next capture step once the engine shows what the awaited state needs.
    pub(in crate::rendering) fn drive_pipboy_capture(&mut self, dt: f32) {
        let Some(scene) = self.capture.as_ref().and_then(PipBoyCapture::awaited_scene) else {
            self.advance_capture(dt);
            return;
        };
        if self.prepare_scene(scene) {
            self.advance_capture(dt);
        }
    }

    /// Moves the engine towards `scene`; true once it is there.
    fn prepare_scene(&mut self, scene: Scene) -> bool {
        match scene {
            Scene::MainMenu => self.flow.phase() == Phase::MainMenu,
            Scene::Hud | Scene::PipBoy if self.flow.phase() == Phase::MainMenu => {
                self.flow.release_to_loading();
                false
            }
            Scene::Hud => self.world_is_shown() && !self.sim.pipboy_snapshot().model.open,
            Scene::PipBoy => {
                if !self.world_is_shown() {
                    return false;
                }
                if self.sim.pipboy_snapshot().model.open {
                    return true;
                }
                self.sim.apply(SimCommand::PipBoy(PipBoyAction::Open));
                false
            }
        }
    }

    fn world_is_shown(&self) -> bool {
        self.flow.phase() == Phase::InGame
            && self.flow.overlay_alpha() == 0.0
            && self.sim.player_eye().is_some()
    }

    fn advance_capture(&mut self, dt: f32) {
        let Some(capture) = self.capture.as_mut() else {
            return;
        };
        match capture.advance(dt) {
            Some(CaptureStep::Apply(actions)) => self.apply_capture_actions(&actions),
            Some(CaptureStep::Finished) => {
                tracing::info!(
                    "pipboy capture: wrote {} screenshots to {}",
                    capture.written,
                    capture.directory.display()
                );
                self.quit_requested = true;
            }
            None => {}
        }
    }

    fn apply_capture_actions(&mut self, actions: &[CaptureAction]) {
        for action in actions {
            match *action {
                CaptureAction::Show { screen, select_row } => {
                    self.pipboy.screen_mut().show(screen, select_row);
                }
                CaptureAction::Input(input) => {
                    let snapshot = self.sim.pipboy_snapshot();
                    self.pipboy.screen_mut().dispatch(input, &snapshot);
                }
            }
        }
        self.flush_pipboy_actions();
    }

    /// Records the copy of the finished frame when this frame is the one to photograph.
    pub(in crate::rendering) fn begin_frame_capture(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::Texture,
    ) -> Option<PendingFrame> {
        let path = self.capture.as_mut()?.pending.take()?;
        match FrameReadback::encode(&self.device, encoder, frame) {
            Ok(readback) => Some(PendingFrame { readback, path }),
            Err(error) => {
                self.abort_capture(&path, &error.into());
                None
            }
        }
    }

    /// Writes the photographed frame once its copy has been submitted.
    pub(in crate::rendering) fn finish_frame_capture(&mut self, pending: Option<PendingFrame>) {
        let Some(PendingFrame { readback, path }) = pending else {
            return;
        };
        let written = readback
            .finish(&self.device)
            .map_err(CaptureError::from)
            .and_then(|image| {
                image.save(&path).map_err(|source| CaptureError::Save {
                    path: path.clone(),
                    source,
                })
            });
        match written {
            Ok(()) => {
                if let Some(capture) = self.capture.as_mut() {
                    capture.written += 1;
                }
                tracing::info!("pipboy capture: wrote {}", path.display());
            }
            Err(error) => self.abort_capture(&path, &error),
        }
    }

    fn abort_capture(&mut self, path: &Path, error: &CaptureError) {
        tracing::error!("pipboy capture of {} failed: {error}", path.display());
        self.quit_requested = true;
    }
}
