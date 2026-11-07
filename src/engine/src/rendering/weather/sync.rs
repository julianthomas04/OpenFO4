//! The weather of the world every frame: the frame the weather cycle and the time of day make,
//! and what the lighting, the lightning flash, the clouds and the grass take from it. Nothing here
//! touches the GPU: what only the renderer can apply comes back as a [`WeatherStep`].

use glam::Vec3;

use super::frame::{WeatherFrame, WeatherFrameError, WindFrame};
use super::library::WeatherLibrary;
use super::panel::{CloudControls, WeatherView};
use super::{rain, sounds};
use crate::audio::weather::WeatherAudioInput;
use crate::rendering::lighting::LightingState;
use crate::rendering::lighting::cell_lighting::luminance;
use crate::rendering::lighting::lightning::{LightningFlashes, Strike};
use crate::rendering::lighting::time_of_day::DayTime;
use crate::rendering::post::night_sky::CloudsInput;
use crate::rendering::rain::RainInput;
use crate::sim::WeatherSnapshot;
use crate::utils::error_once;

/// The weather of a world. A world without a weather library lights its single weather by the
/// clock alone and keeps the grass wind it was made with.
#[derive(Debug)]
pub struct WeatherSync {
    library: Option<WeatherLibrary>,
    flashes: LightningFlashes,
    /// The frame of the latest step; `None` without a weather library or when it could not be made.
    frame: Option<WeatherFrame>,
}

/// What a step of the weather leaves the renderer to apply, besides the lighting, which the step
/// has taken care of.
#[derive(Debug)]
pub struct WeatherStep<'a> {
    /// The wind the grass blows in; `None` leaves it as it is.
    pub wind: Option<WindFrame>,
    /// What the clouds show; nothing without a frame or with the clouds switched off.
    pub clouds: CloudsInput<'a>,
}

impl WeatherSync {
    pub fn new(library: Option<WeatherLibrary>) -> Self {
        Self {
            library,
            flashes: LightningFlashes::default(),
            frame: None,
        }
    }

    /// The weathers of the climate; `None` while the world keeps its single weather.
    pub fn library(&self) -> Option<&WeatherLibrary> {
        self.library.as_ref()
    }

    /// The frame of the latest step; `None` without a weather library or when it could not be
    /// made.
    pub fn frame(&self) -> Option<&WeatherFrame> {
        self.frame.as_ref()
    }

    /// What the weather panel shows; `None` unless there are weathers and a cycle to show.
    pub fn view<'a>(&'a self, snapshot: Option<WeatherSnapshot<'a>>) -> Option<WeatherView<'a>> {
        Some(WeatherView {
            library: self.library.as_ref()?,
            snapshot: snapshot?,
            frame: self.frame.as_ref(),
        })
    }

