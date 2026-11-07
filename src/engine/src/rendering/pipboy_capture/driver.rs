//! The engine side of the capture mode: drives the Pip-Boy and reads the frame back.

use super::states::{CaptureAction, Scene};
use super::{CaptureStep, PipBoyCapture};
use crate::game_flow::Phase;
use crate::rendering::State;
use crate::rendering::frame_capture::{FrameCaptureError, PendingFrame};
use crate::sim::SimCommand;
use crate::sim::pipboy::PipBoyAction;

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
        PendingFrame::record(&self.device, encoder, frame, path)
            .inspect_err(|error| self.abort_capture(error))
            .ok()
    }

    /// Writes the photographed frame once its copy has been submitted.
    pub(in crate::rendering) fn finish_frame_capture(&mut self, pending: Option<PendingFrame>) {
        let Some(pending) = pending else {
            return;
        };
        match pending.write(&self.device) {
            Ok(path) => {
                if let Some(capture) = self.capture.as_mut() {
                    capture.written += 1;
                }
                tracing::info!("pipboy capture: wrote {}", path.display());
            }
            Err(error) => self.abort_capture(&error),
        }
    }

    fn abort_capture(&mut self, error: &FrameCaptureError) {
        tracing::error!("pipboy capture failed: {error}");
        self.quit_requested = true;
    }
}
