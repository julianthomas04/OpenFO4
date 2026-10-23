//! The one kira sub-track every music sound plays on, and the handles of the sounds on it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use kira::sound::FromFileError;
use kira::sound::PlaybackState;
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::track::{TrackBuilder, TrackHandle};
use kira::{AudioManager, Decibels, Tween};

use super::backend::AudioBackend;
use super::error::AudioError;

pub fn fade_tween(duration: Duration) -> Tween {
    Tween {
        duration,
        ..Tween::default()
    }
}

/// How one music sound is played.
#[derive(Debug, Clone, Copy)]
pub struct SoundSettings {
    /// The whole file is the loop region when set; otherwise the sound ends with the file.
    pub looped: bool,
    pub volume: Decibels,
    pub fade_in: Duration,
}

/// Opens a converted music file for streaming.
pub fn open_music_stream(
    path: &Path,
    looped: bool,
) -> Result<StreamingSoundData<FromFileError>, FromFileError> {
    StreamingSoundData::from_file(path).map(|data| if looped { data.loop_region(..) } else { data })
}

/// `open_music_stream` for a file whose failure to open is fatal to the audio.
pub fn open_music_file(
    path: &Path,
    looped: bool,
) -> Result<StreamingSoundData<FromFileError>, AudioError> {
    open_music_stream(path, looped).map_err(|source| AudioError::MusicFile {
        path: path.to_path_buf(),
        source,
    })
}

#[derive(Debug)]
pub struct Playing {
    handle: StreamingSoundHandle<FromFileError>,
    path: PathBuf,
}

impl Playing {
    #[cfg(test)]
    pub(super) fn file_name(&self) -> String {
        self.path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    pub fn state(&self) -> PlaybackState {
        self.handle.state()
    }

    pub fn position(&self) -> f64 {
        self.handle.position()
    }

    pub fn stop(&mut self, fade: Duration) {
        self.handle.stop(fade_tween(fade));
    }

    /// A decode error the stream hit while playing; fatal.
    pub fn take_error(&mut self) -> Option<AudioError> {
        self.handle
            .pop_error()
            .map(|source| AudioError::MusicStream {
                path: self.path.clone(),
                source,
            })
    }
}

pub struct MusicOutput {
    track: TrackHandle,
    /// Earlier sounds that are still fading out; their errors are polled until they stop.
    retiring: Vec<Playing>,
}

impl MusicOutput {
    pub fn new<B: AudioBackend>(manager: &mut AudioManager<B>) -> Result<Self, AudioError> {
        Ok(Self {
            track: manager.add_sub_track(TrackBuilder::new())?,
            retiring: Vec::new(),
        })
    }

    pub fn play(&mut self, path: &Path, settings: SoundSettings) -> Result<Playing, AudioError> {
        let data = open_music_file(path, settings.looped)?
            .volume(settings.volume)
            .fade_in_tween(fade_tween(settings.fade_in));
        let handle = self
            .track
            .play(data)
            .map_err(|source| AudioError::MusicPlayback {
                path: path.to_path_buf(),
                source,
            })?;
        Ok(Playing {
            handle,
            path: path.to_path_buf(),
        })
    }

    /// Fades the sounds out over `fade` and keeps them until they have stopped.
    pub fn fade_out(&mut self, sounds: impl IntoIterator<Item = Playing>, fade: Duration) {
        for mut sound in sounds {
            sound.stop(fade);
            self.retiring.push(sound);
        }
    }

    /// Keeps sounds that need no stop command, such as one that ended by itself, so their errors
    /// are still polled.
    pub fn retire(&mut self, sounds: impl IntoIterator<Item = Playing>) {
        self.retiring.extend(sounds);
    }

    /// Polls every sound for a decode error: the live ones given and the retiring ones.
    pub fn check<'a>(
        &mut self,
        live: impl IntoIterator<Item = &'a mut Playing>,
    ) -> Result<(), AudioError> {
        if let Some(error) = first_error(live).or_else(|| first_error(&mut self.retiring)) {
            return Err(error);
        }
        self.retiring
            .retain(|sound| sound.state() != PlaybackState::Stopped);
        Ok(())
    }

    #[cfg(test)]
    pub fn retiring_count(&self) -> usize {
        self.retiring.len()
    }
}

fn first_error<'a>(sounds: impl IntoIterator<Item = &'a mut Playing>) -> Option<AudioError> {
    sounds.into_iter().find_map(Playing::take_error)
}
