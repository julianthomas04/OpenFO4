use std::path::PathBuf;

use kira::sound::FromFileError;
use kira::{PlaySoundError, ResourceLimitReached};

use super::footsteps::FootstepError;
use super::world_music::WorldMusicError;
use crate::assets::layout::AudioKeyError;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("cannot open the audio device: {0}")]
    Device(String),
    #[error("cannot create the music track: {0}")]
    MusicTrack(#[from] ResourceLimitReached),
    #[error(
        "music {path} cannot be opened or decoded; the installer converts it \
         from the game, so run the installer again: {source}"
    )]
    MusicFile {
        path: PathBuf,
        source: FromFileError,
    },
    #[error("the main menu music file name is invalid: {0}")]
    MusicPath(#[from] AudioKeyError),
    #[error(
        "music {path} failed to decode while playing; the file is damaged, so run \
         the installer again: {source}"
    )]
    MusicStream {
        path: PathBuf,
        source: FromFileError,
    },
    #[error("cannot play the music {path}: {source}")]
    MusicPlayback {
        path: PathBuf,
        source: PlaySoundError<FromFileError>,
    },
    #[error("a music plan has the unusable length of {0} seconds")]
    MusicDuration(f32),
    #[error(transparent)]
    WorldMusic(#[from] WorldMusicError),
    #[error("cannot create the sound effect track: {0}")]
    SfxTrack(ResourceLimitReached),
    #[error(
        "sound effect {path} cannot be opened or decoded; the installer converts it \
         from the game, so run the installer again: {source}"
    )]
    SfxFile {
        path: PathBuf,
        source: FromFileError,
    },
    #[error("cannot play the sound effect {path}: {source}")]
    SfxPlayback {
        path: PathBuf,
        source: PlaySoundError<()>,
    },
    #[error("cannot create the weather sound track: {0}")]
    WeatherTrack(ResourceLimitReached),
    #[error("cannot resolve the surface of a footstep: {0:#}")]
    FootSurface(anyhow::Error),
    #[error(transparent)]
    Footsteps(#[from] FootstepError),
}