    /// Moves the weather one frame on: lights the world with the sky of the cycle's weather at
    /// `day_time`, wets the ground with its rain and flashes the strikes it brings. Call it once a frame, as the strikes of a
    /// snapshot last one frame; the flashes age by `world_dt`, the world seconds of the frame.
    ///
    /// Where no frame can be made, which is the case without a weather library or cycle, with a
    /// sky without a clock and with a cycle that names weathers the library lacks, the world is
    /// lit by the weather of its clock alone and no new strike flashes.
    pub fn step(
        &mut self,
        lighting: &mut LightingState,
        snapshot: Option<WeatherSnapshot<'_>>,
        day_time: DayTime,
        world_dt: f32,
        clouds: &CloudControls,
    ) -> WeatherStep<'_> {
        self.frame = self.make_frame(lighting, snapshot, day_time);
        match &self.frame {
            Some(frame) => lighting.set_time_of_day_with_weather(day_time, &frame.sky),
            None => lighting.set_time_of_day(day_time),
        }
        let strikes: &[Strike] = self.frame.as_ref().map_or(&[], |frame| &frame.strikes);
        let flash = self.flashes.update(world_dt, strikes);
        lighting.set_lightning(flash);
        lighting.set_rain(
            self.frame
                .as_ref()
                .and_then(|frame| frame.precipitation)
                .map_or(0.0, |precipitation| precipitation.intensity),
        );
        let Some(frame) = &self.frame else {
            return WeatherStep {
                wind: None,
                clouds: CloudsInput::default(),
            };
        };
        WeatherStep {
            wind: Some(frame.wind),
            clouds: if clouds.draw {
                // The clouds carry their own colour, so the flash lifts them by how bright it is
                // whatever its colour.
                frame.clouds_input(luminance(flash), clouds.solo())
            } else {
                CloudsInput::default()
            },
        }
    }

    /// What the player hears of the weather at `snapshot`; `None` without a weather library or
    /// cycle, and where the library lacks a weather of the cycle, which is reported once. Call it
    /// once a frame, after the step of the weather, as the strikes of a snapshot last one step.
    pub fn audio_input(
        &self,
        snapshot: Option<WeatherSnapshot<'_>>,
        outdoor: bool,
    ) -> Option<WeatherAudioInput> {
        sounds::audio_input(self.library.as_ref()?, &snapshot?, outdoor)
            .inspect_err(|error| {
                error_once("weather-audio", || {
                    format!("weather sounds not heard, the weather stays silent: {error}")
                });
            })
            .ok()
    }

    /// The rain of the latest frame, in the light of `tint`; `None` while it does not rain or
    /// without a frame. A frame the library cannot make rain of is reported once.
    pub fn rain_input(&self, tint: Vec3) -> Option<RainInput> {
        rain::rain_input(self.frame.as_ref()?, self.library.as_ref()?, tint)
            .inspect_err(|error| {
                error_once("weather-rain", || format!("no rain this frame: {error}"));
            })
            .ok()
            .flatten()
    }

    /// `None` without a weather library, without a weather cycle, without a clocked sky to sample
    /// the weathers at and when the cycle and the library disagree. The cases that are faults are
    /// reported once.
    fn make_frame(
        &self,
        lighting: &LightingState,
        snapshot: Option<WeatherSnapshot<'_>>,
        day_time: DayTime,
    ) -> Option<WeatherFrame> {
        let library = self.library.as_ref()?;
        let Some(snapshot) = snapshot else {
            error_once("weather-cycle-missing", || {
                "the weather library is loaded but the simulation runs no weather cycle; the exterior keeps its single weather".to_owned()
            });
            return None;
        };
        let time = lighting.slot_blend_at(day_time)?;
        WeatherFrame::blend(library, &snapshot, time)
            .inspect_err(report)
            .ok()
    }
}

