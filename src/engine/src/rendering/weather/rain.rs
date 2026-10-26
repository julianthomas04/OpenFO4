//! What the weather asks of the rain: the rain of a frame from the weather frame and the library,
//! and the ripples the rain stirs in the water around the player. Nothing here touches the GPU.

use glam::Vec3;

use super::frame::WeatherFrame;
use super::library::WeatherLibrary;
use crate::bethesda_space::UNITS_TO_ENGINE;
use crate::random::SplitMix64;
use crate::rendering::rain::impacts::rain_impacts;
use crate::rendering::rain::{RainInput, wind_drift};
use crate::rendering::water::displacement::state::QUAD_SIZE;
use crate::weather::{WeatherFlag, WeatherId};

/// How far from the player the rain stirs the water: as far as the water displacement takes an
/// impact, in engine units.
const RIPPLE_RANGE: f32 = QUAD_SIZE * UNITS_TO_ENGINE;

/// What keeps the rain of a frame from being made. The frame never shows precipitation for a
/// weather without any, so this is a disagreement between the frame and the library.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum RainInputError {
    #[error("the weather frame shows the precipitation of {weather:?}, which the library lacks")]
    UnknownWeather { weather: WeatherId },
    #[error(
        "the weather frame shows the precipitation of {weather:?}, which has no particle geometry"
    )]
    NoParticleGeometry { weather: WeatherId },
}

/// The rain of the frame; `None` while no weather shows precipitation. `tint` is the linear
/// colour the rain catches, in the radiance the scene is lit with, lightning flash included.
pub fn rain_input(
    frame: &WeatherFrame,
    library: &WeatherLibrary,
    tint: Vec3,
) -> Result<Option<RainInput>, RainInputError> {
    let Some(precipitation) = frame.precipitation else {
        return Ok(None);
    };
    let weather = precipitation.weather;
    let definition = library
        .get(weather)
        .ok_or(RainInputError::UnknownWeather { weather })?;
    let params = definition
        .precipitation
        .as_ref()
        .ok_or(RainInputError::NoParticleGeometry { weather })?;
    Ok(Some(RainInput {
        intensity: precipitation.intensity,
        occludable: precipitation.flags.contains(WeatherFlag::RainOcclusion),
        fall_speed: params.gravity_velocity,
        size: params.particle_size,
        subtextures: params.subtextures,
        density: params.particle_density,
        box_size: params.box_size as f32,
        texture: params.texture.clone(),
        wind: wind_drift(frame.wind.speed, frame.wind.heading),
        tint,
    }))
}

/// Where the rain strikes the water around the player.
#[derive(Debug)]
pub struct Ripples {
    rng: SplitMix64,
}

impl Ripples {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: SplitMix64::new(seed),
        }
    }

    /// Appends the spots of the water entries the rain of a frame of `dt` seconds and `intensity`
    /// makes within [`RIPPLE_RANGE`] of `origin`: the impacts that fall on water, at the height of
    /// its surface. `surface_height` is the height of the water at an engine-space point, `None`
    /// where there is none.
    pub fn append_entries(
        &mut self,
        entries: &mut Vec<Vec3>,
        dt: f32,
        intensity: f32,
        origin: Vec3,
        surface_height: impl Fn(Vec3) -> Option<f32>,
    ) {
        let impacts = rain_impacts(&mut self.rng, dt, intensity, origin, RIPPLE_RANGE);
        entries.extend(impacts.filter_map(|impact| {
            surface_height(impact).map(|height| Vec3::new(impact.x, height, impact.z))
        }));
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "the copied values of the tests are exact")]
mod tests {
    use glam::Vec2;

    use super::*;
    use crate::rendering::weather::PrecipitationFrame;
    use crate::rendering::weather::WindFrame;
    use crate::rendering::weather::frame::fixtures::{A, B, DAY, dry, library, settled, weather};
    use crate::rendering::weather::library::fixtures::library_of;
    use crate::weather::WeatherFlags;

    const TINT: Vec3 = Vec3::new(0.4, 0.5, 0.6);

    fn frame_of(library: &WeatherLibrary, id: WeatherId) -> WeatherFrame {
        WeatherFrame::blend(library, &settled(id), DAY).unwrap()
    }

    fn input(library: &WeatherLibrary, frame: &WeatherFrame) -> RainInput {
        rain_input(frame, library, TINT).unwrap().unwrap()
    }

    #[test]
    fn a_rainy_weather_falls_as_its_particle_geometry_says() {
        let library = library();
        let frame = frame_of(&library, A);
        let params = library.get(A).unwrap().precipitation.clone().unwrap();

        let rain = input(&library, &frame);

        assert_eq!(rain.intensity, frame.precipitation.unwrap().intensity);
        assert_eq!(rain.fall_speed, params.gravity_velocity);
        assert_eq!(rain.size, params.particle_size);
        assert_eq!(rain.subtextures, params.subtextures);
        assert_eq!(rain.density, params.particle_density);
        assert_eq!(rain.box_size, params.box_size as f32);
        assert_eq!(rain.tint, TINT);
    }

