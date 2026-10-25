//! One weather's eight time slots and the blend of them for a moment of the day. Pure: no
//! database access, the loader in [`super::exterior_weather`] builds the values.

use std::sync::Arc;

use glam::Vec3;
use serde::Deserialize;

use super::ambient::AmbientCube;
use super::fog::{Fog, FogParams};
use super::lerp::{Lerp, lerp_fields};
use super::time_of_day::{
    ClimateMoon, ClimateSky, ClimateSunTextures, ClimateTimes, SlotBlend, TimeSlot,
    fog_daylight_weight,
};
use crate::rendering::post::depth_of_field::ImageSpaceDepthOfField;

pub(super) const SLOT_COUNT: usize = TimeSlot::ALL.len();
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
            mode: if t > 0.5 { other.mode } else { self.mode },
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
});

/// A fog distance set in engine units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct FogRange {
    pub(super) near: f32,
    pub(super) far: f32,
    pub(super) power: f32,
    pub(super) max: f32,
}

lerp_fields!(FogRange {
    near,
    far,
    power,
    max
});

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct FogColors {
    pub(super) near: Vec3,
    pub(super) far: Vec3,
}

lerp_fields!(FogColors { near, far });

/// A weather's fog. The colours exist for every slot but the distances only for Day and
/// Night, which the sample blends by how much daylight the moment has. `None` is a slot without fog.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct WeatherFog {
    pub(super) day: Option<FogRange>,
    pub(super) night: Option<FogRange>,
    pub(super) colors: [FogColors; SLOT_COUNT],
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
}

/// One weather with the values of all eight time slots, loaded once, and the climate around
/// it. [`Self::sample`] turns a moment of the day into the weather lighting uses.
#[derive(Debug, Clone, PartialEq)]
pub struct ExteriorWeatherSlots {
    form_key: Arc<str>,
    editor_id: Option<Arc<str>>,
    slots: [SlotValues; SLOT_COUNT],
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
        slots: [SlotValues; SLOT_COUNT],
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
        }
    }
}
