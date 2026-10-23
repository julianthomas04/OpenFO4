//! Paces the capture: apply a state, let the frame settle, take the picture, move on.

use super::states::{CaptureState, Scene};

/// Wall-clock time a state is shown before it is photographed, so slides and fades have ended.
pub const SETTLE_SECONDS: f32 = 1.5;
/// Frames a state is shown at least, so a stalled frame loop cannot satisfy the time alone.
pub const SETTLE_FRAMES: u32 = 5;

#[derive(Debug, PartialEq)]
pub enum CaptureEvent<'a> {
    Apply(&'a CaptureState),
    Capture(&'a CaptureState),
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Apply,
    Settling { seconds: f32, frames: u32 },
    Finish,
    Done,
}

pub struct Sequencer {
    states: Vec<CaptureState>,
    index: usize,
    phase: Phase,
}

impl Sequencer {
    pub fn new(states: Vec<CaptureState>) -> Self {
        let phase = if states.is_empty() {
            Phase::Finish
        } else {
            Phase::Apply
        };
        Self {
            states,
            index: 0,
            phase,
        }
    }

    /// The scene the state still to be captured needs; `None` once every state is captured.
    pub fn awaited_scene(&self) -> Option<Scene> {
        self.states.get(self.index).map(|state| state.scene)
    }

    /// One frame of progress; `dt` is the frame time in seconds.
    pub fn advance(&mut self, dt: f32) -> Option<CaptureEvent<'_>> {
        match self.phase {
            Phase::Apply => {
                self.phase = Phase::Settling {
                    seconds: 0.0,
                    frames: 0,
                };
                Some(CaptureEvent::Apply(&self.states[self.index]))
            }
            Phase::Settling { seconds, frames } => {
                let seconds = seconds + dt;
                let frames = frames + 1;
                if seconds < SETTLE_SECONDS || frames < SETTLE_FRAMES {
                    self.phase = Phase::Settling { seconds, frames };
                    return None;
                }
                let captured = self.index;
                self.index += 1;
                self.phase = if self.index < self.states.len() {
                    Phase::Apply
                } else {
                    Phase::Finish
                };
                Some(CaptureEvent::Capture(&self.states[captured]))
            }
            Phase::Finish => {
                self.phase = Phase::Done;
                Some(CaptureEvent::Finished)
            }
            Phase::Done => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(name: &str) -> CaptureState {
        CaptureState {
            name: name.to_owned(),
            scene: Scene::PipBoy,
            actions: Vec::new(),
        }
    }

    /// Advances with frame time `dt` until the state being settled is captured; returns the
    /// number of frames that took.
    fn frames_until_capture(sequencer: &mut Sequencer, dt: f32) -> u32 {
        for frames in 1..10_000 {
            if matches!(sequencer.advance(dt), Some(CaptureEvent::Capture(_))) {
                return frames;
            }
        }
        panic!("never captured");
    }

    #[test]
    fn states_are_applied_settled_and_captured_in_order() {
        let mut sequencer = Sequencer::new(vec![state("a"), state("b")]);
        for name in ["a", "b"] {
            assert_eq!(
                sequencer.advance(0.0),
                Some(CaptureEvent::Apply(&state(name)))
            );
            frames_until_capture(&mut sequencer, 0.5);
        }
        assert_eq!(sequencer.advance(0.0), Some(CaptureEvent::Finished));
        assert_eq!(sequencer.advance(0.0), None);
    }

    #[test]
    fn the_capture_names_the_state_that_was_applied() {
        let mut sequencer = Sequencer::new(vec![state("a")]);
        sequencer.advance(0.0);
        let mut event = sequencer.advance(SETTLE_SECONDS);
        while event.is_none() {
            event = sequencer.advance(SETTLE_SECONDS);
        }
        assert_eq!(event, Some(CaptureEvent::Capture(&state("a"))));
    }

    #[test]
    fn slow_frames_still_wait_for_the_minimum_frame_count() {
        let mut sequencer = Sequencer::new(vec![state("a")]);
        sequencer.advance(0.0);
        assert_eq!(frames_until_capture(&mut sequencer, 10.0), SETTLE_FRAMES);
    }

    #[test]
    fn fast_frames_still_wait_for_the_minimum_time() {
        let mut sequencer = Sequencer::new(vec![state("a")]);
        sequencer.advance(0.0);
        let frames = frames_until_capture(&mut sequencer, 0.25);
        assert_eq!(frames, 6);
    }

    #[test]
    fn the_awaited_scene_follows_the_state_to_capture() {
        let mut menu = state("a");
        menu.scene = Scene::MainMenu;
        let mut sequencer = Sequencer::new(vec![menu, state("b")]);
        assert_eq!(sequencer.awaited_scene(), Some(Scene::MainMenu));
        sequencer.advance(0.0);
        assert_eq!(sequencer.awaited_scene(), Some(Scene::MainMenu));
        frames_until_capture(&mut sequencer, 0.5);
        assert_eq!(sequencer.awaited_scene(), Some(Scene::PipBoy));
        sequencer.advance(0.0);
        frames_until_capture(&mut sequencer, 0.5);
        assert_eq!(sequencer.awaited_scene(), None);
    }

    #[test]
    fn an_empty_list_finishes_at_once() {
        let mut sequencer = Sequencer::new(Vec::new());
        assert_eq!(sequencer.advance(0.0), Some(CaptureEvent::Finished));
    }
}
