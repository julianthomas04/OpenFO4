use super::world_streamer::InteriorRequestError;
use crate::audio::AudioError;
use crate::rendering::device_loss::DeviceLost;
use crate::rendering::shot::ShotError;

/// An error after startup that ends the game: the event loop logs it and exits.
#[derive(Debug, thiserror::Error)]
pub enum FatalRuntimeError {
    #[error(transparent)]
    DeviceLost(#[from] DeviceLost),
    #[error(transparent)]
    Audio(#[from] AudioError),
    #[error("cell streaming workers stopped: {0}")]
    CellWorkersStopped(#[from] InteriorRequestError),
    #[error("screenshot failed: {0}")]
    Shot(#[from] ShotError),
}
