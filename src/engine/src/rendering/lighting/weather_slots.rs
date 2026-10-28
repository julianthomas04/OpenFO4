//! One weather's eight time slots and the blend of them for a moment of the day. Pure: no
//! database access, the loader in [`super::exterior_weather`] builds the values.

use std::sync::Arc;

use glam::Vec3;
use serde::Deserialize;

use super::ambient::AmbientCube;
use super::fog::measured::BlendedFogColors;
use super::fog::{ExteriorFog, FogRange};
use super::lerp::{Lerp, lerp_fields, snap_to_nearer};
use super::time_of_day::{
    ClimateMoon, ClimateSky, ClimateSunTextures, ClimateTimes, SlotBlend, TimeSlot,
};
use super::tonemap_settings::ImageSpaceCinematic;
use crate::rendering::post::depth_of_field::ImageSpaceDepthOfField;

/// The slot the menu background and the first frames are lit with.
const INITIAL_SLOT: TimeSlot = TimeSlot::Day;

/// The HDR block of an image space; field meanings follow the Creation Kit
/// image space dialog.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ImageSpaceHdr {
    pub eye_adapt_speed: f32,
    pub sunlight_scale: f32,
    pub sky_scale: f32,
    pub middle_gray: f32,
    pub auto_exposure_min: f32,
    pub auto_exposure_max: f32,
    /// The toe numerator of the filmic tone curve.
    pub tonemap_e: f32,
}

lerp_fields!(ImageSpaceHdr {
    eye_adapt_speed,
    sunlight_scale,
    sky_scale,
    middle_gray,
    auto_exposure_min,
    auto_exposure_max,
    tonemap_e,
});

/// The mode is a bit field, so it snaps to the nearer end instead of blending.
impl Lerp for ImageSpaceDepthOfField {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        Self {
            strength: self.strength.lerp(&other.strength, t),
            distance: self.distance.lerp(&other.distance, t),
            range: self.range.lerp(&other.range, t),
            mode: *snap_to_nearer(&self.mode, &other.mode, t),
            vignette_radius: self.vignette_radius.lerp(&other.vignette_radius, t),
            vignette_strength: self.vignette_strength.lerp(&other.vignette_strength, t),
        }
    }
}

/// `CommonwealthClear`'s values, with neutral light scales, for when no weather loads.
impl Default for ImageSpaceHdr {
    fn default() -> Self {
        Self {
            eye_adapt_speed: 3.0,
            sunlight_scale: 1.0,
            sky_scale: 1.0,
            middle_gray: 0.18,
            auto_exposure_min: 1.6,
            auto_exposure_max: 3.25,
            tonemap_e: 0.02,
        }
    }
}

/// The values of one weather at one time slot that exterior lighting uses, all linear.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SlotValues {
    pub(super) sun_light: Vec3,
    pub(super) sun_disc: Vec3,
    pub(super) sun_glare: Vec3,
    pub(super) sky_upper: Vec3,
    pub(super) sky_lower: Vec3,
    pub(super) horizon: Vec3,
    pub(super) stars: Vec3,
    pub(super) ambient_cube: AmbientCube,
    pub(super) ambient_specular: Vec3,
    pub(super) hdr: ImageSpaceHdr,
    pub(super) cinematic: ImageSpaceCinematic,
    pub(super) depth_of_field: Option<ImageSpaceDepthOfField>,
}

lerp_fields!(SlotValues {
    sun_light,
    sun_disc,
    sun_glare,
    sky_upper,
    sky_lower,
    horizon,
    stars,
    ambient_cube,
    ambient_specular,
    hdr,
    cinematic,
    depth_of_field,
});

/// The colours of a weather's fog in one time slot; the sunlight is linear with gamma 2.2.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct SlotFogColors {
    pub(super) fog: BlendedFogColors,
    pub(super) sunlight: Vec3,
}

lerp_fields!(SlotFogColors { fog, sunlight });

/// A weather's fog. The colours exist for every slot but the distances only for Day and Night,
/// which the time of day weighs later. `None` is a half without fog.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct WeatherFog {
    pub(super) day: Option<FogRange>,
    pub(super) night: Option<FogRange>,
    pub(super) colors: [SlotFogColors; TimeSlot::COUNT],
}

