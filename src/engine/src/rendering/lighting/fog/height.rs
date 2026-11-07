use crate::rendering::lighting::lerp::{Lerp, lerp_fields};
use crate::rendering::world_streamer::SCALE;

/// A height band of the fog, in engine units: its share of the high colours and density rises
/// linearly from `mid - range` to `mid + range`. `weight` scales that share; it is 0 for a band
/// the data leaves unset, so a blend towards such a band fades the height fog out instead of
/// jumping.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct HeightBand {
    pub mid: f32,
    pub range: f32,
    pub weight: f32,
}

/// A band that is unset at one end has no heights to blend towards, so the blend keeps the set
/// end's heights and fades only the weight.
impl Lerp for HeightBand {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        let weight = self.weight.lerp(&other.weight, t);
        match (self.is_set(), other.is_set()) {
            (true, false) => Self { weight, ..*self },
            (false, true) => Self { weight, ..*other },
            (true, true) | (false, false) => Self {
                mid: self.mid.lerp(&other.mid, t),
                range: self.range.lerp(&other.range, t),
                weight,
            },
        }
    }
}

impl HeightBand {
    pub const UNSET: Self = Self {
        mid: 0.0,
        range: 0.0,
        weight: 0.0,
    };

    fn is_set(self) -> bool {
        self.weight > 0.0
    }

    /// A band without a positive, finite range (records that predate height fog store 0) is
    /// unset.
    pub fn new(mid: f32, range: f32) -> Self {
        if mid.is_finite() && range.is_finite() && range > 0.0 {
            Self {
                mid,
                range,
                weight: 1.0,
            }
        } else {
            Self::UNSET
        }
    }

    /// The band of a record's mid and range in Bethesda units: unset when both are absent, as in
    /// databases that predate height fog, and `None` when only one is.
    fn from_bethesda_units(mid: Option<f32>, range: Option<f32>) -> Option<Self> {
        match (mid, range) {
            (Some(mid), Some(range)) => Some(Self::new(mid * SCALE, range * SCALE)),
            (None, None) => Some(Self::UNSET),
            (Some(_), None) | (None, Some(_)) => None,
        }
    }

    /// The lowest and highest heights of the band: its share is 0 below the first and all of
    /// its weight above the second.
    pub fn span(self) -> (f32, f32) {
        (self.mid - self.range, self.mid + self.range)
    }

    /// `(scale, offset)` with `share = weight * saturate(height * scale + offset)`; an unset band
    /// has `(0, 0)`, a share of 0 at every height.
    pub fn share_line(self) -> (f32, f32) {
        if self.is_set() && self.range > 0.0 {
            let scale = (2.0 * self.range).recip();
            (scale, 0.5 - self.mid * scale)
        } else {
            (0.0, 0.0)
        }
    }

    /// How far `height` is into the band, scaled by the band's weight.
    pub fn share(self, height: f32) -> f32 {
        let (scale, offset) = self.share_line();
        self.weight * (height * scale + offset).clamp(0.0, 1.0)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, thiserror::Error)]
pub enum HeightFogError {
    #[error("the {band} fog height band has a mid or range without the other")]
    IncompleteBand { band: &'static str },
    #[error("the fog has a negative high density scale ({scale})")]
    NegativeHighDensityScale { scale: f32 },
}

/// The height-fog fields of a record, in Bethesda units; data that predates height fog has none
/// of them.
#[derive(Debug, Copy, Clone, Default, PartialEq)]
pub struct RawHeightFog {
    pub near_mid: Option<f32>,
    pub near_range: Option<f32>,
    pub far_mid: Option<f32>,
    pub far_range: Option<f32>,
    pub high_density_scale: Option<f32>,
}

/// The colourless part of the height fog: its two bands and how dense the fog is at their top.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct HeightBands {
    pub near: HeightBand,
    pub far: HeightBand,
    pub high_density_scale: f32,
}

lerp_fields!(HeightBands {
    near,
    far,
    high_density_scale
});

