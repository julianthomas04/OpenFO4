//! The time-of-day driver: the weather slots and game settings, and what they make of a moment
//! of the day. Pure; [`super::state::LightingState`] applies the result.

use std::sync::Arc;

use super::exterior_weather::ExteriorWeather;
use super::exterior_weather::ExteriorWeatherSlots;
use super::time_of_day::unmeasured::sun_alpha;
use super::time_of_day::{
    ClimateMoon, DayTime, MoonKind, MoonObject, SkyGameConfig, SkyMoment, SkyObjects, star_alpha,
};

/// Everything the clock makes of one moment, for the weather it drives.
#[derive(Debug, Clone, PartialEq)]
pub struct ClockedSky {
    pub moment: SkyMoment,
    pub weather: ExteriorWeather,
    /// The sun's alpha around sunrise and sunset, 0 to 1.
    pub sun_alpha: f32,
    pub objects: SkyObjects,
}

#[derive(Debug, Clone)]
pub struct SkyClock {
    slots: ExteriorWeatherSlots,
    game: SkyGameConfig,
    model_path: Option<Arc<str>>,
}

impl SkyClock {
    pub fn new(slots: ExteriorWeatherSlots, game: SkyGameConfig) -> Self {
        let model_path = slots.climate_sky().model_path.as_deref().map(Arc::from);
        Self {
            slots,
            game,
            model_path,
        }
    }

    /// The weather before any clock time is known.
    pub fn initial_weather(&self) -> ExteriorWeather {
        self.slots.initial()
    }

    pub fn game(&self) -> &SkyGameConfig {
        &self.game
    }

    /// The kind of the climate's moon, `None` where it shows none.
    pub fn moon_kind(&self) -> Option<MoonKind> {
        self.slots.moon().map(ClimateMoon::kind)
    }

    /// The climate's sky model, `None` where the database carries none.
    pub fn sky_model_path(&self) -> Option<&Arc<str>> {
        self.model_path.as_ref()
    }

    pub fn at(&self, day_time: DayTime) -> ClockedSky {
        let times = self.slots.climate_times();
        let moment = SkyMoment::at(day_time, times, self.slots.moon(), self.game.settings());
        let weather = self.slots.sample(moment.blend);
        let objects = SkyObjects {
            sun_disc_direction: moment.sun.direction(),
            moon: moment.moon.map(MoonObject::from),
            star_rotation: moment.star_rotation,
            star_alpha: star_alpha(weather.stars),
            sky_model_path: self.model_path.clone(),
        };
        ClockedSky {
            sun_alpha: sun_alpha(day_time.hour(), times, self.game.sun_alpha_trans_time()),
            moment,
            weather,
            objects,
        }
    }
}