impl WeatherFog {
    fn sample(&self, blend: SlotBlend) -> ExteriorFog {
        let colors = self.colors[blend.from.index()].lerp(&self.colors[blend.to.index()], blend.t);
        ExteriorFog {
            colors: colors.fog,
            sunlight: colors.sunlight,
            day: self.day,
            night: self.night,
        }
    }
}

/// The values of one weather at a moment of the day that exterior lighting uses,
/// all linear.
#[derive(Debug, Clone, PartialEq)]
pub struct ExteriorWeather {
    pub form_key: Arc<str>,
    pub editor_id: Option<Arc<str>>,
    pub sun_light: Vec3,
    pub sun_disc: Vec3,
    /// The colour of the sun's glare sprite.
    pub sun_glare: Vec3,
    pub sky_upper: Vec3,
    pub sky_lower: Vec3,
    pub horizon: Vec3,
    /// The colour of the star layer; its luminance is the layer's opacity.
    pub stars: Vec3,
    pub ambient_cube: AmbientCube,
    /// Extracted but not rendered yet.
    pub ambient_specular: Vec3,
    pub hdr: ImageSpaceHdr,
    pub cinematic: ImageSpaceCinematic,
    /// The image space's depth of field and vignette: what the game's image-space manager
    /// holds for the depth of field effect. `None` when the database predates the export.
    pub depth_of_field: Option<ImageSpaceDepthOfField>,
    /// `None` when the database carries no fog data for the weather. The time of day resolves
    /// it into the fog that renders.
    pub fog: Option<ExteriorFog>,
}

/// The blend of two weathers at one moment. The names have no midpoint, so they snap to the
/// nearer weather (to `self` at exactly one half), as a fog half without fog and the depth of
/// field mode do.
/// Naming every field keeps one added later from being left out.
impl Lerp for ExteriorWeather {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        let nearer = snap_to_nearer(self, other, t);
        Self {
            form_key: Arc::clone(&nearer.form_key),
            editor_id: nearer.editor_id.clone(),
            sun_light: Lerp::lerp(&self.sun_light, &other.sun_light, t),
            sun_disc: Lerp::lerp(&self.sun_disc, &other.sun_disc, t),
            sun_glare: Lerp::lerp(&self.sun_glare, &other.sun_glare, t),
            sky_upper: Lerp::lerp(&self.sky_upper, &other.sky_upper, t),
            sky_lower: Lerp::lerp(&self.sky_lower, &other.sky_lower, t),
            horizon: Lerp::lerp(&self.horizon, &other.horizon, t),
            stars: Lerp::lerp(&self.stars, &other.stars, t),
            ambient_cube: Lerp::lerp(&self.ambient_cube, &other.ambient_cube, t),
            ambient_specular: Lerp::lerp(&self.ambient_specular, &other.ambient_specular, t),
            hdr: Lerp::lerp(&self.hdr, &other.hdr, t),
            cinematic: Lerp::lerp(&self.cinematic, &other.cinematic, t),
            depth_of_field: Lerp::lerp(&self.depth_of_field, &other.depth_of_field, t),
            fog: Lerp::lerp(&self.fog, &other.fog, t),
        }
    }
}

/// One weather with the values of all eight time slots, loaded once, and the climate around
/// it. [`Self::sample`] turns a moment of the day into the weather lighting uses.
#[derive(Debug, Clone, PartialEq)]
pub struct ExteriorWeatherSlots {
    form_key: Arc<str>,
    editor_id: Option<Arc<str>>,
    slots: [SlotValues; TimeSlot::COUNT],
    fog: Option<WeatherFog>,
    climate_times: ClimateTimes,
    climate_sky: ClimateSky,
    moon: Option<ClimateMoon>,
    sun_textures: ClimateSunTextures,
}

