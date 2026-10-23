//! Game audio: the menu and world music and the player footsteps; see `docs/engine/audio-and-video.md`.

mod backend;
mod backend_errors;
pub mod conditions;
mod error;
mod file_probe;
pub mod footsteps;
mod music;
mod music_output;
pub mod music_stack;
pub mod place;
mod plan_run;
pub mod rng;
pub mod scheduler;
mod settings;
mod sfx_output;
mod system;
pub mod world_music;
pub mod world_music_loader;
mod world_playback;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod world_tests;

pub use backend::AudioBackend;
pub use error::AudioError;
pub use footsteps::{FootstepError, Footsteps};
pub use music::{MusicCommand, MusicDirector};
pub use settings::MenuMusicSettings;
pub use system::{AudioFrame, AudioSystem};
pub use world_playback::{PLACE_POLLS_PER_SECOND, PlacePoll};