    #[test]
    fn the_texture_is_named_below_the_textures_folder() {
        let library = library();

        let rain = input(&library, &frame_of(&library, A));

        assert_eq!(rain.texture.as_deref(), Some("rain.dds"));
    }

    #[test]
    fn a_weather_without_a_texture_names_none() {
        let mut without = weather("A");
        without.precipitation.as_mut().unwrap().texture = None;
        let library = library_of(vec![without]);

        let rain = input(&library, &frame_of(&library, A));

        assert_eq!(rain.texture, None);
    }

    #[test]
    fn roofs_shelter_from_the_rain_of_a_weather_that_says_so() {
        let library = library();
        let mut frame = frame_of(&library, A);
        let precipitation = frame.precipitation.unwrap();
        let mut with_flag = |flag: WeatherFlag| {
            frame.precipitation = Some(PrecipitationFrame {
                flags: WeatherFlags::from_bits(flag.bit().into()).unwrap(),
                ..precipitation
            });
            input(&library, &frame).occludable
        };

        assert!(with_flag(WeatherFlag::RainOcclusion));
        assert!(!with_flag(WeatherFlag::Rainy));
    }

    #[test]
    fn a_dry_weather_has_no_rain() {
        let library = library_of(vec![dry(weather("A"))]);

        assert_eq!(rain_input(&frame_of(&library, A), &library, TINT), Ok(None));
    }

    #[test]
    fn precipitation_the_library_does_not_know_is_an_error() {
        let library = library();
        let mut frame = frame_of(&library, A);
        let unknown = WeatherId::new(9);
        frame.precipitation = Some(PrecipitationFrame {
            weather: unknown,
            ..frame.precipitation.unwrap()
        });

        assert_eq!(
            rain_input(&frame, &library, TINT),
            Err(RainInputError::UnknownWeather { weather: unknown })
        );
    }

    #[test]
    fn precipitation_of_a_weather_without_particle_geometry_is_an_error() {
        let library = library_of(vec![weather("A"), dry(weather("B"))]);
        let mut frame = frame_of(&library, A);
        frame.precipitation = Some(PrecipitationFrame {
            weather: B,
            ..frame.precipitation.unwrap()
        });

        assert_eq!(
            rain_input(&frame, &library, TINT),
            Err(RainInputError::NoParticleGeometry { weather: B })
        );
    }

    #[test]
    fn the_rain_drifts_with_the_wind_of_the_frame() {
        let library = library();
        let mut frame = frame_of(&library, A);
        frame.wind = WindFrame {
            speed: 0.6,
            turbulence: 0.0,
            heading: 1.0,
        };

        let rain = input(&library, &frame);

        assert_eq!(rain.wind, wind_drift(0.6, 1.0));
        assert_ne!(rain.wind, Vec2::ZERO);
    }

    fn entries(dt: f32, intensity: f32, surface: impl Fn(Vec3) -> Option<f32>) -> Vec<Vec3> {
        let mut entries = vec![Vec3::splat(-1.0)];
        Ripples::new(5).append_entries(
            &mut entries,
            dt,
            intensity,
            Vec3::new(10.0, 50.0, 20.0),
            surface,
        );
        entries.split_off(1)
    }

    #[test]
    fn rain_over_water_stirs_ripples_at_the_surface() {
        let entries = entries(0.25, 1.0, |_| Some(3.0));

        assert_ne!(entries, []);
        assert!(entries.iter().all(|entry| entry.y == 3.0));
    }

    #[test]
    fn ripples_keep_to_the_range_around_the_origin() {
        let origin = Vec3::new(10.0, 50.0, 20.0);
        let range = RIPPLE_RANGE;

        let entries = entries(0.25, 1.0, |_| Some(0.0));

        assert!(entries.iter().all(|entry| {
            Vec2::new(entry.x - origin.x, entry.z - origin.z).length() <= range + 1e-3
        }));
    }

    #[test]
    fn ripples_rise_only_where_there_is_water() {
        let entries = entries(0.25, 1.0, |point| (point.x > 10.0).then_some(0.0));

        assert_ne!(entries, []);
        assert!(entries.iter().all(|entry| entry.x > 10.0));
    }

    #[test]
    fn rain_that_falls_on_dry_land_or_not_at_all_stirs_nothing() {
        assert_eq!(entries(0.25, 1.0, |_| None), []);
        assert_eq!(entries(0.25, 0.0, |_| Some(0.0)), []);
        assert_eq!(entries(0.0, 1.0, |_| Some(0.0)), []);
    }
}