impl ExteriorWeatherSlots {
    /// The loader's constructor; `slots` is in the order of [`TimeSlot::index`].
    pub(super) fn new(
        form_key: &str,
        editor_id: Option<&str>,
        slots: [SlotValues; TimeSlot::COUNT],
        fog: Option<WeatherFog>,
        climate_times: ClimateTimes,
        climate_sky: ClimateSky,
        moon: Option<ClimateMoon>,
    ) -> Self {
        Self {
            form_key: form_key.into(),
            editor_id: editor_id.map(Arc::from),
            slots,
            fog,
            climate_times,
            climate_sky,
            moon,
            sun_textures: ClimateSunTextures::default(),
        }
    }

    pub(super) fn with_sun_textures(mut self, sun_textures: ClimateSunTextures) -> Self {
        self.sun_textures = sun_textures;
        self
    }

    #[cfg(test)]
    pub(super) fn without_moon(mut self) -> Self {
        self.moon = None;
        self
    }

    pub fn form_key(&self) -> &str {
        &self.form_key
    }

    pub fn editor_id(&self) -> Option<&str> {
        self.editor_id.as_deref()
    }

    pub fn climate_times(&self) -> &ClimateTimes {
        &self.climate_times
    }

    pub fn climate_sky(&self) -> &ClimateSky {
        &self.climate_sky
    }

    /// The climate's sun and sun glare textures.
    pub fn sun_textures(&self) -> &ClimateSunTextures {
        &self.sun_textures
    }

    /// The climate's moon; `None` where it shows none.
    pub fn moon(&self) -> Option<&ClimateMoon> {
        self.moon.as_ref()
    }

    /// The weather in the slot lighting starts from, before any clock time is known.
    pub fn initial(&self) -> ExteriorWeather {
        self.sample(SlotBlend {
            from: INITIAL_SLOT,
            to: INITIAL_SLOT,
            t: 0.0,
        })
    }

    /// The weather between two neighbouring slots, `blend.t` of the way from the first.
    pub fn sample(&self, blend: SlotBlend) -> ExteriorWeather {
        let from = &self.slots[blend.from.index()];
        let to = &self.slots[blend.to.index()];
        let SlotValues {
            sun_light,
            sun_disc,
            sun_glare,
            sky_upper,
            sky_lower,
            horizon,
            stars,
            ambient_cube,
            ambient_specular,
            hdr,
            cinematic,
            depth_of_field,
        } = from.lerp(to, blend.t);
        ExteriorWeather {
            form_key: Arc::clone(&self.form_key),
            editor_id: self.editor_id.clone(),
            sun_light,
            sun_disc,
            sun_glare,
            sky_upper,
            sky_lower,
            horizon,
            stars,
            ambient_cube,
            ambient_specular,
            hdr,
            cinematic,
            depth_of_field,
            fog: self.fog.as_ref().map(|fog| fog.sample(blend)),
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the blends below land on exactly representable fractions"
)]
mod tests {
    use super::*;
    use crate::rendering::lighting::fog::{FogColors, HeightBands};

    /// A weather whose every value is linear in `level`, with a different slope per field, so a
    /// blend between two levels is the weather at the blended level.
    fn weather(name: &str, level: f32) -> ExteriorWeather {
        ExteriorWeather {
            form_key: name.into(),
            editor_id: Some(format!("{name}Id").as_str().into()),
            sun_light: Vec3::splat(level),
            sun_disc: Vec3::splat(2.0 * level),
            sun_glare: Vec3::splat(3.0 * level),
            sky_upper: Vec3::splat(4.0 * level),
            sky_lower: Vec3::splat(5.0 * level),
            horizon: Vec3::splat(6.0 * level),
            stars: Vec3::splat(7.0 * level),
            ambient_cube: AmbientCube(std::array::from_fn(|face| Vec3::splat(level + face as f32))),
            ambient_specular: Vec3::splat(8.0 * level),
            hdr: ImageSpaceHdr {
                sunlight_scale: 9.0 * level,
                sky_scale: 10.0 * level,
                tonemap_e: 22.0 * level,
                ..ImageSpaceHdr::default()
            },
            cinematic: ImageSpaceCinematic {
                saturation: 23.0 * level,
                brightness: 24.0 * level,
                contrast: 25.0 * level,
                tint_amount: 26.0 * level,
                tint_color: Vec3::splat(27.0 * level),
            },
            depth_of_field: Some(ImageSpaceDepthOfField {
                strength: 11.0 * level,
                distance: 12.0 * level,
                range: 13.0 * level,
                mode: 1,
                vignette_radius: 14.0 * level,
                vignette_strength: 15.0 * level,
            }),
            fog: Some(ExteriorFog {
                colors: BlendedFogColors::from_stored(FogColors::uniform(Vec3::splat(
                    16.0 * level,
                ))),
                sunlight: Vec3::splat(17.0 * level),
                day: Some(FogRange {
                    near: 18.0 * level,
                    far: 19.0 * level,
                    power: 20.0 * level,
                    max: 21.0 * level,
                    height: HeightBands::NONE,
                }),
                night: None,
            }),
        }
    }

