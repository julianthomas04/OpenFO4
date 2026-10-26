//! Weather sounds: the loops of the weathers that show and the thunder of their strikes.
//!
//! `WeatherAudio` is the stateful shell around the pure level math in `levels` and the thunder
//! queue: it holds the sound catalog, the condition context and the random source, checks the
//! converted files at startup and says each frame which loops should play at what gain and which
//! thunder is due. Playing them is up to `LoopOutput` and `SfxOutput`.
//!
//! A source file the game never shipped is absent from the install; startup logs it once and
//! leaves it out of every pick, and a sound left with no file is silent. A strike's thunder plays
//! every layer of every thunder sound of its weather, each layer with its own file and variance,
//! and only outdoors: thunder that comes due while the player is indoors is dropped.

mod error;
mod input;
mod levels;
mod loader;
mod table;
#[cfg(test)]
mod tests;
mod thunder;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use kira::sound::streaming::StreamingSoundData;

pub use error::WeatherAudioError;
pub use input::{ThunderStrike, WeatherAudioInput, WeatherSide};
pub use loader::{load_or_report, load_weather_audio};
pub use table::WeatherSoundTable;

use levels::{LoopLevel, loop_levels};
use thunder::{DueThunder, ThunderQueue};

use super::conditions::{FreshWorldContext, GameTime, evaluate};
use super::decibels::amplitude_to_db;
use super::file_probe::probe_files;
use super::loop_plan::LoopTarget;
use super::sound_resolve::{
    LayerPlay, LoopPlay, lookup_descriptor, reachable_sounds, resolve_layers, resolve_loop,
};
use crate::assets::db::sound_catalog::SoundCatalog;
use crate::random::SplitMix64;
use crate::weather::SoundKind;

/// The tracing target of everything the weather sounds decide.
pub const LOG_TARGET: &str = "openfo4::audio::weather";

/// What the weather sounds play right now.
#[derive(Debug, Default, PartialEq)]
pub struct WeatherPlay {
    /// The loops that should play and the gain of each; a loop that is not listed fades out.
    pub loops: Vec<LoopTarget>,
    /// The layers of the thunder that came due, at their final gain.
    pub thunder: Vec<LayerPlay>,
}

pub struct WeatherAudio {
    sounds: SoundCatalog,
    table: WeatherSoundTable,
    context: FreshWorldContext,
    rng: SplitMix64,
    missing_files: HashSet<String>,
    /// What each looping descriptor plays, decided on first use so that a loop keeps its file;
    /// `None` when every file of the descriptor is missing.
    loops: HashMap<String, Option<LoopPlay>>,
    thunder: ThunderQueue,
}

impl WeatherAudio {
    /// `audio_dir` is where the converted sound files live. Every file a sound of `table` can
    /// reach, whatever its conditions, must be present with a readable header, or absent: an
    /// absent file is logged once and skipped. Every looping sound and every thunder sound is
    /// resolved once, so that data that cannot be played fails here and not in the middle of a
    /// storm.
    pub fn new(
        sounds: SoundCatalog,
        table: WeatherSoundTable,
        audio_dir: &Path,
        seed: u64,
    ) -> Result<Self, WeatherAudioError> {
        let reachable = reachable_sounds(&sounds, table.descriptors())?;
        let missing_files = probe_files(
            "weather sound",
            reachable
                .iter()
                .flat_map(|sound| sound.file_keys.iter().map(String::as_str)),
            audio_dir,
            |path| StreamingSoundData::from_file(path).map(|_| ()),
        )?;
        for sound in &reachable {
            if sound
                .file_keys
                .iter()
                .all(|file_key| missing_files.contains(file_key))
            {
                tracing::warn!(
                    target: LOG_TARGET,
                    "weather sound {} has no file left; it is silent",
                    sound.descriptor
                );
            }
        }
        let mut audio = Self {
            sounds,
            table,
            context: FreshWorldContext::new(0.0, []),
            rng: SplitMix64::new(seed),
            missing_files,
            loops: HashMap::new(),
            thunder: ThunderQueue::default(),
        };
        audio.rehearse()?;
        Ok(audio)
    }

    /// The file keys known to be absent from the install.
    pub fn missing_files(&self) -> &HashSet<String> {
        &self.missing_files
    }

