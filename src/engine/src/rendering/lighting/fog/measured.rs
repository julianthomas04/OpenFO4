//! The constants of the fog model measured in vanilla Fallout 4, and the best estimates for what
//! the measurements leave open. Every estimate is a named constant or function here, so a later
//! measurement changes one place.
//!
//! Measured: the colour factors of the Day slot and of every other slot, interior factors, the
//! byte-space blend of the per-slot factors, how a weather's NAM4 scales each slot's far colour
//! factors, the sunlight's share against its luminance in every slot, where the normal branch's
//! opacity has faded in after Near, and where the capped branch ramps to full fog.
//!
//! Estimates:
//! - [`FogScales::ABSENT`]: that a weather without NAM4 scales as one of 1.0 in every entry.
//! - [`DAY_COLOUR_FACTORS`]' high colours: near high is the sunlit value without the sunlight's
//!   share; far high is the fill's (the sunlit normal branch reads 5 % lower).
//! - [`BlendedFogColors`]: that the colours themselves blend as stored bytes, as their factors
//!   measurably do (the colour patterns fit bytes better than linear light, but neither exactly).

use glam::Vec3;

use super::{FogColors, SunlightTerm};
use crate::rendering::lighting::cell_lighting::{DISPLAY_GAMMA, gamma_decode, luminance};
use crate::rendering::lighting::lerp::Lerp;
use crate::rendering::lighting::time_of_day::TimeSlot;

/// The fog colours as time slots and weathers blend them: as stored, scaled to 0 to 1 and by
/// their colour factors, and decoded with gamma 2.2 by [`Self::linear`] only once every blend is
/// done.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct BlendedFogColors(FogColors);

impl Lerp for BlendedFogColors {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        Self(self.0.lerp(&other.0, t))
    }
}