    /// `weather` with the names of another.
    fn named(name: &str, weather: &ExteriorWeather) -> ExteriorWeather {
        ExteriorWeather {
            form_key: name.into(),
            editor_id: Some(format!("{name}Id").as_str().into()),
            ..weather.clone()
        }
    }

    #[test]
    fn a_blend_blends_every_value_of_the_two_weathers() {
        let blended = weather("Dim", 0.0).lerp(&weather("Bright", 1.0), 0.25);
        assert_eq!(blended, named("Dim", &weather("Dim", 0.25)));
    }

    #[test]
    fn a_blend_starts_at_the_first_weather_and_ends_at_the_second() {
        let (dim, bright) = (weather("Dim", 0.0), weather("Bright", 1.0));
        assert_eq!(dim.lerp(&bright, 0.0), dim);
        assert_eq!(dim.lerp(&bright, 1.0), bright);
    }

    #[test]
    fn the_depth_of_field_mode_follows_the_nearer_weather_and_the_first_at_exactly_one_half() {
        let with_mode = |mode| ExteriorWeather {
            depth_of_field: weather("Mode", 0.0)
                .depth_of_field
                .map(|dof| ImageSpaceDepthOfField { mode, ..dof }),
            ..weather("Mode", 0.0)
        };
        let (first, second) = (with_mode(1), with_mode(2));
        for (t, mode) in [(0.25, 1), (0.5, 1), (0.75, 2)] {
            let blended = first.lerp(&second, t);
            assert_eq!(blended.depth_of_field.unwrap().mode, mode, "{t}");
        }
    }

    #[test]
    fn the_names_follow_the_nearer_weather_and_the_first_at_exactly_one_half() {
        let (dim, bright) = (weather("Dim", 0.0), weather("Bright", 1.0));
        for (t, name) in [(0.25, "Dim"), (0.5, "Dim"), (0.75, "Bright")] {
            let blended = dim.lerp(&bright, t);
            assert_eq!(&*blended.form_key, name, "{t}");
            assert_eq!(
                blended.editor_id.as_deref(),
                Some(format!("{name}Id").as_str()),
                "{t}"
            );
        }
    }

    #[test]
    fn a_fog_half_the_weathers_do_not_share_snaps_with_the_names_of_the_nearer_one() {
        let foggy = weather("Foggy", 1.0);
        let clear = weather("Clear", 0.0);
        let clear = ExteriorWeather {
            fog: clear.fog.map(|fog| ExteriorFog { day: None, ..fog }),
            ..clear
        };
        let day_of = |weather: ExteriorWeather| weather.fog.and_then(|fog| fog.day);
        assert_eq!(day_of(foggy.lerp(&clear, 0.25)), day_of(foggy.clone()));
        assert_eq!(day_of(foggy.lerp(&clear, 0.75)), None);
    }

    #[test]
    fn a_weather_without_fog_data_keeps_without_it_until_the_midpoint() {
        let unknown = ExteriorWeather {
            fog: None,
            ..weather("Unknown", 0.0)
        };
        let foggy = weather("Foggy", 1.0);
        assert_eq!(unknown.lerp(&foggy, 0.25).fog, None);
        assert_eq!(unknown.lerp(&foggy, 0.75).fog, foggy.fog);
    }
}
