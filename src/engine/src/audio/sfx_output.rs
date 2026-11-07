//! The kira sub-track of the sound effects: plain, non-spatial one-shots from memory.

use std::path::PathBuf;

use kira::track::{TrackBuilder, TrackHandle};
use kira::{AudioManager, Decibels, PlaybackRate};

use super::backend::AudioBackend;
use super::decoded_files::DecodedFiles;
use super::error::AudioError;
use super::sound_resolve::LayerPlay;

/// How one effect is played.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectSettings {
    pub playback_rate: f64,
    pub gain_db: f32,
}

pub struct SfxOutput {
    track: TrackHandle,
    files: DecodedFiles,
}

impl SfxOutput {
    pub fn new<B: AudioBackend>(
        manager: &mut AudioManager<B>,
        audio_dir: PathBuf,
    ) -> Result<Self, AudioError> {
        Ok(Self {
            track: manager
                .add_sub_track(TrackBuilder::new())
                .map_err(AudioError::SfxTrack)?,
            files: DecodedFiles::new(audio_dir),
        })
    }

    /// Starts the file below the audio folder; it plays to its end without a handle. The first
    /// play of a file decodes it. A file that cannot be decoded or a track that cannot take
    /// another sound is an error.
    pub fn play(&mut self, file_key: &str, settings: EffectSettings) -> Result<(), AudioError> {
        let data = self
            .files
            .get(file_key)?
            .volume(Decibels(settings.gain_db))
            .playback_rate(PlaybackRate(settings.playback_rate));
        self.track
            .play(data)
            .map(drop)
            .map_err(|source| AudioError::SfxPlayback {
                path: self.files.path(file_key),
                source,
            })
    }

    /// Plays one resolved layer at its own pitch and level.
    pub fn play_layer(&mut self, layer: &LayerPlay) -> Result<(), AudioError> {
        self.play(
            &layer.file_key,
            EffectSettings {
                playback_rate: layer.mix.playback_rate,
                gain_db: layer.mix.gain_db,
            },
        )
    }

    #[cfg(test)]
    pub fn decoded_count(&self) -> usize {
        self.files.len()
    }
}
