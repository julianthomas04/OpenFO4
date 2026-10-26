use std::path::PathBuf;

use kira::sound::FromFileError;

use crate::audio::conditions::ConditionError;
use crate::audio::file_probe::ProbeError;
use crate::audio::sound_resolve::{LayersError, SoundError};

#[derive(Debug, thiserror::Error)]
pub enum WeatherAudioError {
    #[error("cannot read the weather sound data: {0:#}")]
    Catalog(anyhow::Error),
    #[error(transparent)]
    Sound(#[from] SoundError),
    #[error(transparent)]
    Layers(#[from] LayersError),
    #[error(transparent)]
    Condition(#[from] ConditionError),
    #[error("weather {0} is not one of the weathers whose sounds were loaded")]
    UnknownWeather(usize),
    #[error("the audio folder {0} is missing; run the installer")]
    AudioDirectoryMissing(PathBuf),
    #[error("cannot check {path}: {source}")]
    FileCheck {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "weather sound {path} cannot be opened; the installer converts it from the game, \
         so run the installer again: {source}"
    )]
    UnreadableFile {
        path: PathBuf,
        source: FromFileError,
    },
}

impl From<ProbeError> for WeatherAudioError {
    fn from(error: ProbeError) -> Self {
        match error {
            ProbeError::DirectoryMissing(path) => Self::AudioDirectoryMissing(path),
            ProbeError::Check { path, source } => Self::FileCheck { path, source },
            ProbeError::Open { path, source } => Self::UnreadableFile { path, source },
        }
    }
}
