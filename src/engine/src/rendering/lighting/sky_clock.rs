//! The time-of-day driver: the weather slots and game settings, and what they make of a moment
//! of the day. Pure; [`super::state::LightingState`] applies the result.

use std::sync::Arc;

use super::exterior_weather::ExteriorWeather;
use super::exterior_weather::ExteriorWeatherSlots;
use super::time_of_day::unmeasured::sun_alpha;
use super::time_of_day::{
    ClimateMoon, ClimateSunTextures, DayTime, MoonKind, MoonObject, SkyGameConfig, SkyMoment,
    SkyObjects, SlotBlend, slot_blend, star_alpha,
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

    /// Which two slots the weather is blended from at `day_time`, and how far between them.
    pub fn slot_blend(&self, day_time: DayTime) -> SlotBlend {
        slot_blend(day_time, self.slots.climate_times())
    }

    /// The sky at `day_time` with the clock's own weather.
    pub fn at(&self, day_time: DayTime) -> ClockedSky {
        let moment = self.moment(day_time);
        let weather = self.slots.sample(moment.blend);
        self.clocked(day_time, moment, weather)
    }

    /// The sky at `day_time` with `weather` instead of the clock's own: the weather as it was
    /// sampled at [`Self::slot_blend`] of the same time.
    pub fn at_weather(&self, day_time: DayTime, weather: &ExteriorWeather) -> ClockedSky {
        self.clocked(day_time, self.moment(day_time), weather.clone())
    }

    fn moment(&self, day_time: DayTime) -> SkyMoment {
        SkyMoment::at(
            day_time,
            self.slots.climate_times(),
            self.slots.moon(),
            self.game.settings(),
        )
    }

    fn clocked(
        &self,
        day_time: DayTime,
        moment: SkyMoment,
        weather: ExteriorWeather,
    ) -> ClockedSky {
        let objects = SkyObjects {
            sun_disc_direction: moment.sun.direction(),
            moon: moment.moon.map(MoonObject::from),
            star_rotation: moment.star_rotation,
            star_alpha: star_alpha(weather.stars),
            sky_model_path: self.model_path.clone(),
            sun_textures: self.sun_textures.clone(),
        };
        ClockedSky {
            sun_alpha: sun_alpha(
                day_time.hour(),
                self.slots.climate_times(),
                self.game.sun_alpha_trans_time(),
            ),
            moment,
            weather,
            objects,
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "a black stars colour gives exactly no star alpha"
)]
mod tests {
    use glam::Vec3;

    use super::super::exterior_weather::fixtures::weather_slots;
    use super::super::time_of_day::TimeSlot;
    use super::super::time_of_day::test_support::vanilla_game_config;
    use super::*;
    use crate::rendering::lighting::exterior_weather::ImageSpaceHdr;

    fn clock(textures: ClimateSunTextures) -> SkyClock {
        let slots = weather_slots(|_| {}, |_: TimeSlot| ImageSpaceHdr::default());
        SkyClock::new(slots.with_sun_textures(textures), vanilla_game_config())
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

    #[test]
    fn the_slot_blend_is_the_one_the_clocks_own_weather_is_sampled_at() {
        let clock = clock(ClimateSunTextures::default());
        for hour in [0.0, 5.5, 7.25, 12.0, 18.9, 23.9] {
            let time = DayTime::clamped(0, hour);
            assert_eq!(
                clock.slot_blend(time),
                clock.at(time).moment.blend,
                "{hour}"
            );
        }
    }

    #[test]
    fn the_clocks_own_weather_given_back_gives_the_same_sky() {
        let clock = clock(ClimateSunTextures::default());
        for hour in [0.0, 6.5, 12.0, 19.0] {
            let time = DayTime::clamped(2, hour);
            let own = clock.at(time);
            assert_eq!(clock.at_weather(time, &own.weather), own, "{hour}");
        }
    }

    #[test]
    fn a_given_weather_replaces_the_clocks_own_and_the_rest_of_the_sky_stays() {
        let clock = clock(ClimateSunTextures::default());
        let time = DayTime::clamped(0, 12.0);
        let own = clock.at(time);
        assert!(
            own.objects.star_alpha > 0.0,
            "the stars colour of the fixture is not black"
        );
        let given = ExteriorWeather {
            stars: Vec3::ZERO,
            sky_upper: Vec3::new(0.1, 0.2, 0.3),
            ..own.weather.clone()
        };

        let sky = clock.at_weather(time, &given);

        assert_eq!(sky.weather, given);
        assert_eq!(
            sky.objects.star_alpha, 0.0,
            "the stars follow the given weather"
        );
        assert_eq!(sky.moment, own.moment);
        assert_eq!(sky.sun_alpha, own.sun_alpha);
        assert_eq!(sky.objects.moon, own.objects.moon);
        assert_eq!(
            sky.objects.sun_disc_direction,
            own.objects.sun_disc_direction
        );
    }
}
