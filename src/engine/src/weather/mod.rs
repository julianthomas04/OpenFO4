//! Weather domain data shared by the simulation, the renderer and audio: what a weather is made
//! of and what a climate can pick, as plain values. Nothing here knows the database, the GPU or
//! the simulation.

pub mod fade;
mod flag_set;
pub mod flags;
pub mod id;
pub mod lightning;
pub mod roll_table;
pub mod sound;
pub mod wind;

pub use fade::{FadeRange, TransitionSide};
pub use flags::{WeatherFlag, WeatherFlags};
pub use id::{ClimateEntry, WeatherId};
pub use lightning::LightningParams;
pub use roll_table::{WeatherRollTable, WeatherRollTraits, WeatherTableError};
pub use sound::{SoundKind, WeatherSound};
pub use wind::WindParams;
