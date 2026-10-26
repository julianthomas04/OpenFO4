//! Plays what `WorldMusic` decides: a plan at a time, crossfading when the music type changes.
//!
//! - A single track streams once; when it ends and the type cycles, the next plan starts.
//! - A silent track is a timer.
//! - A palette runs one playlist per layer, all layers side by side: a layer starts its first
//!   sub-track with the palette and each following one when the previous has ended, a silent
//!   sub-track being a timed gap. The palette stops when its duration is over, whatever still
//!   plays fading out over the palette's own fade-out time (a palette without one stops at once).
//!
//! A new plan within one type starts without a fade; the file's own start and end shape it, and
//! so do the sub-tracks of a palette after the first.
//!
//! A change of the music type waits for the sounds that play to end and then starts the new type's
//! first pick, unless the new type is abrupt: that one crossfades at once over its fade time.
//! While a change waits, no further sub-track of a palette starts. Nothing to wait for (silence,
//! a silent plan, or a gap in every layer) starts the new type at once.

use std::path::PathBuf;
use std::time::Duration;

use super::conditions::GameTime;
use super::error::AudioError;
use super::music_output::{MusicOutput, Playing, SoundSettings};
use super::place::PlaceSnapshot;
use super::plan_run::{PlanRun, Start, Step};
use super::scheduler::{LOG_TARGET, LayerStep, PlayPlan};
use super::world_music::{MusicChange, WorldMusic};

pub const PLACE_POLLS_PER_SECOND: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
enum PlanEnd {
    /// Nothing is scheduled: silence until the music type changes.
    Open,
    WhenFinished,
    At {
        time: Duration,
        stop_fade: Duration,
    },
}

pub struct WorldPlayback {
    music: WorldMusic,
    audio_dir: PathBuf,
    active: bool,
    run: PlanRun,
    end: PlanEnd,
    /// A music type change waiting for the sounds to end; holds the plan it starts.
    queued: Option<QueuedChange>,
}

#[derive(Debug, Clone, PartialEq)]
struct QueuedChange {
    fade: Duration,
    plan: Option<PlayPlan>,
}

fn seconds(value: f32) -> Result<Duration, AudioError> {
    Duration::try_from_secs_f32(value).map_err(|_| AudioError::MusicDuration(value))
}

fn step(layer_step: LayerStep) -> Result<Step, AudioError> {
    match layer_step {
        LayerStep::Sound { file_key } => Ok(Step::Sound(file_key)),
        LayerStep::Gap { seconds: length } => seconds(length).map(Step::Gap),
    }
}

impl WorldPlayback {
    pub fn new(music: WorldMusic, audio_dir: PathBuf) -> Self {
        Self {
            music,
            audio_dir,
            active: false,
            run: PlanRun::default(),
            end: PlanEnd::Open,
            queued: None,
        }
    }

    pub fn start(&mut self) {
        self.active = true;
    }

    /// Fades the world music out over `fade` and forgets the place.
    pub fn stop(&mut self, output: &mut MusicOutput, fade: Duration) {
        tracing::debug!(
            target: LOG_TARGET,
            fade = ?fade,
            sounds = self.run.sound_count(),
            "world music stops"
        );
        self.active = false;
        self.end = PlanEnd::Open;
        self.queued = None;
        self.music.reset();
        output.fade_out(self.run.take_sounds(), fade);
    }

    pub fn sounds_mut(&mut self) -> impl Iterator<Item = &mut Playing> {
        self.run.sounds_mut()
    }

    pub fn sound_count(&self) -> usize {
        self.run.sound_count()
    }

    #[cfg(test)]
    pub(super) fn sound_file_names(&self) -> Vec<String> {
        self.run.file_names()
    }

    pub fn music(&self) -> &WorldMusic {
        &self.music
    }

    /// Follows the place and the plan; call once per frame while the world music is wanted.
    /// `now` is the audio clock; `place` is given when the place was polled this frame.
    pub fn sync(
        &mut self,
        output: &mut MusicOutput,
        now: Duration,
        time: GameTime,
        place: Option<&PlaceSnapshot>,
    ) -> Result<(), AudioError> {
        if !self.active {
            return Ok(());
        }
        if let Some(place) = place
            && let Some(change) = self.music.update_place(place, time)?
        {
            return self.change_music_type(output, now, change);
        }
        self.advance_run(output, now, Duration::ZERO)?;
        if self.plan_is_over(now) {
            self.start_next_plan(output, now, time)?;
        }
        Ok(())
    }

    fn change_music_type(
        &mut self,
        output: &mut MusicOutput,
        now: Duration,
        change: MusicChange,
    ) -> Result<(), AudioError> {
        if change.abrupt || !self.run.is_audible() {
            return self.crossfade(output, now, change);
        }
        tracing::debug!(
            target: LOG_TARGET,
            at = ?now,
            end = ?self.end,
            "type change queued: the playing sounds finish first"
        );
        self.queued = Some(QueuedChange {
            fade: change.fade,
            plan: change.plan,
        });
        Ok(())
    }

