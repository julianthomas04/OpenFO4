//! The engine side of the shot: reads how ready the world is and writes the frame.

use winit::dpi::PhysicalSize;

use super::gate::{ShotFrame, ShotStep};
use super::{Progress, ShotError};
use crate::game_flow::Phase;
use crate::rendering::State;
use crate::rendering::frame_capture::PendingFrame;

impl State {
    pub(in crate::rendering) fn shot_active(&self) -> bool {
        self.shot.is_some()
    }

    /// Feeds this frame's state of the world to the shot; an error ends the game.
    pub(in crate::rendering) fn drive_shot(&mut self) -> Result<(), ShotError> {
        let frame = self.shot_frame();
        let Some(shot) = self.shot.as_mut() else {
            return Ok(());
        };
        if let Some(error) = shot.take_failure() {
            return Err(error);
        }
        if !matches!(shot.progress, Progress::Waiting) {
            return Ok(());
        }
        let after = shot.started.elapsed();
        match shot.gate.step(&frame, after) {
            ShotStep::Wait => Ok(()),
            ShotStep::Capture => {
                tracing::info!("shot: the world is ready after {:.1}s", after.as_secs_f32());
                shot.progress = Progress::Due;
                Ok(())
            }
            ShotStep::TimedOut(blocker) => {
                shot.progress = Progress::Reported;
                Err(ShotError::TimedOut { blocker, after })
            }
        }
    }

    fn shot_frame(&self) -> ShotFrame {
        ShotFrame {
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