impl HeightBands {
    /// Fog that is the same at every height.
    pub const NONE: Self = Self {
        near: HeightBand::UNSET,
        far: HeightBand::UNSET,
        high_density_scale: 1.0,
    };

    /// The one height share of both fog colours and the density: the near band's share where
    /// the colour weight `x` is 0, the far band's where it is 1.
    pub fn share(&self, height: f32, x: f32) -> f32 {
        self.near.share(height).lerp(&self.far.share(height), x)
    }

    /// How much of the fog stays at `share`: 1 at the bottom, the high density scale at the top.
    pub fn density(&self, share: f32) -> f32 {
        1.0.lerp(&self.high_density_scale, share)
    }
}

impl RawHeightFog {
    /// The bands in engine units. A missing high density scale is 1, the same fog at every
    /// height.
    pub fn bands(self) -> Result<HeightBands, HeightFogError> {
        let band = |mid, range, band| {
            HeightBand::from_bethesda_units(mid, range)
                .ok_or(HeightFogError::IncompleteBand { band })
        };
        let high_density_scale = self.high_density_scale.unwrap_or(1.0);
        if high_density_scale < 0.0 {
            return Err(HeightFogError::NegativeHighDensityScale {
                scale: high_density_scale,
            });
        }
        Ok(HeightBands {
            near: band(self.near_mid, self.near_range, "near")?,
            far: band(self.far_mid, self.far_range, "far")?,
            high_density_scale,
        })
    }
}

#[cfg(test)]
pub(in crate::rendering::lighting) mod test_support {
    use super::HeightBand;

    pub(in crate::rendering::lighting) fn assert_band(band: HeightBand, mid: f32, range: f32) {
        assert!(
            (band.mid - mid).abs() < 1e-3 && (band.range - range).abs() < 1e-3,
            "{band:?} is not mid {mid} range {range}"
        );
        assert!((band.weight - 1.0).abs() < f32::EPSILON, "{band:?}");
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "the shares below are exact in f32")]
mod tests {
    use super::*;

    /// The probe's height rig: both bands at mid 17304, range 500 (Bethesda units).
    fn probe_band() -> HeightBand {
        HeightBand::new(1730.4, 50.0)
    }

    #[test]
    fn the_share_rises_linearly_over_mid_plus_and_minus_range() {
        let band = HeightBand::new(100.0, 20.0);
        assert_eq!(band.span(), (80.0, 120.0));
        for (height, share) in [
            (0.0, 0.0),
            (80.0, 0.0),
            (100.0, 0.5),
            (110.0, 0.75),
            (500.0, 1.0),
        ] {
            assert!((band.share(height) - share).abs() < 1e-6, "{height}");
        }
    }

    #[test]
    fn the_share_follows_the_probes_measured_near_high_profile() {
        let band = probe_band();
        for (bethesda_z, measured) in [
            (16_766.0, 0.000),
            (16_926.0, 0.111),
            (17_252.0, 0.415),
            (17_421.0, 0.634),
            (17_596.0, 0.828),
            (17_967.0, 1.000),
        ] {
            let share = band.share(bethesda_z * SCALE);
            assert!((share - measured).abs() < 0.04, "z {bethesda_z}: {share}");
        }
    }

    #[test]
    fn the_density_follows_the_probes_measured_half_density_profile() {
        let bands = HeightBands {
            near: probe_band(),
            far: probe_band(),
            high_density_scale: 0.5,
        };
        for (bethesda_z, measured_ratio) in [
            (16_609.0, 1.000),
            (16_926.0, 0.941),
            (17_087.0, 0.853),
            (17_421.0, 0.685),
            (17_777.0, 0.508),
            (18_167.0, 0.502),
        ] {
            let ratio = bands.density(bands.share(bethesda_z * SCALE, 0.3));
            assert!(
                (ratio - measured_ratio).abs() < 0.025,
                "z {bethesda_z}: {ratio}"
            );
        }
    }

