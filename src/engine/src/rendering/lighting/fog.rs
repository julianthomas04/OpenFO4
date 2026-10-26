use glam::{Vec3, Vec4};

use super::lerp::{Lerp, lerp_fields, snap_to_nearer};
use crate::rendering::world_streamer::SCALE;

/// Bethesda-style parametric fog in engine units and linear colours; shared by
/// the exterior fog settings and interior cell lighting.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct FogParams {
    pub near_color: Vec3,
    pub far_color: Vec3,
    pub near: f32,
    pub far: f32,
    pub power: f32,
    pub max: f32,
    pub height: HeightFog,
}

lerp_fields!(FogParams {
    near_color,
    far_color,
    near,
    far,
    power,
    max,
    height
});

/// Heights, in engine units, over which a fog colour turns from its low to its high colour:
/// all low below `mid - range`, all high above `mid + range`. `weight` is how far towards the
/// high colour the band goes; it is 0 for a band the data leaves unset, so a blend towards such
/// a band fades the height colour out instead of jumping.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct HeightBand {
    pub mid: f32,
    pub range: f32,
    pub weight: f32,
}

lerp_fields!(HeightBand { mid, range, weight });

impl HeightBand {
    pub const UNSET: Self = Self {
        mid: 0.0,
        range: 0.0,
        weight: 0.0,
    };

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
    pub fn from_bethesda_units(mid: Option<f32>, range: Option<f32>) -> Option<Self> {
        match (mid, range) {
            (Some(mid), Some(range)) => Some(Self::new(mid * SCALE, range * SCALE)),
            (None, None) => Some(Self::UNSET),
            (Some(_), None) | (None, Some(_)) => None,
        }
    }

    /// The lowest height of the band and the reciprocal of its span, for
    /// `saturate((height - bottom) * inverse_span)`; `(0, 0)` holds that factor at 0.
    pub fn bottom_and_inverse_span(self) -> (f32, f32) {
        if self.weight > 0.0 && self.range > 0.0 {
            (self.mid - self.range, 0.5 / self.range)
        } else {
            (0.0, 0.0)
        }
    }
}

/// How the fog changes with the height of the fogged point: the near colour turns towards
/// `near_high_color` over `near_band`, the far colour towards `far_high_color` over `far_band`,
/// and the opacity scales towards `high_density_scale` over `far_band`.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct HeightFog {
    pub near_high_color: Vec3,
    pub far_high_color: Vec3,
    pub near_band: HeightBand,
    pub far_band: HeightBand,
    pub high_density_scale: f32,
}

lerp_fields!(HeightFog {
    near_high_color,
    far_high_color,
    near_band,
    far_band,
    high_density_scale
});

impl HeightFog {
    /// Fog that is the same at every height.
    pub const NONE: Self = Self {
        near_high_color: Vec3::ZERO,
        far_high_color: Vec3::ZERO,
        near_band: HeightBand::UNSET,
        far_band: HeightBand::UNSET,
        high_density_scale: 1.0,
    };

    fn map_colors(self, color: impl Fn(Vec3) -> Vec3) -> Self {
        Self {
            near_high_color: color(self.near_high_color),
            far_high_color: color(self.far_high_color),
            ..self
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum Fog {
    Disabled,
    Enabled(FogParams),
}

/// Fog that is on at one end and off at the other cannot be blended, so it snaps at the midpoint
/// (to `self` at exactly one half), as a fog that is on at one end of the day only does.
impl Lerp for Fog {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        match (self, other) {
            (Fog::Enabled(from), Fog::Enabled(to)) => Fog::Enabled(from.lerp(to, t)),
            _ => *snap_to_nearer(self, other, t),
        }
    }
}

/// The `FrameUniform` fog fields, laid out as `fog_near_color`, `fog_far_color`, `fog_params`,
/// `fog_near_high_color`, `fog_far_high_color` and `fog_heights` in `shaders/frame.wgsl`.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct FogUniformFields {
    /// rgb: colour, w: distance at which the fog starts.
    pub near_color: Vec4,
    /// rgb: colour, w: distance at which the fog is fully developed.
    pub far_color: Vec4,
    /// x: falloff power, y: maximum opacity, z: opacity scale at the top of the far band.
    pub params: Vec4,
    /// rgb: the near colour at the top of the near band, the band's weight folded in.
    pub near_high_color: Vec4,
    /// rgb: the far colour at the top of the far band, the band's weight folded in.
    pub far_high_color: Vec4,
    /// The bottom and inverse span of the near band (xy) and of the far band (zw).
    pub heights: Vec4,
}

impl FogUniformFields {
    /// Zero maximum opacity, so `apply_fog` leaves the colour untouched.
    const DISABLED: Self = Self {
        near_color: Vec4::ZERO,
        far_color: Vec4::ZERO,
        params: Vec4::ZERO,
        near_high_color: Vec4::ZERO,
        far_high_color: Vec4::ZERO,
        heights: Vec4::ZERO,
    };
}

impl Fog {
    /// The fog with `light` added to all of its colours; where it begins and ends and how it
    /// builds up stay. Disabled fog has no colour to lift.
    #[must_use]
    pub fn brightened(self, light: Vec3) -> Self {
        self.map_colors(|color| color + light)
    }

