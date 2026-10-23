//! Timeline playback of a Pip-Boy art clip: the clip's frame scripts decide when it holds, jumps
//! or wraps. Frames in scripts and start requests are 1-based; the frame returned is 0-based.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScriptAction {
    Stop,
    Play,
    GotoAndStop,
    GotoAndPlay,
    NextFrame,
    PrevFrame,
    /// An action name this engine does not know; the manifest loader rejects it.
    #[serde(other)]
    Unknown,
}

impl ScriptAction {
    pub fn needs_target(self) -> bool {
        matches!(self, Self::GotoAndStop | Self::GotoAndPlay)
    }
}

/// What a clip does on entering `frame`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptStep {
    pub frame: u32,
    pub action: ScriptAction,
    #[serde(default)]
    pub target: Option<u32>,
}

/// Where playback begins when the clip does not simply start playing at frame 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackStart {
    PlayFrom(u32),
    HoldAt(u32),
}

#[derive(Clone, Copy)]
struct Head {
    frame: u32,
    playing: bool,
}

struct Timeline<'a> {
    script: &'a [ScriptStep],
    frame_count: u32,
}

impl Timeline<'_> {
    fn clamp(&self, frame: u32) -> u32 {
        frame.clamp(1, self.frame_count)
    }

    /// Enters `frame` and runs its scripts in order; a jump enters its target and runs that
    /// frame's scripts too. A chain of jumps longer than the script is a cycle and holds.
    fn enter(&self, frame: u32, playing: bool) -> Head {
        let mut head = Head {
            frame: self.clamp(frame),
            playing,
        };
        let mut jumps_left = self.script.len();
        'entered: loop {
            for step in self.script.iter().filter(|step| step.frame == head.frame) {
                let jump = match (step.action, step.target) {
                    (ScriptAction::Stop, _) => {
                        head.playing = false;
                        None
                    }
                    (ScriptAction::Play, _) => {
                        head.playing = true;
                        None
                    }
                    (ScriptAction::GotoAndStop, Some(target)) => Some((target, false)),
                    (ScriptAction::GotoAndPlay, Some(target)) => Some((target, true)),
                    (ScriptAction::NextFrame, _) => Some((head.frame.saturating_add(1), false)),
                    (ScriptAction::PrevFrame, _) => Some((head.frame.saturating_sub(1), false)),
                    (
                        ScriptAction::GotoAndStop
                        | ScriptAction::GotoAndPlay
                        | ScriptAction::Unknown,
                        _,
                    ) => None,
                };
                if let Some((target, playing)) = jump {
                    if jumps_left == 0 {
                        head.playing = false;
                        return head;
                    }
                    jumps_left -= 1;
                    head = Head {
                        frame: self.clamp(target),
                        playing,
                    };
                    continue 'entered;
                }
            }
            return head;
        }
    }

    fn tick(&self, head: Head) -> Head {
        if !head.playing {
            return head;
        }
        let next = if head.frame >= self.frame_count {
            1
        } else {
            head.frame + 1
        };
        self.enter(next, true)
    }
}

fn start_head(timeline: &Timeline<'_>, start: Option<PlaybackStart>) -> Head {
    match start {
        None => timeline.enter(1, true),
        Some(PlaybackStart::PlayFrom(frame)) => timeline.enter(frame, true),
        Some(PlaybackStart::HoldAt(frame)) => timeline.enter(frame, false),
    }
}

fn whole_ticks(fps: f32, elapsed: f64) -> u64 {
    let seconds = if elapsed.is_finite() {
        elapsed.max(0.0)
    } else {
        0.0
    };
    (seconds * f64::from(fps)).floor() as u64
}

