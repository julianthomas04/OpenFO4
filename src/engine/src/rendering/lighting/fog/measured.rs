//! The constants of the fog model measured in vanilla Fallout 4, and the best estimates for what
//! the measurements leave open. Every estimate is a named constant or function here, so a later
//! measurement changes one place.
//!
//! Measured: the colour factors by day without sunlight and at night, the sunlight term at noon,
//! the colour day weight's crossover hours, and the ramp of the capped branch towards full fog.
//!
//! Estimates:
//! - [`NIGHT_FOG_RAMP_HOURS`]: where the night weight of the distances reaches 1 (no samples
//!   between 20:00 and 06:00).
//! - [`sunlight_term`]: that the term scales linearly with the sunlight colour and with the
//!   colour day weight (only noon with black or (225, 225, 225) sunlight was measured).
//! - [`cap_ramp_start`]: that the ramp to full fog starts at a share of Far, not of Near..Far.
//! - [`DAY_COLOUR_FACTORS`]' high colours: the sunlit values without the sunlight term. The fill
//!   beyond Far above the height bands takes the far high colour with its factor (0.55 by day),
//!   as the normal branch does; the fill of a point above the bands was never measured.
//! - [`INTERIOR_COLOUR_FACTORS`]: interiors were not measured at all.
//! - [`BlendedFogColors`]: that time slots and weathers blend the stored fog colour bytes, which
//!   are decoded only afterwards (every slot of the measured weathers was the same).

use glam::Vec3;

use super::{FogColors, SunlightTerm};
use crate::rendering::lighting::cell_lighting::gamma_decode;
use crate::rendering::lighting::lerp::{Lerp, lerp_fields};

/// The fog colours as time slots and weathers blend them: as stored, scaled to 0 to 1, and
/// decoded with gamma 2.2 by [`Self::linear`] only once every blend is done.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct BlendedFogColors(FogColors);

impl Lerp for BlendedFogColors {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        Self(self.0.lerp(&other.0, t))
    }
}

impl BlendedFogColors {
    /// From the stored colours, each channel scaled to 0 to 1.
    pub fn from_stored(stored: FogColors) -> Self {
        Self(stored)
    }

    pub fn linear(self) -> FogColors {
        self.0.map(gamma_decode)
    }
}

/// How strongly each fog colour reaches the picture, relative to the fully fogged far colour at
/// noon.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct ColourFactors {
    pub near: f32,
    pub near_high: f32,
    pub far: f32,
    pub far_high: f32,
}

lerp_fields!(ColourFactors {
    near,
    near_high,
    far,
    far_high
});

impl ColourFactors {
    const fn uniform(factor: f32) -> Self {
        Self {
            near: factor,
            near_high: factor,
            far: factor,
            far_high: factor,
        }
    }

    pub fn apply(self, colors: FogColors) -> FogColors {
        FogColors {
            near: colors.near * self.near,
            far: colors.far * self.far,
            near_high: colors.near_high * self.near_high,
            far_high: colors.far_high * self.far_high,
        }
    }
}

/// Every colour at night, the high ones included.
pub const NIGHT_COLOUR_FACTORS: ColourFactors = ColourFactors::uniform(0.48);

/// By day without the sunlight term. The far factor is also the fill beyond Far and the capped
/// branch's.
pub const DAY_COLOUR_FACTORS: ColourFactors = ColourFactors {
    near: 0.47,
    near_high: 0.47,
    far: 1.0,
    far_high: 0.55,
};

/// Interiors have no sunlight and no time of day; they start from the day factors.
pub const INTERIOR_COLOUR_FACTORS: ColourFactors = DAY_COLOUR_FACTORS;

pub fn colour_factors(day_colour_weight: f32) -> ColourFactors {
    NIGHT_COLOUR_FACTORS.lerp(&DAY_COLOUR_FACTORS, day_colour_weight)
}