    /// The fog with all of its colours multiplied by `scale`.
    #[must_use]
    pub fn scaled(self, scale: f32) -> Self {
        self.map_colors(|color| color * scale)
    }

    fn map_colors(self, color: impl Fn(Vec3) -> Vec3) -> Self {
        match self {
            Fog::Enabled(fog) => Fog::Enabled(FogParams {
                near_color: color(fog.near_color),
                far_color: color(fog.far_color),
                height: fog.height.map_colors(color),
                ..fog
            }),
            Fog::Disabled => Fog::Disabled,
        }
    }

    /// A fog range that does not increase has no meaningful gradient and is
    /// treated as disabled.
    pub fn uniform_fields(&self) -> FogUniformFields {
        match *self {
            Fog::Enabled(fog) if fog.far > fog.near => {
                let height = fog.height;
                let (near_bottom, near_inverse_span) = height.near_band.bottom_and_inverse_span();
                let (far_bottom, far_inverse_span) = height.far_band.bottom_and_inverse_span();
                let far_weight = height.far_band.weight;
                FogUniformFields {
                    near_color: fog.near_color.extend(fog.near),
                    far_color: fog.far_color.extend(fog.far),
                    params: Vec4::new(
                        fog.power,
                        fog.max,
                        1.0.lerp(&height.high_density_scale, far_weight),
                        0.0,
                    ),
                    near_high_color: fog
                        .near_color
                        .lerp(height.near_high_color, height.near_band.weight)
                        .extend(0.0),
                    far_high_color: fog
                        .far_color
                        .lerp(height.far_high_color, far_weight)
                        .extend(0.0),
                    heights: Vec4::new(
                        near_bottom,
                        near_inverse_span,
                        far_bottom,
                        far_inverse_span,
                    ),
                }
            }
            Fog::Enabled(_) | Fog::Disabled => FogUniformFields::DISABLED,
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "the packed values are exact in f32")]
mod tests {
    use super::*;

    fn params(near: f32, far: f32) -> FogParams {
        FogParams {
            near_color: Vec3::new(0.1, 0.2, 0.3),
            far_color: Vec3::new(0.4, 0.5, 0.6),
            near,
            far,
            power: 2.0,
            max: 0.5,
            height: HeightFog::NONE,
        }
    }

    fn height_fog() -> HeightFog {
        HeightFog {
            near_high_color: Vec3::new(0.7, 0.8, 0.9),
            far_high_color: Vec3::new(1.0, 1.1, 1.2),
            near_band: HeightBand::new(20.0, 10.0),
            far_band: HeightBand::new(50.0, 25.0),
            high_density_scale: 0.5,
        }
    }

    fn with_height(height: HeightFog) -> Fog {
        Fog::Enabled(FogParams {
            height,
            ..params(10.0, 100.0)
        })
    }

    #[test]
    fn enabled_fog_packs_colours_with_their_distances_and_power_with_max() {
        let fields = Fog::Enabled(params(10.0, 100.0)).uniform_fields();
        assert_eq!(fields.near_color, Vec4::new(0.1, 0.2, 0.3, 10.0));
        assert_eq!(fields.far_color, Vec4::new(0.4, 0.5, 0.6, 100.0));
        assert_eq!(fields.params, Vec4::new(2.0, 0.5, 1.0, 0.0));
    }

