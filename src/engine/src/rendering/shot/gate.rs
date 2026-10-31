//! When the world is ready to be photographed: everything around the camera streamed in, the
//! weather settled and the eye adapted, for a run of consecutive frames.

use std::fmt;
use std::time::Duration;

/// How long the shot waits for the world before it gives up.
pub const SHOT_TIMEOUT: Duration = Duration::from_secs(300);
/// The largest frame-to-frame change of the exposure, relative to it, that counts as adapted.
const EXPOSURE_TOLERANCE: f32 = 0.02;

/// What one frame reports about the world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShotFrame {
    /// The loading screen and its fade are gone.
    pub world_shown: bool,
    /// No cell of the detail or LOD ring is still loading or waiting to be integrated.
    pub cells_streamed: bool,
    /// Texture reads in flight or waiting for their upload.
    pub textures_pending: u32,
    /// The material bind group shows every texture slot as it is now.
    pub materials_bound: bool,
    /// No weather transition is running.
    pub weather_settled: bool,
    /// The latest exposure the GPU reported; 0 until it reported one.
    pub exposure: f32,
}

/// The first condition a frame fails, in the order the world gets ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocker {
    Loading,
    Cells,
    Textures(u32),
    Materials,
    Weather,
    Exposure,
}

impl fmt::Display for Blocker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Loading => write!(f, "the loading screen is still up"),
            Self::Cells => write!(f, "cells are still streaming"),
            Self::Textures(pending) => write!(f, "{pending} texture reads are still pending"),
            Self::Materials => write!(f, "the material bind group is not yet rebuilt"),
            Self::Weather => write!(f, "a weather transition is still running"),
            Self::Exposure => write!(f, "the eye adaptation has not converged"),
        }
    }
}

/// What the shot does after a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotStep {
    Wait,
    Capture,
    TimedOut(Blocker),
}

#[derive(Debug, Clone)]
pub struct ShotGate {
    settle_frames: u32,
    ready_frames: u32,
    previous_exposure: Option<f32>,
    last_blocker: Blocker,
}

impl ShotGate {
    pub fn new(settle_frames: u32) -> Self {
        Self {
            settle_frames,
            ready_frames: 0,
            previous_exposure: None,
            last_blocker: Blocker::Loading,
        }
    }

    /// Takes one frame's report, `elapsed` after the shot began. The shot is taken once the
    /// world has been ready for `settle_frames` frames in a row; a frame that is not ready starts
    /// the count over. Past [`SHOT_TIMEOUT`] the shot gives up instead of taking a frame that is
    /// not ready.
    pub fn step(&mut self, frame: &ShotFrame, elapsed: Duration) -> ShotStep {
        let blocker = self.blocker(frame);
        self.previous_exposure = Some(frame.exposure);
        match blocker {
            Some(blocker) => {
                self.ready_frames = 0;
                self.last_blocker = blocker;
            }
            None => self.ready_frames = self.ready_frames.saturating_add(1),
        }
        if self.ready_frames >= self.settle_frames {
            ShotStep::Capture
        } else if elapsed >= SHOT_TIMEOUT {
            ShotStep::TimedOut(self.last_blocker)
        } else {
            ShotStep::Wait
        }
    }

    fn blocker(&self, frame: &ShotFrame) -> Option<Blocker> {
        if !frame.world_shown {
            Some(Blocker::Loading)
        } else if !frame.cells_streamed {
            Some(Blocker::Cells)
        } else if frame.textures_pending > 0 {
            Some(Blocker::Textures(frame.textures_pending))
        } else if !frame.materials_bound {
            Some(Blocker::Materials)
        } else if !frame.weather_settled {
            Some(Blocker::Weather)
        } else if !exposure_converged(self.previous_exposure, frame.exposure) {
            Some(Blocker::Exposure)
        } else {
            None
        }
    }
}

/// Whether the exposure has arrived and moved by no more than the tolerance since the frame before.
fn exposure_converged(previous: Option<f32>, current: f32) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    current.is_finite()
        && current > 0.0
        && (current - previous).abs() <= current * EXPOSURE_TOLERANCE
}

#[cfg(test)]
mod tests {
    use super::*;

    const EARLY: Duration = Duration::from_secs(1);

    fn ready() -> ShotFrame {
        ShotFrame {
            world_shown: true,
            cells_streamed: true,
            textures_pending: 0,
            materials_bound: true,
            weather_settled: true,
            exposure: 1.0,
        }
    }

    fn steps(gate: &mut ShotGate, frame: &ShotFrame, count: u32) -> Vec<ShotStep> {
        (0..count).map(|_| gate.step(frame, EARLY)).collect()
    }

    #[test]
    fn the_shot_is_taken_after_the_settle_frames_of_a_ready_world() {
        let mut gate = ShotGate::new(3);
        let taken = steps(&mut gate, &ready(), 4);
        // The first frame has no earlier exposure to compare against.
        assert_eq!(
            taken,
            [
                ShotStep::Wait,
                ShotStep::Wait,
                ShotStep::Wait,
                ShotStep::Capture
            ]
        );
    }

    #[test]
    fn a_frame_that_is_not_ready_starts_the_count_over() {
        let mut gate = ShotGate::new(2);
        steps(&mut gate, &ready(), 2);
        let streaming = ShotFrame {
            cells_streamed: false,
            ..ready()
        };
        assert_eq!(gate.step(&streaming, EARLY), ShotStep::Wait);
        assert_eq!(gate.step(&ready(), EARLY), ShotStep::Wait);
        assert_eq!(gate.step(&ready(), EARLY), ShotStep::Capture);
    }

    #[test]
    fn each_condition_alone_holds_the_shot() {
        let frames = [
            ShotFrame {
                world_shown: false,
                ..ready()
            },
            ShotFrame {
                cells_streamed: false,
                ..ready()
            },
            ShotFrame {
                textures_pending: 1,
                ..ready()
            },
            ShotFrame {
                materials_bound: false,
                ..ready()
            },
            ShotFrame {
                weather_settled: false,
                ..ready()
            },
            ShotFrame {
                exposure: 0.0,
                ..ready()
            },
        ];
        for frame in frames {
            let mut gate = ShotGate::new(1);
            assert!(
                steps(&mut gate, &frame, 10)
                    .iter()
                    .all(|step| *step == ShotStep::Wait),
                "{frame:?}"
            );
        }
    }

    #[test]
    fn an_exposure_still_moving_beyond_the_tolerance_is_not_converged() {
        assert!(!exposure_converged(None, 1.0));
        assert!(!exposure_converged(Some(1.0), 1.1));
        assert!(!exposure_converged(Some(1.0), f32::NAN));
        assert!(exposure_converged(Some(1.0), 1.0));
        assert!(exposure_converged(Some(1.01), 1.0));
    }

    #[test]
    fn a_world_never_ready_times_out_with_what_blocked_it() {
        let mut gate = ShotGate::new(1);
        let textures = ShotFrame {
            textures_pending: 4,
            ..ready()
        };
        assert_eq!(gate.step(&textures, EARLY), ShotStep::Wait);
        assert_eq!(
            gate.step(&textures, SHOT_TIMEOUT),
            ShotStep::TimedOut(Blocker::Textures(4))
        );
    }

    #[test]
    fn a_ready_world_is_taken_even_on_the_frame_of_the_timeout() {
        let mut gate = ShotGate::new(2);
        gate.step(&ready(), EARLY);
        gate.step(&ready(), EARLY);
        assert_eq!(gate.step(&ready(), SHOT_TIMEOUT), ShotStep::Capture);
    }
}
