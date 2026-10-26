//! One weather's eight time slots and the blend of them for a moment of the day. Pure: no
//! database access, the loader in [`super::exterior_weather`] builds the values.

use std::sync::Arc;

use glam::Vec3;
use serde::Deserialize;

use super::ambient::AmbientCube;
use super::fog::{Fog, FogParams, HeightBand, HeightFog};
use super::god_rays::SunInScatter;
use super::lerp::{Lerp, lerp_fields, snap_to_nearer};
use super::time_of_day::{
    ClimateMoon, ClimateSky, ClimateSunTextures, ClimateTimes, SlotBlend, TimeSlot,
    fog_daylight_weight,
};
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
}

lerp_fields!(ImageSpaceHdr {
    eye_adapt_speed,
    sunlight_scale,
    sky_scale,
    middle_gray,
    auto_exposure_min,
    auto_exposure_max,
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
    pub(super) depth_of_field: Option<ImageSpaceDepthOfField>,
    pub(super) sun_in_scatter: SunInScatter,
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
    depth_of_field,
    sun_in_scatter,
});

/// A fog distance set in engine units, with the height bands and density scale of its height
/// fog.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct FogRange {
    pub(super) near: f32,
    pub(super) far: f32,
    pub(super) power: f32,
    pub(super) max: f32,
    pub(super) near_band: HeightBand,
    pub(super) far_band: HeightBand,
    pub(super) high_density_scale: f32,
}

lerp_fields!(FogRange {
    near,
    far,
    power,
    max,
    near_band,
    far_band,
    high_density_scale
});

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct FogColors {
    pub(super) near: Vec3,
    pub(super) far: Vec3,
    pub(super) near_high: Vec3,
    pub(super) far_high: Vec3,
}

lerp_fields!(FogColors {
    near,
    far,
    near_high,
    far_high
});

/// A weather's fog. The colours exist for every slot but the distances only for Day and
/// Night, which the sample blends by how much daylight the moment has. `None` is a slot without fog.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct WeatherFog {
    pub(super) day: Option<FogRange>,
    pub(super) night: Option<FogRange>,
    pub(super) colors: [FogColors; TimeSlot::COUNT],
}

impl WeatherFog {
    /// A fog that is on at one end of the day and off at the other cannot be blended, so it
    /// snaps at the midpoint of the daylight weight (to Night at exactly one half).
    fn sample(&self, blend: SlotBlend) -> Fog {
        let colors = self.colors[blend.from.index()].lerp(&self.colors[blend.to.index()], blend.t);
        let range = self.night.lerp(&self.day, fog_daylight_weight(&blend));
        match range {
            Some(range) => Fog::Enabled(FogParams {
                near_color: colors.near,
                far_color: colors.far,
                near: range.near,
                far: range.far,
                power: range.power,
                max: range.max,
                height: HeightFog {
                    near_high_color: colors.near_high,
                    far_high_color: colors.far_high,
                    near_band: range.near_band,
                    far_band: range.far_band,
                    high_density_scale: range.high_density_scale,
                },
            }),
            None => Fog::Disabled,
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
    /// The image space's depth of field and vignette: what the game's image-space manager
    /// holds for the depth of field effect. `None` when the database predates the export.
    pub depth_of_field: Option<ImageSpaceDepthOfField>,
    /// `None` when the database carries no fog data for the weather;
    /// `Fog::Disabled` when the weather explicitly has no fog.
    pub fog: Option<Fog>,
    /// The sun's light the fog scatters towards the eye, from the weather's god rays;
    /// [`SunInScatter::NONE`] where the weather has none.
    pub sun_in_scatter: SunInScatter,
}

/// The blend of two weathers at one moment. The names have no midpoint, so they snap to the
/// nearer weather (to `self` at exactly one half), as the fog and the depth of field mode do.
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
            depth_of_field: Lerp::lerp(&self.depth_of_field, &other.depth_of_field, t),
            fog: Lerp::lerp(&self.fog, &other.fog, t),
            sun_in_scatter: Lerp::lerp(&self.sun_in_scatter, &other.sun_in_scatter, t),
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
            depth_of_field,
            sun_in_scatter,
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
            depth_of_field,
            fog: self.fog.as_ref().map(|fog| fog.sample(blend)),
            sun_in_scatter,
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
                ..ImageSpaceHdr::default()
            },
            depth_of_field: Some(ImageSpaceDepthOfField {
                strength: 11.0 * level,
                distance: 12.0 * level,
                range: 13.0 * level,
                mode: 1,
                vignette_radius: 14.0 * level,
                vignette_strength: 15.0 * level,
            }),
            fog: Some(Fog::Enabled(FogParams {
                near_color: Vec3::splat(16.0 * level),
                far_color: Vec3::splat(17.0 * level),
                near: 18.0 * level,
                far: 19.0 * level,
                power: 20.0 * level,
                max: 21.0 * level,
                height: HeightFog::NONE,
            })),
            sun_in_scatter: SunInScatter {
                forward: Vec3::splat(22.0 * level),
                back: Vec3::splat(23.0 * level),
                air: Vec3::splat(24.0 * level),
                forward_asymmetry: 0.5 * level,
                back_asymmetry: -0.25 * level,
            },
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
    fn a_fog_the_weathers_do_not_share_snaps_with_the_names_of_the_nearer_one() {
        let foggy = weather("Foggy", 1.0);
        let clear = ExteriorWeather {
            fog: Some(Fog::Disabled),
            ..weather("Clear", 0.0)
        };
        assert_eq!(foggy.lerp(&clear, 0.25).fog, foggy.fog);
        assert_eq!(foggy.lerp(&clear, 0.75).fog, Some(Fog::Disabled));
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
