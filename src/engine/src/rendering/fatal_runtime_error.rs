use crate::audio::AudioError;
use crate::rendering::device_loss::DeviceLost;

/// An error after startup that ends the game: the event loop logs it and exits.
#[derive(Debug, thiserror::Error)]
pub enum FatalRuntimeError {
    #[error(transparent)]
    DeviceLost(#[from] DeviceLost),
    #[error(transparent)]
    Audio(#[from] AudioError),
}
