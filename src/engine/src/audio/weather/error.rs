use crate::audio::conditions::ConditionError;
use crate::audio::file_probe::ProbeError;
use crate::audio::sound_resolve::{LayersError, SoundError};

#[derive(Debug, thiserror::Error)]
pub enum WeatherAudioError {
    #[error("cannot read the weather sound data: {0:#}")]
    Catalog(anyhow::Error),
    #[error(transparent)]
    Sound(#[from] SoundError),
    #[error("sound descriptor {0} contains itself")]
    Cycle(String),
    #[error(transparent)]
    Condition(#[from] ConditionError),
    #[error("weather {0} is not one of the weathers whose sounds were loaded")]
    UnknownWeather(usize),
    #[error(transparent)]
    Probe(#[from] ProbeError),
}

impl From<LayersError> for WeatherAudioError {
    fn from(error: LayersError) -> Self {
        match error {
            LayersError::Cycle(descriptor) => Self::Cycle(descriptor),
            LayersError::Condition(error) => Self::Condition(error),
            LayersError::Sound(error) => Self::Sound(error),
        }
    }
}
