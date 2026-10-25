//! Pure time-of-day maths for exterior lighting and the sky: slot blending,
//! sun, moon and star placement. No GPU and no database access; the rules that
//! are still hypotheses live in [`unmeasured`].

mod blend;
mod climate_sky;
mod climate_times;
mod day_time;
mod light;
mod moon;
mod sky;
mod slot;
mod stars;
#[cfg(test)]
mod test_support;
pub mod unmeasured;
mod wrap;

pub use blend::{SlotBlend, fog_daylight_weight, slot_blend};
pub use climate_sky::{ClimateSky, ClimateSkyError, Moons};
pub use climate_times::{ClimateTimes, ClimateTimesError, ClimateWindow};
pub use day_time::{DayTime, DayTimeError};
pub use light::LightSource;
pub use moon::{MoonError, MoonKind, MoonPhase, moon_phase, moon_visibility, texture_stem};
pub use sky::{SkyGameSettings, SkyPosition, SkySettingsError};
pub use slot::TimeSlot;
pub use stars::{StarRotation, star_rotation};
pub use unmeasured::{light_source, moon_position, star_alpha, sun_position};
