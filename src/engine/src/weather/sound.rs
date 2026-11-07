use serde::Deserialize;

/// The situation a weather sound plays in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundKind {
    Default,
    Precipitation,
    Wind,
    Thunder,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeatherSound {
    pub kind: SoundKind,
    /// Form key of the sound descriptor.
    pub descriptor: String,
}
