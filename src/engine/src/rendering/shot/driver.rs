//! The engine side of the shot: reads how ready the world is and writes the frame.

use std::time::Instant;

use winit::dpi::PhysicalSize;

use super::gate::{ShotFrame, ShotGate, ShotStep};
use super::{LoadingScreenStage, Progress, ShotError};
use crate::game_flow::Phase;
use crate::rendering::State;
use crate::rendering::frame_capture::PendingFrame;
use crate::rendering::load_transition::ScreenRequest;

impl State {
    pub(in crate::rendering) fn shot_active(&self) -> bool {
        self.shot.is_some()
    }

    /// Feeds this frame's state of the world to the shot; an error ends the game.
    pub(in crate::rendering) fn drive_shot(&mut self) -> Result<(), ShotError> {
        let loading_screen_pending = self.shot_loading_screen_pending();
        let frame = self.shot_frame(loading_screen_pending);
        let Some(shot) = self.shot.as_mut() else {
            return Ok(());
        };
        if let Some(error) = shot.take_failure() {
            return Err(error);
        }
        if !matches!(shot.progress, Progress::Waiting) {
            return Ok(());
        }
        if shot.loading_screen == LoadingScreenStage::Opened
            && !self.load_screens.preview.is_open()
        {
            let reason = self
                .load_screens
                .preview
                .error
                .clone()
                .unwrap_or_else(|| "it was closed".to_owned());
            return self.fail_shot(ShotError::LoadingScreen(reason));
        }
        let Some(shot) = self.shot.as_mut() else {
            return Ok(());
        };
        let after = shot
            .loading_screen_opened
            .unwrap_or(shot.started)
            .elapsed();
        match shot.gate.step(&frame, after) {
            ShotStep::Wait => Ok(()),
            ShotStep::Capture => {
                tracing::info!("shot: the world is ready after {:.1}s", after.as_secs_f32());
                self.begin_loading_screen_or_capture()
            }
            ShotStep::TimedOut(blocker) => {
                shot.progress = Progress::Reported;
                Err(ShotError::TimedOut { blocker, after })
            }
        }
    }

    /// Opens the loading screen the shot was asked for, to be photographed once it is all shown;
    /// without one the frame is due.
    fn begin_loading_screen_or_capture(&mut self) -> Result<(), ShotError> {
        let Some(shot) = self.shot.as_mut() else {
            return Ok(());
        };
        let Some(name) = shot.loading_screen.take_wanted() else {
            shot.progress = Progress::Due;
            return Ok(());
        };
        let settle_frames = shot.config.settle_frames;
        if let Err(error) = self.open_loading_screen_preview(&ScreenRequest::Named(name), true) {
            return self.fail_shot(ShotError::LoadingScreen(error.to_string()));
        }
        if let Some(shot) = self.shot.as_mut() {
            shot.gate = ShotGate::for_loading_screen(settle_frames);
            shot.loading_screen_opened = Some(Instant::now());
        }
        Ok(())
    }

    fn fail_shot(&mut self, error: ShotError) -> Result<(), ShotError> {
        if let Some(shot) = self.shot.as_mut() {
            shot.progress = Progress::Reported;
        }
        Err(error)
    }

    /// Whether the loading screen being photographed is not yet all shown.
    fn shot_loading_screen_pending(&mut self) -> bool {
        let opened = self
            .shot
            .as_ref()
            .is_some_and(|shot| shot.loading_screen == LoadingScreenStage::Opened);
        opened && !self.loading_screen_fully_shown()
    }

    fn shot_frame(&self, loading_screen_pending: bool) -> ShotFrame {
        ShotFrame {
            loading_screen_pending,
            world_shown: self.flow.phase() == Phase::InGame && self.flow.overlay_alpha() == 0.0,
            cells_streamed: self.streamer.fully_streamed(),
            textures_pending: self.texture_residency.stats.pending,
            materials_bound: !self.material_bg_dirty,
            weather_settled: self
                .sim
                .weather_snapshot()
                .is_none_or(|weather| weather.previous.is_none()),
            exposure: self.lighting.exposure.readout.exposure,
        }
    }

    /// Records the copy of the finished frame when the shot is due.
    pub(in crate::rendering) fn begin_shot_capture(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::Texture,
    ) -> Option<PendingFrame> {
        let shot = self.shot.as_mut()?;
        if !matches!(shot.progress, Progress::Due) {
            return None;
        }
        let expected = shot.config.window_size;
        let actual = PhysicalSize::new(frame.width(), frame.height());
        let recorded = if actual == expected {
            PendingFrame::record(&self.device, encoder, frame, shot.config.path.clone())
                .map_err(ShotError::from)
        } else {
            Err(ShotError::SizeMismatch { expected, actual })
        };
        match recorded {
            Ok(pending) => Some(pending),
            Err(error) => {
                shot.progress = Progress::Failed(error);
                None
            }
        }
    }

    /// Writes the frame once its copy has been submitted, then quits.
    pub(in crate::rendering) fn finish_shot_capture(&mut self, pending: Option<PendingFrame>) {
        let Some(pending) = pending else {
            return;
        };
        let written = pending.write(&self.device);
        let Some(shot) = self.shot.as_mut() else {
            return;
        };
        match written {
            Ok(path) => {
                tracing::info!("shot: wrote {}", path.display());
                shot.progress = Progress::Written;
                self.quit_requested = true;
            }
            Err(error) => shot.progress = Progress::Failed(error.into()),
        }
    }
}