impl BlendedFogColors {
    /// From the stored colours, each channel scaled to 0 to 1, with `factors` applied so that
    /// [`Self::linear`] returns each decoded colour times its factor.
    pub fn from_stored(stored: FogColors, factors: ColourFactors) -> Self {
        let encoded = |factor: f32| factor.powf(DISPLAY_GAMMA.recip());
        Self(FogColors {
            near: stored.near * encoded(factors.near),
            far: stored.far * encoded(factors.far),
            near_high: stored.near_high * encoded(factors.near_high),
            far_high: stored.far_high * encoded(factors.far_high),
        })
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

impl ColourFactors {
    #[cfg(test)]
    pub const UNSCALED: Self = Self::uniform(1.0);

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

/// Every colour, the high ones included, of every slot but Day (night and the sunrise and sunset
/// slots alike) and of interiors.
pub const NON_DAY_COLOUR_FACTORS: ColourFactors = ColourFactors::uniform(0.48);

/// The Day slot's, without the sunlight's share. They hold in every branch: the far factor is
/// also the fill's beyond Far, the far high one the fill's above the height bands.
pub const DAY_COLOUR_FACTORS: ColourFactors = ColourFactors {
    near: 0.47,
    near_high: 0.46,
    far: 1.0,
    far_high: 0.58,
};

/// Interiors take the factors every slot but Day takes outside.
pub const INTERIOR_COLOUR_FACTORS: ColourFactors = NON_DAY_COLOUR_FACTORS;

/// The power of a slot's NAM4 far colour entry the far and far high colour factors follow (free
/// fit over six entries at noon, rms 0.041; the Night entry at 01:00 fits 2.2).
const FAR_COLOUR_SCALE_POWER: f32 = 2.3;

/// The NAM4 bank whose entry for a slot scales that slot's far colour factors. The second bank
/// has a weak effect whose target is unmeasured; it and the first and third banks stay unapplied.
const FAR_COLOUR_SCALE_BANK: usize = 3;

/// The NAM4 far colour entry of each slot of the probe weathers the colour factors above were
/// measured on, copies of `CommonwealthClear`: 1.1 in the Day slot, 1.0 in every other.
fn measured_far_colour_scale(slot: TimeSlot) -> f32 {
    if slot == TimeSlot::Day { 1.1 } else { 1.0 }
}

#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum FogScalesError {
    #[error("NAM4 has {0} entries, expected {expected}", expected = FogScales::LEN)]
    WrongLength(usize),
    #[error("NAM4 entry {index} is not finite ({value})")]
    NotFinite { index: usize, value: f32 },
    #[error("NAM4 far colour scale of {slot} is negative ({value})")]
    NegativeFarColourScale { slot: &'static str, value: f32 },
}

/// A weather's NAM4: four banks of one entry per time slot, each in the game's record order.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct FogScales([f32; FogScales::LEN]);

impl FogScales {
    pub const LEN: usize = 4 * TimeSlot::COUNT;

    /// For a weather without NAM4; unmeasured.
    pub const ABSENT: Self = Self([1.0; Self::LEN]);

    pub fn new(entries: &[f32]) -> Result<Self, FogScalesError> {
        let entries: [f32; Self::LEN] = entries
            .try_into()
            .map_err(|_| FogScalesError::WrongLength(entries.len()))?;
        if let Some((index, &value)) = entries
            .iter()
            .enumerate()
            .find(|(_, value)| !value.is_finite())
        {
            return Err(FogScalesError::NotFinite { index, value });
        }
        let scales = Self(entries);
        if let Some(slot) = TimeSlot::ALL
            .into_iter()
            .find(|&slot| scales.far_colour_scale(slot) < 0.0)
        {
            return Err(FogScalesError::NegativeFarColourScale {
                slot: slot.json_key(),
                value: scales.far_colour_scale(slot),
            });
        }
        Ok(scales)
    }

    fn far_colour_scale(&self, slot: TimeSlot) -> f32 {
        self.0[FAR_COLOUR_SCALE_BANK * TimeSlot::COUNT + slot.index()]
    }
}

/// The Day slot's weight in a blend of slots: 1 for Day, 0 for every other slot.
pub fn slot_day_weight(slot: TimeSlot) -> f32 {
    if slot == TimeSlot::Day { 1.0 } else { 0.0 }
}

/// The factors of `slot`'s colours in a weather with `scales`: the measured ones, with the far
/// and far high factors following the slot's NAM4 far colour entry relative to the measured one.
pub fn slot_colour_factors(slot: TimeSlot, scales: &FogScales) -> ColourFactors {
    let measured = if slot == TimeSlot::Day {
        DAY_COLOUR_FACTORS
    } else {
        NON_DAY_COLOUR_FACTORS
    };
    let far_scale = (scales.far_colour_scale(slot) / measured_far_colour_scale(slot))
        .powf(FAR_COLOUR_SCALE_POWER);
    ColourFactors {
        far: measured.far * far_scale,
        far_high: measured.far_high * far_scale,
        ..measured
    }
}

/// The factors a colour stored alike in every slot shows once the slots are blended with the Day
/// slot at `day_weight`: its decoded colour divided by its decoded stored colour.
pub fn blended_colour_factors(day_weight: f32) -> ColourFactors {
    let white = FogColors::uniform(Vec3::ONE);
    let other = BlendedFogColors::from_stored(white, NON_DAY_COLOUR_FACTORS);
    let day = BlendedFogColors::from_stored(white, DAY_COLOUR_FACTORS);
    let factors = other.lerp(&day, day_weight).linear();
    ColourFactors {
        near: factors.near.x,
        near_high: factors.near_high.x,
        far: factors.far.x,
        far_high: factors.far_high.x,
    }
}

/// The share of the normal branch's fog colour that one unit of sunlight luminance takes away,
/// in whichever slots the sunlight comes from: at the twilight hours without the Day slot the
/// probe's day/night fog fits a share there (rms 0.0015 and 0.0013 under two climates) better
/// than none (0.0033 and 0.0028).
const SUNLIGHT_SHARE_PER_LUMINANCE: f32 = 0.102;

/// `sunlight` is the slot-blended sunlight, linear (decoded with gamma 2.2).
pub fn sunlight_term(sunlight: Vec3) -> SunlightTerm {
    SunlightTerm {
        kept: 1.0 - SUNLIGHT_SHARE_PER_LUMINANCE * luminance(sunlight),
    }
}

/// The share of Near..Far over which the normal branch's opacity fades in from Near, linearly in
/// distance, whatever the power and the span.
const ONSET_SHARE: f32 = 0.015;

/// Where the normal branch's opacity has faded in: from here on the colour weight alone sets it.
pub fn onset_end(near: f32, far: f32) -> f32 {
    near + ONSET_SHARE * (far - near)
}

/// The share of Near..Far at which the capped fog starts ramping to full fog.
const CAP_RAMP_START_SHARE: f32 = 0.766;

/// Where `x` reaches Max only past this distance, the fog jumps up the ramp as the capped branch
/// takes over; whether the vanilla ramp avoids that jump is unmeasured.
pub fn cap_ramp_start(near: f32, far: f32) -> f32 {
    near + CAP_RAMP_START_SHARE * (far - near)
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "the factors are exact at the ends")]
mod tests {
    use super::*;

    fn decoded_grey(byte: f32) -> Vec3 {
        Vec3::splat((byte / 255.0).powf(2.2))
    }

    /// `CommonwealthClear`'s NAM4, the one the colour factors were measured under.
    fn clear_scales() -> [f32; FogScales::LEN] {
        let mut entries = [1.0; FogScales::LEN];
        entries[9] = 1.4;
        entries[25] = 1.1;
        entries
    }

    fn scales(entries: &[f32]) -> FogScales {
        FogScales::new(entries).unwrap()
    }

    #[test]
    fn under_the_measured_nam4_each_slot_takes_its_measured_factors() {
        let clear = scales(&clear_scales());
        for slot in TimeSlot::ALL {
            let (factors, day_weight) = if slot == TimeSlot::Day {
                (DAY_COLOUR_FACTORS, 1.0)
            } else {
                (NON_DAY_COLOUR_FACTORS, 0.0)
            };
            assert_eq!(slot_colour_factors(slot, &clear), factors, "{slot:?}");
            assert_eq!(slot_day_weight(slot), day_weight, "{slot:?}");
        }
    }

    #[test]
    fn a_slots_last_bank_entry_scales_its_far_factors_by_its_power_and_leaves_the_near_ones() {
        let mut entries = clear_scales();
        entries[24 + TimeSlot::Day.index()] = 1.4;
        entries[24 + TimeSlot::Night.index()] = 0.5;
        let foggy = scales(&entries);
        for (slot, measured, ratio) in [
            (TimeSlot::Day, DAY_COLOUR_FACTORS, (1.4_f32 / 1.1).powf(2.3)),
            (TimeSlot::Night, NON_DAY_COLOUR_FACTORS, 0.5_f32.powf(2.3)),
        ] {
            let factors = slot_colour_factors(slot, &foggy);
            assert!(
                (factors.far - measured.far * ratio).abs() < 1e-6,
                "{slot:?}"
            );
            assert!(
                (factors.far_high - measured.far_high * ratio).abs() < 1e-6,
                "{slot:?}"
            );
            assert_eq!(factors.near, measured.near, "{slot:?}");
            assert_eq!(factors.near_high, measured.near_high, "{slot:?}");
        }
        assert_eq!(
            slot_colour_factors(TimeSlot::Sunset, &foggy),
            NON_DAY_COLOUR_FACTORS
        );
    }

    #[test]
    fn the_other_banks_leave_every_factor() {
        let mut entries = clear_scales();
        entries[..24].fill(3.0);
        let clear = scales(&clear_scales());
        for slot in TimeSlot::ALL {
            assert_eq!(
                slot_colour_factors(slot, &scales(&entries)),
                slot_colour_factors(slot, &clear),
                "{slot:?}"
            );
        }
    }

    #[test]
    fn a_weather_without_nam4_scales_as_one_of_all_ones() {
        assert_eq!(FogScales::ABSENT, scales(&[1.0; FogScales::LEN]));
        let day = slot_colour_factors(TimeSlot::Day, &FogScales::ABSENT);
        let ratio = (1.0_f32 / 1.1).powf(2.3);
        assert!((day.far - DAY_COLOUR_FACTORS.far * ratio).abs() < 1e-6);
        assert_eq!(
            slot_colour_factors(TimeSlot::Night, &FogScales::ABSENT),
            NON_DAY_COLOUR_FACTORS
        );
    }

    #[test]
    fn nam4_needs_thirty_two_finite_entries_and_no_negative_far_colour_scale() {
        assert_eq!(
            FogScales::new(&[1.0; 31]),
            Err(FogScalesError::WrongLength(31))
        );
        assert_eq!(FogScales::new(&[]), Err(FogScalesError::WrongLength(0)));
        for value in [f32::NAN, f32::INFINITY] {
            let mut entries = clear_scales();
            entries[3] = value;
            assert!(matches!(
                FogScales::new(&entries),
                Err(FogScalesError::NotFinite { index: 3, .. })
            ));
        }
        let mut entries = clear_scales();
        entries[30] = -0.5;
        assert_eq!(
            FogScales::new(&entries),
            Err(FogScalesError::NegativeFarColourScale {
                slot: "EarlySunset",
                value: -0.5
            })
        );
        entries[30] = 0.0;
        entries[2] = -0.5;
        assert!(FogScales::new(&entries).is_ok());
    }

    #[test]
    fn the_blended_factors_are_the_slots_own_at_the_ends() {
        for (day_weight, factors) in [(0.0, NON_DAY_COLOUR_FACTORS), (1.0, DAY_COLOUR_FACTORS)] {
            let blended = blended_colour_factors(day_weight);
            for (actual, expected) in [
                (blended.near, factors.near),
                (blended.near_high, factors.near_high),
                (blended.far, factors.far),
                (blended.far_high, factors.far_high),
            ] {
                assert!((actual - expected).abs() < 1e-6, "{day_weight}");
            }
        }
    }

    #[test]
    fn the_fill_factor_follows_the_measured_fade_of_the_day_slot() {
        // Hour, Day slot weight under DefaultClimate, measured fill factor (`slots_h*`).
        for (hour, day_weight, measured) in [
            (7.5, 0.0, 0.482),
            (8.0, 1.0 / 3.0, 0.636),
            (8.5, 2.0 / 3.0, 0.805),
            (17.5, 2.0 / 3.0, 0.805),
            (18.0, 1.0 / 3.0, 0.628),
            (18.5, 0.0, 0.482),
        ] {
            let fill = blended_colour_factors(day_weight).far;
            assert!((fill - measured).abs() < 0.006, "{hour}: {fill}");
        }
    }

    #[test]
    fn the_sunlit_noon_factors_are_the_measured_ones() {
        // Sunlight grey byte, then the near and far factors measured at noon.
        for (byte, near, far) in [
            (0.0, 0.470, 0.993),
            (112.0, 0.468, 0.967),
            (225.0, 0.435, 0.907),
            (255.0, 0.423, 0.896),
        ] {
            let kept = sunlight_term(decoded_grey(byte)).kept;
            assert!(
                (DAY_COLOUR_FACTORS.near * kept - near).abs() < 0.007,
                "{byte}"
            );
            assert!(
                (DAY_COLOUR_FACTORS.far * kept - far).abs() < 0.017,
                "{byte}"
            );
        }
    }

    #[test]
    fn coloured_sunlight_takes_its_share_by_its_luminance() {
        let orange = Vec3::new(
            1.0,
            (160.0_f32 / 255.0).powf(2.2),
            (64.0_f32 / 255.0).powf(2.2),
        );
        let kept = sunlight_term(orange).kept;
        assert!(
            (DAY_COLOUR_FACTORS.far * kept - 0.942).abs() < 0.011,
            "{kept}"
        );
    }

    #[test]
    fn the_night_sunlight_keeps_the_night_factor_within_its_measured_fit() {
        let night = Vec3::new(53.0, 70.0, 87.0).map(|byte| (byte / 255.0).powf(2.2));
        let kept = sunlight_term(night).kept;
        assert!(kept < 1.0, "{kept}");
        assert!(
            (NON_DAY_COLOUR_FACTORS.near * kept - 0.48).abs() < 0.005,
            "{kept}"
        );
    }

    #[test]
    fn without_sunlight_the_colour_is_kept_whole() {
        assert_eq!(sunlight_term(Vec3::ZERO), SunlightTerm::NONE);
    }

    #[test]
    fn blended_colours_blend_as_stored_and_decode_afterwards() {
        let black =
            BlendedFogColors::from_stored(FogColors::uniform(Vec3::ZERO), ColourFactors::UNSCALED);
        let white =
            BlendedFogColors::from_stored(FogColors::uniform(Vec3::ONE), ColourFactors::UNSCALED);
        let halfway = black.lerp(&white, 0.5).linear();
        assert!((halfway.far.x - 0.5_f32.powf(2.2)).abs() < 1e-6);
        assert_eq!(white.linear(), FogColors::uniform(Vec3::ONE));
    }

    #[test]
    fn each_decoded_colour_carries_its_own_factor() {
        let stored = FogColors::uniform(Vec3::splat(0.6));
        let decoded = BlendedFogColors::from_stored(stored, DAY_COLOUR_FACTORS).linear();
        let plain = gamma_decode(Vec3::splat(0.6));
        for (actual, factor) in [
            (decoded.near, DAY_COLOUR_FACTORS.near),
            (decoded.near_high, DAY_COLOUR_FACTORS.near_high),
            (decoded.far, DAY_COLOUR_FACTORS.far),
            (decoded.far_high, DAY_COLOUR_FACTORS.far_high),
        ] {
            assert!(actual.abs_diff_eq(plain * factor, 1e-6), "{actual}");
        }
    }

    #[test]
    fn the_capped_fog_ramps_over_the_last_quarter_of_near_to_far() {
        assert!((cap_ramp_start(2400.0, 3400.0) - 3166.0).abs() < 1e-3);
        assert!((cap_ramp_start(0.0, 1000.0) - 766.0).abs() < 1e-3);
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
            (scaled.near_high, 1.38),
            (scaled.far_high, 2.32),
        ] {
            assert!(actual.abs_diff_eq(Vec3::splat(expected), 1e-6), "{actual}");
        }
    }
}