    #[test]
    fn fog_without_height_fog_repeats_its_colours_as_the_high_ones_and_holds_the_height_factor() {
        let fields = Fog::Enabled(params(10.0, 100.0)).uniform_fields();
        assert_eq!(fields.near_high_color, Vec4::new(0.1, 0.2, 0.3, 0.0));
        assert_eq!(fields.far_high_color, Vec4::new(0.4, 0.5, 0.6, 0.0));
        assert_eq!(fields.heights, Vec4::ZERO);
        assert_eq!(fields.params.z, 1.0);
    }

    #[test]
    fn height_fog_packs_the_high_colours_band_bottoms_and_inverse_spans_and_density_scale() {
        let fields = with_height(height_fog()).uniform_fields();
        assert_eq!(fields.near_high_color, Vec4::new(0.7, 0.8, 0.9, 0.0));
        assert_eq!(fields.far_high_color, Vec4::new(1.0, 1.1, 1.2, 0.0));
        assert_eq!(fields.heights, Vec4::new(10.0, 0.05, 25.0, 0.02));
        assert_eq!(fields.params.z, 0.5);
    }

    #[test]
    fn a_half_weighted_band_goes_half_way_to_its_high_colour_and_density_scale() {
        let half = |band: HeightBand| HeightBand {
            weight: 0.5,
            ..band
        };
        let height = height_fog();
        let fields = with_height(HeightFog {
            near_band: half(height.near_band),
            far_band: half(height.far_band),
            ..height
        })
        .uniform_fields();
        assert!(
            fields
                .near_high_color
                .truncate()
                .abs_diff_eq(Vec3::new(0.4, 0.5, 0.6), 1e-6)
        );
        assert!(
            fields
                .far_high_color
                .truncate()
                .abs_diff_eq(Vec3::new(0.7, 0.8, 0.9), 1e-6)
        );
        assert_eq!(fields.params.z, 0.75);
    }

    #[test]
    fn an_unset_band_ignores_its_high_colour_and_the_density_scale() {
        let fields = with_height(HeightFog {
            near_band: HeightBand::UNSET,
            far_band: HeightBand::UNSET,
            ..height_fog()
        })
        .uniform_fields();
        assert_eq!(fields, Fog::Enabled(params(10.0, 100.0)).uniform_fields());
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
        assert_eq!(
            HeightBand::new(5.0, 2.0),
            HeightBand {
                mid: 5.0,
                range: 2.0,
                weight: 1.0
            }
        );
    }

    #[test]
    fn a_blend_towards_an_unset_band_fades_the_bands_weight() {
        let band = HeightBand::new(40.0, 20.0);
        let halfway = band.lerp(&HeightBand::UNSET, 0.5);
        assert_eq!(halfway.weight, 0.5);
        assert_eq!(halfway.bottom_and_inverse_span(), (10.0, 0.05));
    }

    #[test]
    fn scaling_multiplies_every_colour_and_keeps_the_rest() {
        let Fog::Enabled(scaled) = with_height(height_fog()).scaled(2.0) else {
            panic!("scaled fog stays enabled");
        };
        assert_eq!(scaled.near_color, Vec3::new(0.2, 0.4, 0.6));
        assert_eq!(scaled.far_color, Vec3::new(0.8, 1.0, 1.2));
        assert_eq!(scaled.height.near_high_color, Vec3::new(1.4, 1.6, 1.8));
        assert_eq!(scaled.height.far_high_color, Vec3::new(2.0, 2.2, 2.4));
        assert_eq!(scaled.near, 10.0);
        assert_eq!(scaled.height.far_band, height_fog().far_band);
        assert_eq!(Fog::Disabled.scaled(2.0), Fog::Disabled);
    }

    #[test]
    fn disabled_fog_has_zero_opacity() {
        let fields = Fog::Disabled.uniform_fields();
        assert_eq!(fields.params, Vec4::ZERO);
    }

    #[test]
    fn a_fog_range_that_does_not_increase_is_treated_as_disabled() {
        for (near, far) in [(100.0, 100.0), (100.0, 10.0), (0.0, 0.0)] {
            let fields = Fog::Enabled(params(near, far)).uniform_fields();
            assert_eq!(fields, Fog::Disabled.uniform_fields(), "{near}..{far}");
        }
    }

