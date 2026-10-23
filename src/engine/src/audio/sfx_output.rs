//! The kira sub-track of the sound effects: plain, non-spatial one-shots from memory.

use std::collections::HashMap;
use std::path::PathBuf;

use kira::sound::static_sound::StaticSoundData;
use kira::track::{TrackBuilder, TrackHandle};
use kira::{AudioManager, Decibels, PlaybackRate};

use super::backend::AudioBackend;
use super::error::AudioError;

/// How one effect is played.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectSettings {
    pub playback_rate: f64,
    pub gain_db: f32,
}

pub struct SfxOutput {
    track: TrackHandle,
    audio_dir: PathBuf,
    /// Decoded files by file key; a clone shares the samples.
    decoded: HashMap<String, StaticSoundData>,
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
            audio_dir,
            decoded: HashMap::new(),
        })
    }

    /// Starts the file below the audio folder; it plays to its end without a handle. The first
    /// play of a file decodes it. A file that cannot be decoded or a track that cannot take
    /// another sound is an error.
    pub fn play(&mut self, file_key: &str, settings: EffectSettings) -> Result<(), AudioError> {
        let path = self.audio_dir.join(file_key);
        if !self.decoded.contains_key(file_key) {
            let data = StaticSoundData::from_file(&path).map_err(|source| AudioError::SfxFile {
                path: path.clone(),
                source,
            })?;
            self.decoded.insert(file_key.to_string(), data);
        }
        let data = self.decoded[file_key]
            .volume(Decibels(settings.gain_db))
            .playback_rate(PlaybackRate(settings.playback_rate));
        self.track
            .play(data)
            .map(drop)
            .map_err(|source| AudioError::SfxPlayback { path, source })
    }

    #[cfg(test)]
    pub fn decoded_count(&self) -> usize {
        self.decoded.len()
    }
}
