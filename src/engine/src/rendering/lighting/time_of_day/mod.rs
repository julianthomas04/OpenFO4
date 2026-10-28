//! Pure time-of-day maths for exterior lighting and the sky: slot blending,
//! sun disc, sun light, moon and star placement. No GPU and no database access; the rules that
//! are still hypotheses live in [`unmeasured`].

mod blend;
mod climate_sky;
mod climate_times;
mod day_time;
mod game_config;
mod keyframes;
mod light;
mod moment;
mod moon;
mod sky;
mod sky_objects;
mod slot;
mod solar_turn;
mod stars;
mod sun_disc;
mod sun_light;
#[cfg(test)]
pub(crate) mod test_support;
pub mod unmeasured;
mod wrap;

pub use blend::{SlotBlend, slot_blend};
pub use climate_sky::{ClimateSky, ClimateSkyError, ClimateSunTextures, Moons};
pub use climate_times::{ClimateTimes, ClimateTimesError, ClimateWindow};
pub use day_time::{DayTime, DayTimeError};
pub use game_config::{SkyGameConfig, SkyGameConfigError};
pub use keyframes::night_weight;
pub use light::LightSource;
pub use moment::{ClimateMoon, ClimateMoonError, MoonSky, SkyMoment};
pub use moon::{MoonError, MoonKind, MoonPhase, moon_phase, moon_visibility, texture_stem};
pub use sky::{SkyGameSettings, SkyPosition, SkySettingsError};
pub use sky_objects::{MoonObject, SkyObjects};
pub use slot::TimeSlot;
pub use stars::{StarRotation, star_rotation};
pub use sun_disc::sun_position;
pub use sun_light::light_source;
pub use unmeasured::{moon_position, star_alpha, sun_light_weight};