    fn crossfade(
        &mut self,
        output: &mut MusicOutput,
        now: Duration,
        change: MusicChange,
    ) -> Result<(), AudioError> {
        tracing::debug!(
            target: LOG_TARGET,
            at = ?now,
            fade = ?change.fade,
            audible = self.run.is_audible(),
            abrupt = change.abrupt,
            previous_end = ?self.end,
            "type change: fading out the old sounds, the new plan fades in over the same time"
        );
        self.queued = None;
        output.fade_out(self.run.take_sounds(), change.fade);
        self.start_plan(output, now, change.plan, change.fade)
    }

    /// Retires the sounds that ended and, unless a type change waits, starts the next steps of
    /// the layers; sounds that start now fade in over `fade_in`.
    fn advance_run(
        &mut self,
        output: &mut MusicOutput,
        now: Duration,
        fade_in: Duration,
    ) -> Result<(), AudioError> {
        let start = self.queued.is_none().then(|| Start {
            audio_dir: &self.audio_dir,
            settings: SoundSettings {
                looped: false,
                volume: self.music.volume(),
                fade_in,
            },
        });
        self.run.advance(now, output, start.as_ref())
    }

    fn plan_is_over(&self, now: Duration) -> bool {
        if self.queued.is_some() && !self.run.is_audible() {
            return true;
        }
        match self.end {
            PlanEnd::Open => false,
            PlanEnd::WhenFinished => self.run.is_finished(),
            PlanEnd::At { time, .. } => now >= time,
        }
    }

    fn start_next_plan(
        &mut self,
        output: &mut MusicOutput,
        now: Duration,
        time: GameTime,
    ) -> Result<(), AudioError> {
        tracing::debug!(target: LOG_TARGET, at = ?now, end = ?self.end, "plan ended");
        let sounds = self.run.take_sounds();
        match self.end {
            PlanEnd::At { stop_fade, .. } => output.fade_out(sounds, stop_fade),
            PlanEnd::WhenFinished | PlanEnd::Open => output.retire(sounds),
        }
        if let Some(queued) = self.queued.take() {
            return self.start_plan(output, now, queued.plan, queued.fade);
        }
        let plan = self.music.next_plan(time)?;
        self.start_plan(output, now, plan, Duration::ZERO)
    }

    fn start_plan(
        &mut self,
        output: &mut MusicOutput,
        now: Duration,
        plan: Option<PlayPlan>,
        fade_in: Duration,
    ) -> Result<(), AudioError> {
        tracing::debug!(
            target: LOG_TARGET,
            at = ?now,
            fade_in = ?fade_in,
            plan = ?plan,
            "plan starts"
        );
        match plan {
            None => {
                self.run = PlanRun::default();
                self.end = PlanEnd::Open;
            }
            Some(PlayPlan::Single { file_key }) => {
                self.run = PlanRun::new(vec![vec![Step::Sound(file_key)]]);
                self.end = PlanEnd::WhenFinished;
            }
            Some(PlayPlan::Silent { seconds: length }) => {
                self.run = PlanRun::default();
                self.end = PlanEnd::At {
                    time: now + seconds(length)?,
                    stop_fade: Duration::ZERO,
                };
            }
            Some(PlayPlan::Palette {
                layers,
                duration,
                fade_out,
            }) => {
                let playlists = layers
                    .into_iter()
                    .map(|layer| layer.into_iter().map(step).collect())
                    .collect::<Result<Vec<Vec<Step>>, _>>()?;
                self.run = PlanRun::new(playlists);
                self.end = PlanEnd::At {
                    time: now + seconds(duration)?,
                    stop_fade: fade_out.map_or(Ok(Duration::ZERO), seconds)?,
                };
            }
        }
        self.advance_run(output, now, fade_in)
    }
}

/// Lets the caller ask for the place at a fixed rate whatever the frame rate is.
#[derive(Debug, Clone)]
pub struct PlacePoll {
    interval: Duration,
    since_last: Duration,
}

impl PlacePoll {
    /// The first call to `due` is true.
    pub fn new(polls_per_second: u32) -> Self {
        let interval = Duration::from_secs(1) / polls_per_second;
        Self {
            interval,
            since_last: interval,
        }
    }

    pub fn due(&mut self, elapsed: Duration) -> bool {
        self.since_last += elapsed;
        let due = self.since_last >= self.interval;
        if due {
            self.since_last = Duration::ZERO;
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_place_poll_is_due_at_once_and_then_four_times_a_second() {
        let mut poll = PlacePoll::new(PLACE_POLLS_PER_SECOND);
        assert!(poll.due(Duration::ZERO));
        let due_polls = (0..4)
            .filter(|_| poll.due(Duration::from_millis(250)))
            .count();
        assert_eq!(due_polls, 4);
        assert!(!poll.due(Duration::from_millis(100)));
        assert!(poll.due(Duration::from_millis(150)));
    }
}