/// The 0-based frame a clip shows `elapsed` seconds after it started, running its frame scripts.
/// A clip with an empty script plays on and wraps. Non-finite or negative `elapsed` counts as zero.
pub fn frame_at(
    script: &[ScriptStep],
    frame_count: usize,
    fps: f32,
    elapsed: f64,
    start: Option<PlaybackStart>,
) -> usize {
    let Ok(frame_count) = u32::try_from(frame_count) else {
        return 0;
    };
    if frame_count == 0 {
        return 0;
    }
    let timeline = Timeline {
        script,
        frame_count,
    };
    let mut head = start_head(&timeline, start);
    let mut ticks = whole_ticks(fps, elapsed);
    let mut seen: HashMap<u32, u64> = HashMap::new();
    while ticks > 0 && head.playing {
        if let Some(earlier) = seen.insert(head.frame, ticks) {
            ticks %= earlier - ticks;
            seen.clear();
            if ticks == 0 {
                break;
            }
        }
        head = timeline.tick(head);
        ticks -= 1;
    }
    (head.frame - 1) as usize
}

/// Whether a clip shows the 1-based `frame` at any tick up to `elapsed` seconds after it started.
pub fn reaches_frame(
    script: &[ScriptStep],
    frame_count: usize,
    fps: f32,
    elapsed: f64,
    start: Option<PlaybackStart>,
    frame: usize,
) -> bool {
    let (Ok(frame_count), Ok(frame)) = (u32::try_from(frame_count), u32::try_from(frame)) else {
        return false;
    };
    if frame_count == 0 {
        return false;
    }
    let timeline = Timeline {
        script,
        frame_count,
    };
    let mut head = start_head(&timeline, start);
    let mut ticks = whole_ticks(fps, elapsed);
    let mut seen = HashSet::new();
    while head.frame != frame && ticks > 0 && head.playing && seen.insert(head.frame) {
        head = timeline.tick(head);
        ticks -= 1;
    }
    head.frame == frame
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(frame: u32, action: ScriptAction, target: Option<u32>) -> ScriptStep {
        ScriptStep {
            frame,
            action,
            target,
        }
    }

    fn at(script: &[ScriptStep], frames: usize, tick: u32, start: Option<PlaybackStart>) -> usize {
        frame_at(script, frames, 10.0, f64::from(tick) / 10.0 + 0.01, start)
    }

    fn run(
        script: &[ScriptStep],
        frames: usize,
        ticks: u32,
        start: Option<PlaybackStart>,
    ) -> Vec<usize> {
        (0..ticks)
            .map(|tick| at(script, frames, tick, start))
            .collect()
    }

    #[test]
    fn an_empty_script_plays_on_and_wraps() {
        assert_eq!(run(&[], 4, 9, None), [0, 1, 2, 3, 0, 1, 2, 3, 0]);
    }

    #[test]
    fn stop_on_the_first_frame_holds_it() {
        let script = [step(1, ScriptAction::Stop, None)];
        assert_eq!(at(&script, 5, 0, None), 0);
        assert_eq!(at(&script, 5, 1000, None), 0);
    }

    #[test]
    fn stop_on_the_last_frame_holds_there() {
        let script = [step(4, ScriptAction::Stop, None)];
        assert_eq!(run(&script, 4, 6, None), [0, 1, 2, 3, 3, 3]);
    }

    #[test]
    fn a_goto_and_play_on_the_last_frame_loops_the_tail() {
        let script = [
            step(1, ScriptAction::Stop, None),
            step(5, ScriptAction::GotoAndPlay, Some(2)),
        ];
        assert_eq!(at(&script, 5, 0, None), 0);
        let from_two = Some(PlaybackStart::PlayFrom(2));
        assert_eq!(run(&script, 5, 9, from_two), [1, 2, 3, 1, 2, 3, 1, 2, 3]);
    }

    #[test]
    fn a_goto_and_stop_jumps_and_holds() {
        let script = [step(2, ScriptAction::GotoAndStop, Some(4))];
        assert_eq!(run(&script, 6, 4, None), [0, 3, 3, 3]);
    }

    #[test]
    fn a_mid_clip_goto_and_play_jumps_back_and_continues() {
        let script = [step(3, ScriptAction::GotoAndPlay, Some(1))];
        assert_eq!(run(&script, 5, 6, None), [0, 1, 0, 1, 0, 1]);
    }

    #[test]
    fn next_and_prev_frame_step_and_hold() {
        let next = [step(1, ScriptAction::NextFrame, None)];
        assert_eq!(run(&next, 4, 3, None), [1, 1, 1]);
        let prev = [step(3, ScriptAction::PrevFrame, None)];
        assert_eq!(run(&prev, 4, 5, None), [0, 1, 1, 1, 1]);
    }

    #[test]
    fn play_resumes_a_stopped_clip() {
        let script = [
            step(1, ScriptAction::Stop, None),
            step(1, ScriptAction::Play, None),
        ];
        assert_eq!(run(&script, 3, 4, None), [0, 1, 2, 0]);
    }

    #[test]
    fn a_start_request_plays_from_or_holds_at_the_given_frame() {
        assert_eq!(
            run(&[], 5, 4, Some(PlaybackStart::PlayFrom(3))),
            [2, 3, 4, 0]
        );
        assert_eq!(at(&[], 5, 30, Some(PlaybackStart::HoldAt(4))), 3);
    }

    #[test]
    fn a_start_frame_outside_the_clip_is_clamped() {
        assert_eq!(at(&[], 3, 0, Some(PlaybackStart::PlayFrom(99))), 2);
        assert_eq!(at(&[], 3, 0, Some(PlaybackStart::HoldAt(0))), 0);
    }

    #[test]
    fn a_start_on_a_scripted_frame_runs_that_frames_script() {
        let script = [step(3, ScriptAction::Stop, None)];
        assert_eq!(at(&script, 5, 20, Some(PlaybackStart::PlayFrom(3))), 2);
    }

    #[test]
    fn a_goto_cycle_within_one_tick_terminates_and_holds() {
        let script = [
            step(1, ScriptAction::GotoAndPlay, Some(2)),
            step(2, ScriptAction::GotoAndPlay, Some(1)),
        ];
        let shown = at(&script, 3, 7, None);
        assert!(shown < 2);
        assert_eq!(shown, at(&script, 3, 80, None));
    }

    #[test]
    fn a_huge_elapsed_time_reduces_by_the_loop_period() {
        let script = [step(5, ScriptAction::GotoAndPlay, Some(2))];
        assert_eq!(frame_at(&script, 5, 10.0, 1.0e9, None), 1);
        assert_eq!(frame_at(&script, 5, 10.0, 1.0e9 + 0.15, None), 2);
    }

    #[test]
    fn degenerate_inputs_show_the_first_frame() {
        assert_eq!(frame_at(&[], 0, 10.0, 5.0, None), 0);
        assert_eq!(frame_at(&[], 4, 10.0, -3.0, None), 0);
        assert_eq!(frame_at(&[], 4, 10.0, f64::NAN, None), 0);
        assert_eq!(frame_at(&[], 4, 0.0, 9.0, None), 0);
    }

    #[test]
    fn a_clip_reaches_its_last_frame_when_it_plays_there_even_if_it_then_wraps() {
        let reached =
            |ticks: u32| reaches_frame(&[], 4, 10.0, f64::from(ticks) / 10.0 + 0.01, None, 4);
        assert!(!reached(2));
        assert!(reached(3));
        assert!(reached(40));
    }

    #[test]
    fn a_clip_that_loops_before_the_frame_or_stops_short_never_reaches_it() {
        let loops = [step(3, ScriptAction::GotoAndPlay, Some(1))];
        assert!(!reaches_frame(&loops, 5, 10.0, 1000.0, None, 5));
        let stops = [step(2, ScriptAction::Stop, None)];
        assert!(!reaches_frame(&stops, 5, 10.0, 1000.0, None, 5));
    }

    #[test]
    fn a_one_frame_clip_is_on_its_last_frame_at_once() {
        assert!(reaches_frame(&[], 1, 10.0, 0.0, None, 1));
        assert!(!reaches_frame(&[], 0, 10.0, 0.0, None, 1));
    }
}
