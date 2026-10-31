//! The weather of the world every frame, on [`State`]: the weather steps on, and what it asks of
//! the grass and the sky is applied to the GPU.

use super::State;
use super::lighting::time_of_day::DayTime;
use super::post::night_sky::NightSkyInput;

impl State {
    pub(super) fn sync_weather(&mut self, world_dt: f32) {
        let clock = self.sim.world_clock();
        let day_time = DayTime::clamped(clock.days_passed(), clock.hour());
        let step = self.weather.step(
            &mut self.lighting,
            self.sim.weather_snapshot(),
            day_time,
            world_dt,
            &self.weather_panel.clouds,
        );
        if let Some(wind) = step.wind {
            self.grass.set_wind(&self.queue, wind);
        }
        if self.lighting.draws_sky()
            && let Some(objects) = self.lighting.sky_objects()
            && let Some(night_sky) = &mut self.night_sky
        {
            let radiance_scale = self.lighting.sky.radiance_scale;
            night_sky.update(
                &self.device,
                &self.queue,
                &NightSkyInput::new(objects, self.lighting.sun_input()),
                radiance_scale,
            );
            night_sky.update_clouds(
                &self.device,
                &self.queue,
                &step.clouds,
                self.lighting.sky.colour_scale(),
                world_dt,
            );
        }
    }
}