fn report(error: &WeatherFrameError) {
    let kind = match error {
        WeatherFrameError::UnknownWeather { .. } => "unknown-weather",
        WeatherFrameError::Progress { .. } => "progress",
    };
    error_once(&format!("weather-frame:{kind}"), || {
        format!("weather frame not made, the sky keeps its single weather: {error}")
    });
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::rendering::lighting::cell_lighting::gamma_decode;
    use crate::rendering::lighting::exterior_weather::fixtures::slots_with_sky;
    use crate::rendering::lighting::lightning::tuning;
    use crate::rendering::lighting::state::fixtures::{
        exterior_state, in_neutral_interior, lighting_state,
    };
    use crate::rendering::lighting::time_of_day::TimeSlot;
    use crate::rendering::lighting::wet_ground::landscape_albedo_scale;
    use crate::rendering::weather::frame::fixtures::{
        A, B, dry, settled, transition, weather, with_sky,
    };
    use crate::rendering::weather::library::fixtures::library_of;
    use crate::sim::LightningStrike;
    use crate::weather::WeatherId;

    type Rgb = (u8, u8, u8);

    const CLOCK_DAY: Rgb = (255, 0, 0);
    const CLOCK_NIGHT: Rgb = (255, 255, 0);
    const A_DAY: Rgb = (0, 255, 0);
    const A_NIGHT: Rgb = (0, 100, 0);
    const B_SKY: Rgb = (0, 0, 255);

    fn by_day_and_night(day: Rgb, night: Rgb) -> impl Fn(TimeSlot) -> Rgb {
        move |slot| if slot == TimeSlot::Night { night } else { day }
    }

    /// A sky colour as the weather decodes it.
    fn linear((red, green, blue): Rgb) -> Vec3 {
        gamma_decode(Vec3::new(red.into(), green.into(), blue.into()) / 255.0)
    }

    /// The weathers `A` and `B`; `A` is green by day and dark green by night.
    fn library() -> WeatherLibrary {
        library_of(vec![
            with_sky(weather("A"), by_day_and_night(A_DAY, A_NIGHT)),
            with_sky(weather("B"), |_| B_SKY),
        ])
    }

    /// An exterior whose own weather, the one of its clock, is red by day and yellow by night.
    fn lighting() -> LightingState {
        exterior_state(slots_with_sky(
            "Clock",
            "Clock",
            by_day_and_night(CLOCK_DAY, CLOCK_NIGHT),
        ))
    }

    fn noon() -> DayTime {
        DayTime::clamped(0, 12.0)
    }

    fn midnight() -> DayTime {
        DayTime::clamped(0, 0.0)
    }

    /// The light a strike of `intensity` flashes at its peak: the fixture weathers are all alike
    /// in their lightning colour.
    fn peak_light(intensity: f32) -> Vec3 {
        let strike = Strike::from_srgb8(intensity, weather("A").lightning.color_srgb);
        strike.colour * strike.intensity
    }

    fn strike(weather: WeatherId, flash_intensity: f32) -> LightningStrike {
        LightningStrike {
            weather,
            distance_m: 900.0,
            thunder_delay_s: 2.6,
            flash_intensity,
        }
    }

    fn with_strikes<'a>(
        snapshot: WeatherSnapshot<'a>,
        strikes: &'a [LightningStrike],
    ) -> WeatherSnapshot<'a> {
        WeatherSnapshot {
            strikes,
            ..snapshot
        }
    }

    fn step<'a>(
        sync: &'a mut WeatherSync,
        lighting: &mut LightingState,
        snapshot: Option<WeatherSnapshot<'_>>,
        world_dt: f32,
    ) -> WeatherStep<'a> {
        sync.step(
            lighting,
            snapshot,
            noon(),
            world_dt,
            &CloudControls::default(),
        )
    }

    /// Lets `strikes` strike in a frame of `snapshot` and the flashes rise to their peak in the
    /// next.
    fn strike_and_peak(
        sync: &mut WeatherSync,
        lighting: &mut LightingState,
        snapshot: WeatherSnapshot<'static>,
        strikes: &[LightningStrike],
    ) {
        step(sync, lighting, Some(with_strikes(snapshot, strikes)), 0.0);
        step(sync, lighting, Some(snapshot), tuning::ATTACK_S);
    }

    fn sky_upper(lighting: &LightingState) -> Vec3 {
        lighting.exterior_weather().unwrap().sky_upper
    }

    fn assert_close(actual: Vec3, expected: Vec3) {
        assert!(
            actual.abs_diff_eq(expected, 1e-6),
            "{actual} is not {expected}"
        );
    }

    #[test]
    fn the_sky_is_that_of_the_weather_of_the_cycle_instead_of_that_of_the_clock() {
        let mut sync = WeatherSync::new(Some(library()));
        let mut lighting = lighting();
        for (snapshot, expected) in [(settled(A), A_DAY), (settled(B), B_SKY)] {
            step(&mut sync, &mut lighting, Some(snapshot), 0.0);
            assert_close(sky_upper(&lighting), linear(expected));
            assert_eq!(
                lighting.exterior_weather(),
                Some(&sync.frame().unwrap().sky)
            );
        }
    }

    #[test]
    fn the_sky_is_that_of_the_cycle_at_the_time_of_day_it_is_given() {
        let mut sync = WeatherSync::new(Some(library()));
        let mut lighting = lighting();
        sync.step(
            &mut lighting,
            Some(settled(A)),
            midnight(),
            0.0,
            &CloudControls::default(),
        );
        assert_close(sky_upper(&lighting), linear(A_NIGHT));
        assert!(lighting.sky_moment().is_some());
    }

    #[test]
    fn without_a_weather_library_the_world_is_lit_by_its_clock_and_has_no_weather_to_apply() {
        let mut sync = WeatherSync::new(None);
        let mut lighting = lighting();
        let strikes = [strike(A, 1.0)];
        let snapshot = with_strikes(settled(A), &strikes);
        let output = sync.step(
            &mut lighting,
            Some(snapshot),
            midnight(),
            tuning::ATTACK_S,
            &CloudControls::default(),
        );
        assert_eq!(output.wind, None);
        assert_eq!(output.clouds, CloudsInput::default());
        assert!(sync.frame().is_none());
        assert_close(sky_upper(&lighting), linear(CLOCK_NIGHT));
        assert!(lighting.sky_moment().is_some());
        assert_eq!(lighting.lightning(), Vec3::ZERO);
    }

    #[test]
    fn without_a_weather_cycle_the_world_is_lit_by_its_clock_and_has_no_weather_to_apply() {
        let mut sync = WeatherSync::new(Some(library()));
        let mut lighting = lighting();
        let output = sync.step(
            &mut lighting,
            None,
            midnight(),
            0.0,
            &CloudControls::default(),
        );
        assert_eq!(output.wind, None);
        assert_eq!(output.clouds, CloudsInput::default());
        assert!(sync.frame().is_none());
        assert_close(sky_upper(&lighting), linear(CLOCK_NIGHT));
        assert!(lighting.sky_moment().is_some());
    }

    #[test]
    fn a_sky_without_a_clock_has_no_weather_to_sample_and_no_frame() {
        let mut sync = WeatherSync::new(Some(library()));
        let mut lighting = lighting_state();
        let output = step(&mut sync, &mut lighting, Some(settled(A)), 0.0);
        assert_eq!(output.wind, None);
        assert_eq!(output.clouds, CloudsInput::default());
        assert!(sync.frame().is_none());
    }

    #[test]
    fn a_cycle_that_names_a_weather_the_library_lacks_makes_no_frame_and_flashes_no_strike() {
        let mut sync = WeatherSync::new(Some(library()));
        let mut lighting = lighting();
        let strikes = [strike(A, 1.0)];
        let unknown = with_strikes(settled(WeatherId::new(9)), &strikes);
        let output = step(&mut sync, &mut lighting, Some(unknown), 0.0);
        assert_eq!(output.wind, None);
        assert_eq!(output.clouds, CloudsInput::default());
        assert!(sync.frame().is_none());
        assert_close(sky_upper(&lighting), linear(CLOCK_DAY));
        step(&mut sync, &mut lighting, Some(settled(A)), tuning::ATTACK_S);
        assert_eq!(lighting.lightning(), Vec3::ZERO);
    }

    #[test]
    fn the_ground_is_as_wet_as_the_frame_rains_and_dry_without_rain() {
        let mut sync = WeatherSync::new(Some(library_of(vec![weather("A"), dry(weather("B"))])));
        let mut lighting = lighting();
        step(&mut sync, &mut lighting, Some(settled(A)), 0.0);
        let rain = sync.frame().unwrap().precipitation.unwrap().intensity;
        assert!(rain > 0.0, "{rain}");
        assert_eq!(
            lighting.landscape_albedo_scale(),
            landscape_albedo_scale(rain)
        );
        step(&mut sync, &mut lighting, Some(settled(B)), 0.0);
        assert_eq!(lighting.landscape_albedo_scale(), 1.0);
    }

    #[test]
    fn the_wind_is_that_of_the_frame() {
        let mut sync = WeatherSync::new(Some(library()));
        let mut lighting = lighting();
        let wind = step(&mut sync, &mut lighting, Some(settled(A)), 0.0).wind;
        assert_eq!(wind, Some(WindFrame::of(&weather("A").wind)));
        assert_eq!(wind, Some(sync.frame().unwrap().wind));
    }

    #[test]
    fn a_strike_flashes_the_exterior_once_at_its_intensity_in_the_colour_of_its_weather() {
        let mut sync = WeatherSync::new(Some(library()));
        let mut lighting = lighting();
        let strikes = [strike(A, 0.5)];
        step(
            &mut sync,
            &mut lighting,
            Some(with_strikes(settled(A), &strikes)),
            0.0,
        );
        assert_eq!(lighting.lightning(), Vec3::ZERO);
        step(&mut sync, &mut lighting, Some(settled(A)), tuning::ATTACK_S);
        assert_close(lighting.lightning(), peak_light(0.5));
    }

    #[test]
    fn the_strikes_of_one_frame_flash_together_each_once() {
        let mut sync = WeatherSync::new(Some(library()));
        let mut lighting = lighting();
        let strikes = [strike(A, 0.5), strike(B, 0.25)];
        strike_and_peak(&mut sync, &mut lighting, transition(B, A, 0.5), &strikes);
        assert_close(lighting.lightning(), peak_light(0.5) + peak_light(0.25));
    }

    #[test]
    fn the_flashes_age_by_the_world_seconds_and_stand_still_while_none_pass() {
        let mut sync = WeatherSync::new(Some(library()));
        let mut lighting = lighting();
        let strikes = [strike(A, 1.0)];
        step(
            &mut sync,
            &mut lighting,
            Some(with_strikes(settled(A), &strikes)),
            0.0,
        );
        for _ in 0..3 {
            step(&mut sync, &mut lighting, Some(settled(A)), 0.0);
            assert_eq!(lighting.lightning(), Vec3::ZERO);
        }
        step(&mut sync, &mut lighting, Some(settled(A)), tuning::ATTACK_S);
        assert_close(lighting.lightning(), peak_light(1.0));
        step(
            &mut sync,
            &mut lighting,
            Some(settled(A)),
            tuning::FLASH_LENGTH_S,
        );
        assert_eq!(lighting.lightning(), Vec3::ZERO);
    }

    #[test]
    fn a_flash_goes_on_while_the_frame_cannot_be_made() {
        let mut sync = WeatherSync::new(Some(library()));
        let mut lighting = lighting();
        let strikes = [strike(A, 1.0)];
        step(
            &mut sync,
            &mut lighting,
            Some(with_strikes(settled(A), &strikes)),
            0.0,
        );
        step(&mut sync, &mut lighting, None, tuning::ATTACK_S);
        assert!(sync.frame().is_none());
        assert_close(lighting.lightning(), peak_light(1.0));
    }

    #[test]
    fn the_flash_lights_no_interior_but_goes_on_when_the_player_steps_out() {
        let mut sync = WeatherSync::new(Some(library()));
        let mut lighting = in_neutral_interior(lighting());
        let strikes = [strike(A, 1.0)];
        strike_and_peak(&mut sync, &mut lighting, settled(A), &strikes);
        assert_eq!(lighting.lightning(), Vec3::ZERO);
        lighting.sync_environment(None, None);
        step(&mut sync, &mut lighting, Some(settled(A)), 0.0);
        assert_close(lighting.lightning(), peak_light(1.0));
    }

    #[test]
    fn the_clouds_are_those_of_the_frame_lifted_by_the_luminance_of_the_flash() {
        let mut sync = WeatherSync::new(Some(library()));
        let mut lighting = lighting();
        let strikes = [strike(A, 1.0)];
        strike_and_peak(&mut sync, &mut lighting, settled(A), &strikes);
        let output = step(&mut sync, &mut lighting, Some(settled(A)), 0.0);
        let clouds = output.clouds;
        assert_eq!(clouds.solo, None);
        assert!(clouds.flash > 0.0);
        assert_eq!(clouds.flash, luminance(lighting.lightning()));
        let layers = clouds.layers.into_owned();
        assert_ne!(layers, []);
        assert_eq!(layers, sync.frame().unwrap().cloud_layers);
    }

    #[test]
    fn the_clouds_leave_out_what_the_controls_switch_off_and_keep_what_they_solo() {
        let mut sync = WeatherSync::new(Some(library()));
        let mut lighting = lighting();
        let output = sync.step(
            &mut lighting,
            Some(settled(A)),
            noon(),
            0.0,
            &CloudControls::hidden(),
        );
        assert_eq!(output.clouds, CloudsInput::default());
        assert!(output.wind.is_some());
        let output = sync.step(
            &mut lighting,
            Some(settled(A)),
            noon(),
            0.0,
            &CloudControls::soloing(5),
        );
        assert_eq!(output.clouds.solo, Some(5));
        assert!(!output.clouds.layers.is_empty());
    }

    #[test]
    fn the_panel_has_something_to_show_only_with_weathers_and_a_cycle() {
        let mut sync = WeatherSync::new(Some(library()));
        assert!(sync.view(None).is_none());
        assert!(WeatherSync::new(None).view(Some(settled(A))).is_none());
        let mut lighting = lighting();
        step(&mut sync, &mut lighting, Some(settled(A)), 0.0);
        let view = sync.view(Some(settled(B))).unwrap();
        assert_eq!(view.snapshot.current, B);
        assert_eq!(view.frame, sync.frame());
        assert_eq!(view.library, sync.library().unwrap());
    }
}
