//! The time-of-day driver: the weather slots and game settings, and what they make of a moment
//! of the day. Pure; [`super::state::LightingState`] applies the result.

use std::sync::Arc;

use super::exterior_weather::ExteriorWeather;
use super::exterior_weather::ExteriorWeatherSlots;
use super::time_of_day::unmeasured::sun_alpha;
use super::time_of_day::{
    ClimateMoon, ClimateSunTextures, DayTime, MoonKind, MoonObject, SkyGameConfig, SkyMoment,
    SkyObjects, star_alpha,
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
    sun_textures: ClimateSunTextures,
}

impl SkyClock {
    pub fn new(slots: ExteriorWeatherSlots, game: SkyGameConfig) -> Self {
        let model_path = slots.climate_sky().model_path.as_deref().map(Arc::from);
        let sun_textures = slots.sun_textures().clone();
        Self {
            slots,
            game,
            model_path,
            sun_textures,
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
            sun_textures: self.sun_textures.clone(),
        };
        ClockedSky {
            sun_alpha: sun_alpha(day_time.hour(), times, self.game.sun_alpha_trans_time()),
            moment,
            weather,
            objects,
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::super::exterior_weather::fixtures::weather_slots;
    use super::super::time_of_day::{SkyGameSettings, TimeSlot};
    use super::*;
    use crate::rendering::lighting::exterior_weather::ImageSpaceHdr;

    fn game() -> SkyGameConfig {
        let settings = SkyGameSettings::new(
            600.0,
            -325.0,
            1.0027,
            Vec3::new(-0.09, 0.35, 1.0),
            5.0,
            10.0,
        )
        .unwrap();
        SkyGameConfig::new(settings, 0.15, 90.0, 75.0).unwrap()
    }

    fn clock(textures: ClimateSunTextures) -> SkyClock {
        let slots = weather_slots(|_| {}, |_: TimeSlot| ImageSpaceHdr::default());
        SkyClock::new(slots.with_sun_textures(textures), game())
    }

    #[test]
    fn the_sky_objects_carry_the_climates_sun_textures() {
        let textures = ClimateSunTextures::new(Some(r"Sky\Sun.dds"), Some(r"Sky\SunGlare.dds"));

        let sky = clock(textures.clone()).at(DayTime::clamped(0, 12.0));

        assert_eq!(sky.objects.sun_textures, textures);
    }

    #[test]
    fn a_climate_naming_no_sun_texture_gives_the_sky_none() {
        let sky = clock(ClimateSunTextures::default()).at(DayTime::clamped(0, 12.0));

        assert_eq!(sky.objects.sun_textures, ClimateSunTextures::default());
    }
}
