//! The sounds of the plan that plays: one playlist per layer, each running its steps one after
//! another while the layers run side by side.

use std::collections::VecDeque;
use std::mem;
use std::path::Path;
use std::time::Duration;

use kira::sound::PlaybackState;

use super::error::AudioError;
use super::music_output::{MusicOutput, Playing, SoundSettings};

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Sound(String),
    Gap(Duration),
}

/// Where the sounds of a started step come from and how they play.
pub struct Start<'a> {
    pub audio_dir: &'a Path,
    pub settings: SoundSettings,
}

enum Current {
    Idle,
    Gap(Duration),
    Sound(Box<Playing>),
}

struct Layer {
    pending: VecDeque<Step>,
    current: Current,
}

impl Layer {
    fn is_finished(&self) -> bool {
        self.pending.is_empty() && matches!(self.current, Current::Idle)
    }

    fn is_audible(&self) -> bool {
        matches!(&self.current, Current::Sound(sound) if sound.state() != PlaybackState::Stopped)
    }

    /// Retires the sound that has ended and the gap that has elapsed, then starts the next steps
    /// as long as `start` is given and a step takes no time to start.
    fn advance(
        &mut self,
        now: Duration,
        output: &mut MusicOutput,
        start: Option<&Start>,
    ) -> Result<(), AudioError> {
        loop {
            match &self.current {
                Current::Sound(sound) if sound.state() == PlaybackState::Stopped => {
                    if let Current::Sound(ended) = mem::replace(&mut self.current, Current::Idle) {
                        output.retire([*ended]);
                    }
                }
                Current::Gap(until) if now >= *until => self.current = Current::Idle,
                Current::Sound(_) | Current::Gap(_) => return Ok(()),
                Current::Idle => {
                    let Some(start) = start else {
                        return Ok(());
                    };
                    match self.pending.pop_front() {
                        None => return Ok(()),
                        Some(Step::Gap(length)) => self.current = Current::Gap(now + length),
                        Some(Step::Sound(file_key)) => {
                            let sound =
                                output.play(&start.audio_dir.join(file_key), start.settings)?;
                            self.current = Current::Sound(Box::new(sound));
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
}

#[derive(Default)]
pub struct PlanRun {
    layers: Vec<Layer>,
}

impl PlanRun {
    pub fn new(playlists: Vec<Vec<Step>>) -> Self {
        Self {
            layers: playlists
                .into_iter()
                .map(|steps| Layer {
                    pending: steps.into(),
                    current: Current::Idle,
                })
                .collect(),
        }
    }

    /// Every layer has played its last step.
    pub fn is_finished(&self) -> bool {
        self.layers.iter().all(Layer::is_finished)
    }

    /// Some sound plays or fades.
    pub fn is_audible(&self) -> bool {
        self.layers.iter().any(Layer::is_audible)
    }

    pub fn sound_count(&self) -> usize {
        self.layers
            .iter()
            .filter(|layer| matches!(layer.current, Current::Sound(_)))
            .count()
    }

    #[cfg(test)]
    pub(super) fn file_names(&self) -> Vec<String> {
        self.layers
            .iter()
            .filter_map(|layer| match &layer.current {
                Current::Sound(sound) => Some(sound.file_name()),
                Current::Idle | Current::Gap(_) => None,
            })
            .collect()
    }

    pub fn sounds_mut(&mut self) -> impl Iterator<Item = &mut Playing> {
        self.layers
            .iter_mut()
            .filter_map(|layer| match &mut layer.current {
                Current::Sound(sound) => Some(sound.as_mut()),
                Current::Idle | Current::Gap(_) => None,
            })
    }

    /// Retires what has ended and, given `start`, starts what follows it.
    pub fn advance(
        &mut self,
        now: Duration,
        output: &mut MusicOutput,
        start: Option<&Start>,
    ) -> Result<(), AudioError> {
        self.layers
            .iter_mut()
            .try_for_each(|layer| layer.advance(now, output, start))
    }

    /// Empties the run and hands over the sounds that were playing.
    pub fn take_sounds(&mut self) -> Vec<Playing> {
        mem::take(&mut self.layers)
            .into_iter()
            .filter_map(|layer| match layer.current {
                Current::Sound(sound) => Some(*sound),
                Current::Idle | Current::Gap(_) => None,
            })
            .collect()
    }
}
