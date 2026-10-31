//! The weather of the world over time: which weather is current, the transition between two,
//! and the lightning of storms. The cycle follows the world clock and keeps no clock of its own:
//! its times are deadlines in game hours on that clock. What the data fixes comes from
//! [`crate::weather`]; how its bytes become hours and seconds is in `tuning`.

mod cycle;
mod lightning;
mod roll;
#[cfg(test)]
pub(super) mod test_support;
mod tuning;

pub use cycle::{WeatherCycle, WeatherEditError, WeatherSnapshot, WeatherStep};
pub use lightning::LightningStrike;