    #[test]
    fn brightening_adds_the_light_to_every_colour_and_nothing_else() {
        let light = Vec3::new(0.5, 0.25, 0.125);
        let Fog::Enabled(original) = with_height(height_fog()) else {
            unreachable!("with_height builds enabled fog");
        };
        let Fog::Enabled(brightened) = Fog::Enabled(original).brightened(light) else {
            panic!("brightened fog stays enabled");
        };
        assert_eq!(brightened.near_color, original.near_color + light);
        assert_eq!(brightened.far_color, original.far_color + light);
        assert_eq!(
            brightened.height.near_high_color,
            original.height.near_high_color + light
        );
        assert_eq!(
            brightened.height.far_high_color,
            original.height.far_high_color + light
        );
        assert_eq!(
            (
                brightened.near,
                brightened.far,
                brightened.power,
                brightened.max
            ),
            (original.near, original.far, original.power, original.max)
        );
        assert_eq!(
            HeightFog {
                near_high_color: original.height.near_high_color,
                far_high_color: original.height.far_high_color,
                ..brightened.height
            },
            original.height
        );
    }

    #[test]
    fn brightening_by_no_light_changes_nothing_and_disabled_fog_stays_disabled() {
        let fog = Fog::Enabled(params(10.0, 100.0));
        assert_eq!(fog.brightened(Vec3::ZERO), fog);
        assert_eq!(Fog::Disabled.brightened(Vec3::ONE), Fog::Disabled);
    }

    #[test]
    fn a_nan_fog_range_is_treated_as_disabled() {
        let fields = Fog::Enabled(params(f32::NAN, 100.0)).uniform_fields();
        assert_eq!(fields.params, Vec4::ZERO);
    }

    fn denser() -> FogParams {
        FogParams {
            near_color: Vec3::new(0.3, 0.4, 0.5),
            far_color: Vec3::new(0.8, 0.9, 1.0),
            near: 30.0,
            far: 300.0,
            power: 4.0,
            max: 1.0,
            height: HeightFog::NONE,
        }
    }

    #[test]
    fn two_enabled_fogs_blend_every_parameter() {
        let halfway = Fog::Enabled(params(10.0, 100.0)).lerp(&Fog::Enabled(denser()), 0.5);
        assert_eq!(
            halfway,
            Fog::Enabled(FogParams {
                near_color: Vec3::new(0.2, 0.3, 0.4),
                far_color: Vec3::new(0.6, 0.7, 0.8),
                near: 20.0,
                far: 200.0,
                power: 3.0,
                max: 0.75,
                height: HeightFog::NONE,
            })
        );
    }

    #[test]
    fn a_blend_of_two_enabled_fogs_starts_at_the_first_and_ends_at_the_second() {
        let (from, to) = (Fog::Enabled(params(10.0, 100.0)), Fog::Enabled(denser()));
        assert_eq!(from.lerp(&to, 0.0), from);
        assert_eq!(from.lerp(&to, 1.0), to);
    }

    #[test]
    fn a_fog_that_is_on_at_one_end_only_snaps_at_the_midpoint() {
        let on = Fog::Enabled(params(10.0, 100.0));
        assert_eq!(on.lerp(&Fog::Disabled, 0.4), on);
        assert_eq!(on.lerp(&Fog::Disabled, 0.6), Fog::Disabled);
        assert_eq!(Fog::Disabled.lerp(&on, 0.4), Fog::Disabled);
        assert_eq!(Fog::Disabled.lerp(&on, 0.6), on);
    }

    #[test]
    fn a_fog_that_is_on_at_one_end_only_stays_with_the_first_at_exactly_one_half() {
        let on = Fog::Enabled(params(10.0, 100.0));
        assert_eq!(on.lerp(&Fog::Disabled, 0.5), on);
        assert_eq!(Fog::Disabled.lerp(&on, 0.5), Fog::Disabled);
    }

    #[test]
    fn two_disabled_fogs_stay_disabled() {
        assert_eq!(Fog::Disabled.lerp(&Fog::Disabled, 0.7), Fog::Disabled);
    }
}