    /// What plays at `now`, the time the frames have added up to, for the weather `input` describes
    /// and the game time `time`: the loops that should play, and the layers of the thunder whose
    /// strike came `delay_s` ago. Without an `input` there is no weather to hear: no loop is wanted
    /// and the thunder on its way is dropped. A weather the table does not know, or sound data that
    /// cannot be played, is an error.
    pub fn update(
        &mut self,
        input: Option<&WeatherAudioInput>,
        time: GameTime,
        now: Duration,
    ) -> Result<WeatherPlay, WeatherAudioError> {
        let Some(input) = input else {
            self.thunder.clear();
            return Ok(WeatherPlay::default());
        };
        self.context.set_time(time);
        self.context.set_interior(!input.outdoor);
        for strike in &input.strikes {
            self.thunder.push(*strike, now);
        }
        Ok(WeatherPlay {
            loops: self.loop_targets(input)?,
            thunder: self.due_thunder(input.outdoor, now)?,
        })
    }

    fn loop_targets(
        &mut self,
        input: &WeatherAudioInput,
    ) -> Result<Vec<LoopTarget>, WeatherAudioError> {
        let mut targets = Vec::new();
        for LoopLevel { descriptor, level } in
            loop_levels(&self.table, &input.sides, input.outdoor)?
        {
            if let Some(play) = self.loop_play(&descriptor)? {
                targets.push(LoopTarget {
                    file_key: play.file_key,
                    gain_db: play.gain_db + amplitude_to_db(level),
                });
            }
        }
        Ok(targets)
    }

    /// The file and the steady gain `descriptor` loops with; `None` while its conditions fail or
    /// when it has no file left.
    fn loop_play(&mut self, descriptor: &str) -> Result<Option<LoopPlay>, WeatherAudioError> {
        if !self.conditions_hold(descriptor)? {
            return Ok(None);
        }
        self.resolved_loop(descriptor)
    }

    fn conditions_hold(&self, descriptor: &str) -> Result<bool, WeatherAudioError> {
        let conditions = &lookup_descriptor(&self.sounds, descriptor)?.conditions;
        Ok(evaluate(conditions, &self.context)?)
    }

    /// What `descriptor` loops with whatever its conditions; the first call decides the file.
    fn resolved_loop(&mut self, descriptor: &str) -> Result<Option<LoopPlay>, WeatherAudioError> {
        if !self.loops.contains_key(descriptor) {
            let play = resolve_loop(&self.sounds, descriptor, &self.missing_files, &mut self.rng)?;
            self.loops.insert(descriptor.to_string(), play);
        }
        Ok(self.loops.get(descriptor).cloned().flatten())
    }

    fn due_thunder(
        &mut self,
        outdoor: bool,
        now: Duration,
    ) -> Result<Vec<LayerPlay>, WeatherAudioError> {
        let mut layers = Vec::new();
        for due in self.thunder.take_due(now) {
            if outdoor {
                layers.extend(self.thunder_layers(due)?);
            }
        }
        Ok(layers)
    }

    /// The layers of every thunder sound of the strike's weather, at the strike's gain.
    fn thunder_layers(&mut self, due: DueThunder) -> Result<Vec<LayerPlay>, WeatherAudioError> {
        let mut layers = Vec::new();
        for sound in self
            .table
            .sounds(due.weather)?
            .iter()
            .filter(|sound| sound.kind == SoundKind::Thunder)
        {
            layers.extend(resolve_layers(
                &self.sounds,
                &self.context,
                &sound.descriptor,
                &self.missing_files,
                &mut self.rng,
            )?);
        }
        for layer in &mut layers {
            layer.mix.gain_db += due.gain_db;
        }
        tracing::debug!(
            target: LOG_TARGET,
            "thunder of weather {} plays {}",
            due.weather.index(),
            layers
                .iter()
                .map(|layer| layer.file_key.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        Ok(layers)
    }

    fn rehearse(&mut self) -> Result<(), WeatherAudioError> {
        let looping = owned(self.table.looping_descriptors());
        let thunder = owned(self.table.thunder_descriptors());
        for descriptor in &looping {
            self.conditions_hold(descriptor)?;
            self.resolved_loop(descriptor)?;
        }
        for descriptor in &thunder {
            resolve_layers(
                &self.sounds,
                &self.context,
                descriptor,
                &self.missing_files,
                &mut self.rng,
            )?;
        }
        Ok(())
    }
}

fn owned(descriptors: BTreeSet<&str>) -> Vec<String> {
    descriptors.into_iter().map(str::to_string).collect()
}
