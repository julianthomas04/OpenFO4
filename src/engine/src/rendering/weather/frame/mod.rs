//! The weather of one frame: what the weather cycle and the time of day make of the weathers of
//! the library, as plain values for the sky, the clouds, the rain, the grass and the sound to read.
//! While one weather turns into another the frame blends the two; a settled weather is itself.
//! Nothing here touches the GPU or the lighting state.

mod clouds;
mod precipitation;
mod sides;
mod wind;

use std::borrow::Cow;

use self::clouds::cloud_layers;
use self::precipitation::{precipitation, thunder_intensity};
use self::sides::{Sides, weather_of};
pub use self::{precipitation::PrecipitationFrame, wind::WindFrame};
use super::library::WeatherLibrary;
use crate::rendering::lighting::exterior_weather::ExteriorWeather;
use crate::rendering::lighting::lightning::Strike;
use crate::rendering::lighting::time_of_day::SlotBlend;
use crate::rendering::post::night_sky::{CloudLayerDraw, CloudsInput};
use crate::sim::WeatherSnapshot;
use crate::weather::WeatherId;

/// What keeps a frame from being made. Both mean the cycle and the library disagree, which
/// they never do when the cycle was made from the library.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum WeatherFrameError {
    #[error(
        "the weather cycle names {weather:?}, which is none of the {count} weathers of the library"
    )]
    UnknownWeather { weather: WeatherId, count: usize },
    #[error("the transition has come {progress}, which is not a fraction from 0 to 1")]
    Progress { progress: f32 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct WeatherFrame {
    /// The lighting of the sky at the time of day.
    pub sky: ExteriorWeather,
    /// The layers of the clouds, back to front.
    pub cloud_layers: Vec<CloudLayerDraw>,
    pub precipitation: Option<PrecipitationFrame>,
    /// `0..=1`.
    pub thunder_intensity: f32,
    pub wind: WindFrame,
    /// The lightning that strikes this frame, in the colour of the weather it comes from.
    pub strikes: Vec<Strike>,
}

impl WeatherFrame {
    /// The frame of the weather `snapshot` at the time of day `time` stands for. A settled
    /// weather is sampled as it is, so the sky is what the weather alone makes of the time of
    /// day. During a transition both weathers are sampled at the same time of day and blended by
    /// how far the transition has come.
    pub fn blend(
        library: &WeatherLibrary,
        snapshot: &WeatherSnapshot<'_>,
        time: SlotBlend,
    ) -> Result<Self, WeatherFrameError> {
        let sides = Sides::of(library, snapshot)?;
        let strikes = snapshot
            .strikes
            .iter()
            .map(|strike| {
                let weather = weather_of(library, strike.weather)?;
                Ok(Strike::from_srgb8(
                    strike.flash_intensity,
                    weather.lightning.color_srgb,
                ))
            })
            .collect::<Result<_, WeatherFrameError>>()?;
        Ok(Self {
            sky: sides.blend(|weather| weather.slots.sample(time)),
            cloud_layers: cloud_layers(&sides, time),
            precipitation: precipitation(&sides),
            thunder_intensity: thunder_intensity(&sides),
            wind: sides.blend(|weather| WindFrame::of(&weather.wind)),
            strikes,
        })
    }

    /// What the clouds of the frame show, with `flash` brightening them and `solo` leaving only
    /// that layer.
    pub fn clouds_input(&self, flash: f32, solo: Option<u8>) -> CloudsInput<'_> {
        CloudsInput {
            layers: Cow::Borrowed(&self.cloud_layers),
            flash,
            solo,
        }
    }
}

#[cfg(test)]
pub(super) mod fixtures;
#[cfg(test)]
mod tests;