    #[test]
    fn the_shared_share_is_the_near_bands_at_no_colour_weight_and_the_far_bands_at_full() {
        let bands = HeightBands {
            near: HeightBand::new(0.0, 10.0),
            far: HeightBand::new(100.0, 10.0),
            high_density_scale: 1.0,
        };
        assert_eq!(bands.share(5.0, 0.0), 0.75);
        assert_eq!(bands.share(5.0, 1.0), 0.0);
        assert_eq!(bands.share(5.0, 0.5), 0.375);
    }

    #[test]
    fn an_unset_band_has_no_share_at_any_height() {
        for height in [-1.0e6, 0.0, 1.0e6] {
            assert_eq!(HeightBand::UNSET.share(height), 0.0);
        }
        assert_eq!(HeightBand::UNSET.share_line(), (0.0, 0.0));
    }

    #[test]
    fn the_share_line_reproduces_the_share() {
        let band = HeightBand::new(64.0, 1000.0);
        let (scale, offset) = band.share_line();
        for height in [-2000.0, 0.0, 500.0, 3000.0] {
            let share = (height * scale + offset).clamp(0.0, 1.0);
            assert!((share - band.share(height)).abs() < 1e-6, "{height}");
        }
    }

    #[test]
    fn a_band_without_a_positive_finite_range_is_unset() {
        for (mid, range) in [
            (5.0, 0.0),
            (5.0, -1.0),
            (f32::NAN, 1.0),
            (5.0, f32::INFINITY),
        ] {
            assert_eq!(
                HeightBand::new(mid, range),
                HeightBand::UNSET,
                "{mid} {range}"
            );
        }
    }

    #[test]
    fn a_blend_towards_an_unset_band_keeps_the_set_bands_heights_and_fades_its_share() {
        let band = HeightBand::new(40.0, 20.0);
        for halfway in [
            band.lerp(&HeightBand::UNSET, 0.5),
            HeightBand::UNSET.lerp(&band, 0.5),
        ] {
            assert_eq!(
                halfway,
                HeightBand {
                    weight: 0.5,
                    ..band
                }
            );
            assert_eq!(halfway.share(100.0), 0.5);
        }
    }

    #[test]
    fn a_blend_of_two_set_bands_blends_their_heights() {
        let halfway = HeightBand::new(40.0, 20.0).lerp(&HeightBand::new(80.0, 60.0), 0.5);
        assert_eq!(halfway, HeightBand::new(60.0, 40.0));
    }

    fn raw_height_fog() -> RawHeightFog {
        RawHeightFog {
            near_mid: Some(500.0),
            near_range: Some(2000.0),
            far_mid: Some(1800.0),
            far_range: Some(15_000.0),
            high_density_scale: Some(0.95),
        }
    }

    #[test]
    fn raw_height_fog_turns_into_bands_in_engine_units() {
        let bands = raw_height_fog().bands().unwrap();
        test_support::assert_band(bands.near, 50.0, 200.0);
        test_support::assert_band(bands.far, 180.0, 1500.0);
        assert_eq!(bands.high_density_scale, 0.95);
    }

    #[test]
    fn height_fog_from_before_its_export_is_the_same_at_every_height() {
        assert_eq!(RawHeightFog::default().bands(), Ok(HeightBands::NONE));
    }

    #[test]
    fn a_mid_without_its_range_or_a_range_without_its_mid_names_the_band() {
        let near = RawHeightFog {
            near_range: None,
            ..raw_height_fog()
        };
        let far = RawHeightFog {
            far_mid: None,
            ..raw_height_fog()
        };
        assert_eq!(
            near.bands(),
            Err(HeightFogError::IncompleteBand { band: "near" })
        );
        assert_eq!(
            far.bands(),
            Err(HeightFogError::IncompleteBand { band: "far" })
        );
    }

    #[test]
    fn a_negative_high_density_scale_is_rejected() {
        let raw = RawHeightFog {
            high_density_scale: Some(-0.5),
            ..raw_height_fog()
        };
        assert_eq!(
            raw.bands(),
            Err(HeightFogError::NegativeHighDensityScale { scale: -0.5 })
        );
    }
}