/// The share of the normal branch's fog colour the sunlight replaces at noon.
const SUNLIGHT_SHARE: f32 = 0.08;
/// What one unit of linear sunlight adds to the normal branch's fog colour at noon.
const SUNLIGHT_GAIN: f32 = 0.004;

pub fn sunlight_term(sunlight: Vec3, day_colour_weight: f32) -> SunlightTerm {
    SunlightTerm {
        kept: 1.0 - SUNLIGHT_SHARE * day_colour_weight,
        added: sunlight * (SUNLIGHT_GAIN * day_colour_weight),
    }
}

/// Hours over which the night weight of the fog distances ramps from 0 to 1.
pub const NIGHT_FOG_RAMP_HOURS: f32 = 6.0;

/// The share of Far at which the capped fog starts ramping to full fog. Open estimate: where `x`
/// reaches Max only past this distance, the fog jumps up the ramp as the capped branch takes over
/// (19 of 82 vanilla fog halves with Max below 1, for example `GoodneighborWeatherBase` at night
/// by 0.06 at 4600 units). Whether the vanilla ramp avoids that jump is unmeasured.
const CAP_RAMP_START_SHARE_OF_FAR: f32 = 0.8;

pub fn cap_ramp_start(far: f32) -> f32 {
    CAP_RAMP_START_SHARE_OF_FAR * far
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "the factors are exact at the ends")]
mod tests {
    use super::*;

    #[test]
    fn the_colour_factors_are_the_night_ones_at_night_and_the_day_ones_by_day() {
        assert_eq!(colour_factors(0.0), NIGHT_COLOUR_FACTORS);
        assert_eq!(colour_factors(1.0), DAY_COLOUR_FACTORS);
        assert!((colour_factors(0.5).far - 0.74).abs() < 1e-6);
    }

    #[test]
    fn the_sunlit_noon_factors_are_the_measured_ones() {
        let sunlight = Vec3::splat((225.0_f32 / 255.0).powf(2.2));
        let sun = sunlight_term(sunlight, 1.0);
        let sunlit = |factor: f32| factor * sun.kept;
        assert!((sunlit(DAY_COLOUR_FACTORS.near) - 0.435).abs() < 0.01);
        assert!((sunlit(DAY_COLOUR_FACTORS.far) - 0.907).abs() < 0.015);
        assert!((sunlit(DAY_COLOUR_FACTORS.far_high) - 0.51).abs() < 0.01);
        assert!((sun.added.x - 0.0030).abs() < 2e-4);
    }

    #[test]
    fn there_is_no_sunlight_term_at_night() {
        assert_eq!(sunlight_term(Vec3::ONE, 0.0), SunlightTerm::NONE);
    }

    #[test]
    fn blended_colours_blend_as_stored_and_decode_afterwards() {
        let black = BlendedFogColors::from_stored(FogColors::uniform(Vec3::ZERO));
        let white = BlendedFogColors::from_stored(FogColors::uniform(Vec3::ONE));
        let halfway = black.lerp(&white, 0.5).linear();
        assert!((halfway.far.x - 0.5_f32.powf(2.2)).abs() < 1e-6);
        assert_eq!(white.linear(), FogColors::uniform(Vec3::ONE));
    }

    #[test]
    fn the_capped_fog_ramps_over_the_last_fifth_before_far() {
        assert_eq!(cap_ramp_start(3000.0), 2400.0);
    }

    #[test]
    fn factors_scale_each_colour_by_its_own_factor() {
        let colors = FogColors {
            near: Vec3::ONE,
            far: Vec3::splat(2.0),
            near_high: Vec3::splat(3.0),
            far_high: Vec3::splat(4.0),
        };
        let scaled = DAY_COLOUR_FACTORS.apply(colors);
        for (actual, expected) in [
            (scaled.near, 0.47),
            (scaled.far, 2.0),
            (scaled.near_high, 1.41),
            (scaled.far_high, 2.2),
        ] {
            assert!(actual.abs_diff_eq(Vec3::splat(expected), 1e-6), "{actual}");
        }
    }
}
