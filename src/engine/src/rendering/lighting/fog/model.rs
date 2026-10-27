//! The fog of one point, as `fog.wgsl` computes it. Below Max the colour weight `x = ramp^power`
//! picks between the near and far colours and, scaled by the height density, is the opacity.
//! From Max on the fog holds at Max and ramps to full fog towards Far; beyond Far it is full fog
//! of the far colour. One height share, lerped from the near band's to the far band's by `x`,
//! turns both colours towards their high colours and scales the density.

use glam::Vec3;

use super::measured::cap_ramp_start;
use super::{Fog, FogParams};

/// The shortest span a ramp divides by, as in `fog.wgsl`.
const FOG_MIN_RANGE: f32 = 0.001;

/// 0 up to `start`, 1 from `end` on, linear between.
fn ramp(distance: f32, start: f32, end: f32) -> f32 {
    ((distance - start) / (end - start).max(FOG_MIN_RANGE)).clamp(0.0, 1.0)
}

/// A high density scale above 1 makes the fog denser than full.
fn opacity(weight: f32, density: f32) -> f32 {
    (weight * density).min(1.0)
}

/// What the fog does to a surface behind it: `mix(surface, color, opacity)`.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct FogBlend {
    pub color: Vec3,
    pub opacity: f32,
}

impl FogBlend {
    pub const NONE: Self = Self {
        color: Vec3::ZERO,
        opacity: 0.0,
    };

    pub fn apply(self, surface: Vec3) -> Vec3 {
        surface.lerp(self.color, self.opacity)
    }

    /// This blend followed by a blend of `t` towards full fog of `fill`.
    fn towards(self, fill: Vec3, t: f32) -> Self {
        let opacity = self.opacity + (1.0 - self.opacity) * t;
        if opacity <= 0.0 {
            return self;
        }
        Self {
            color: (self.color * (self.opacity * (1.0 - t)) + fill * t) / opacity,
            opacity,
        }
    }
}

impl Fog {
    /// The fog at `point` seen from `camera`, both in engine space.
    pub fn at(&self, camera: Vec3, point: Vec3) -> FogBlend {
        self.rendered()
            .map_or(FogBlend::NONE, |fog| fog.at(camera, point))
    }
}

impl FogParams {
    /// `ramp^power`: 0 up to Near, 1 at Far.
    pub fn colour_weight(&self, distance: f32) -> f32 {
        let ramp = ramp(distance, self.near, self.far);
        if ramp > 0.0 {
            ramp.powf(self.power)
        } else {
            0.0
        }
    }

    /// The near and far colours at height share `share`.
    fn colors_at(&self, share: f32) -> (Vec3, Vec3) {
        let colors = self.colors;
        (
            colors.near.lerp(colors.near_high, share),
            colors.far.lerp(colors.far_high, share),
        )
    }

    fn at(&self, camera: Vec3, point: Vec3) -> FogBlend {
        let distance = camera.distance(point);
        let height = point.y;
        if distance >= self.far {
            let (_, fill) = self.colors_at(self.height.share(height, 1.0));
            return FogBlend {
                color: fill,
                opacity: 1.0,
            };
        }
        let x = self.colour_weight(distance);
        let share = self.height.share(height, x);
        let (near, far) = self.colors_at(share);
        let density = self.height.density(share);
        if x < self.max {
            return FogBlend {
                color: self.sunlight.apply(near.lerp(far, x)),
                opacity: opacity(x, density),
            };
        }
        let cap = FogBlend {
            color: near.lerp(far, self.max),
            opacity: opacity(self.max, density),
        };
        cap.towards(far, ramp(distance, cap_ramp_start(self.far), self.far))
    }
}

/// The fog probe's rigs and what it measured in them, in Bethesda units: shared by the unit tests
/// of the model and the GPU tests of `fog.wgsl`.
#[cfg(test)]
pub(crate) mod probe {
    use glam::Vec3;

    use super::super::measured::{colour_factors, sunlight_term};
    use super::super::{FogColors, FogParams, HeightBands, SunlightTerm};
    use crate::rendering::world_streamer::SCALE;

    /// `CommonwealthClear`'s day sunlight, (225, 225, 225) decoded with gamma 2.2.
    const NOON_SUNLIGHT: f32 = 0.759_3;

    /// Fog by the probe's FNAM values at noon in sunlight or at midnight, with `colors` taken as
    /// the calibration colours.
    pub(crate) fn fog(
        colors: FogColors,
        (near, far, power, max): (f32, f32, f32, f32),
        noon: bool,
    ) -> FogParams {
        let day_colour_weight = if noon { 1.0 } else { 0.0 };
        FogParams {
            colors: colour_factors(day_colour_weight).apply(colors),
            near: near * SCALE,
            far: far * SCALE,
            power,
            max,
            height: HeightBands::NONE,
            sunlight: if noon {
                sunlight_term(Vec3::splat(NOON_SUNLIGHT), day_colour_weight)
            } else {
                SunlightTerm::NONE
            },
        }
    }

    pub(crate) const DIST_BASE: (f32, f32, f32, f32) = (500.0, 6000.0, 1.0, 1.0);
    pub(crate) const DIST_P050: (f32, f32, f32, f32) = (500.0, 6000.0, 0.5, 1.0);
    pub(crate) const DIST_F3000_MAX050: (f32, f32, f32, f32) = (500.0, 3000.0, 1.0, 0.5);

    /// Distance and fog factor `f` (fog over a black surface, relative to the fill colour) at
    /// noon in sunlight.
    pub(crate) const DIST_BASE_FACTORS: [(f32, f32); 7] = [
        (929.0, 0.036),
        (1778.0, 0.125),
        (2500.0, 0.215),
        (3401.0, 0.366),
        (4705.0, 0.611),
        (6507.0, 1.000),
        (9000.0, 1.000),
    ];
    pub(crate) const DIST_P050_FACTORS: [(f32, f32); 5] = [
        (672.0, 0.093),
        (1778.0, 0.323),
        (3401.0, 0.577),
        (4705.0, 0.751),
        (9000.0, 1.000),
    ];
    /// On the strip: the normal branch, the plateau at Max, the ramp to full fog and the fill.
    pub(crate) const DIST_F3000_MAX050_FACTORS: [(f32, f32); 7] = [
        (1650.0, 0.311),
        (1950.0, 0.368),
        (2250.0, 0.368),
        (2450.0, 0.424),
        (2550.0, 0.514),
        (2650.0, 0.612),
        (3050.0, 1.000),
    ];
    /// Distance and the near and far colours' weights in `dist_base` at noon in sunlight.
    pub(crate) const DIST_BASE_COLOUR_WEIGHTS: [(f32, f32, f32); 3] = [
        (1778.0, 0.073, 0.044),
        (3401.0, 0.111, 0.255),
        (4705.0, 0.079, 0.532),
    ];
    /// The ranges of the near and far weights measured on the plateau of `mix_far_max050`.
    pub(crate) const MAX050_PLATEAU_COLOUR_WEIGHTS: ((f32, f32), (f32, f32)) =
        ((0.113, 0.118), (0.248, 0.251));
    /// The fill at midnight against the noon fill.
    pub(crate) const NIGHT_FILL: f32 = 0.482;

    /// The calibration colour in every slot.
    pub(crate) fn grey() -> FogColors {
        FogColors::uniform(Vec3::ONE)
    }

    /// The near colours red and the far ones green, to tell their weights apart.
    pub(crate) fn red_near_green_far() -> FogColors {
        FogColors {
            near: Vec3::X,
            far: Vec3::Y,
            near_high: Vec3::X,
            far_high: Vec3::Y,
        }
    }

    /// A point `distance` Bethesda units ahead of a camera at the origin, in engine units.
    pub(crate) fn ahead(distance: f32) -> glam::Vec3 {
        Vec3::new(0.0, 0.0, -distance * SCALE)
    }
}

#[cfg(test)]
mod tests {
    use super::super::measured::{DAY_COLOUR_FACTORS, cap_ramp_start};
    use super::super::{FogColors, HeightBand, HeightBands, SunlightTerm};
    use super::probe::*;
    use super::*;

    fn factor(fog: &FogParams, distance: f32) -> f32 {
        Fog::Enabled(*fog)
            .at(Vec3::ZERO, ahead(distance))
            .apply(Vec3::ZERO)
            .x
    }

    fn assert_factors(fog: &FogParams, measured: &[(f32, f32)], tolerance: f32) {
        for &(distance, expected) in measured {
            let actual = factor(fog, distance);
            assert!(
                (actual - expected).abs() < tolerance,
                "d {distance}: {actual} against the measured {expected}"
            );
        }
    }

    #[test]
    fn the_factor_follows_the_probes_distance_ramp() {
        assert_factors(&fog(grey(), DIST_BASE, true), &DIST_BASE_FACTORS, 0.015);
    }

    #[test]
    fn the_power_bends_the_ramp_as_measured() {
        assert_factors(&fog(grey(), DIST_P050, true), &DIST_P050_FACTORS, 0.015);
    }

    #[test]
    fn max_holds_a_plateau_and_ramps_to_full_fog_before_far() {
        assert_factors(
            &fog(grey(), DIST_F3000_MAX050, true),
            &DIST_F3000_MAX050_FACTORS,
            0.025,
        );
    }

    #[test]
    fn the_colour_weights_of_near_and_far_are_the_measured_ones() {
        let fog = Fog::Enabled(fog(red_near_green_far(), DIST_BASE, true));
        for (distance, near, far) in DIST_BASE_COLOUR_WEIGHTS {
            let seen = fog.at(Vec3::ZERO, ahead(distance)).apply(Vec3::ZERO);
            assert!(
                (seen.x - near).abs() < 0.01,
                "near at {distance}: {}",
                seen.x
            );
            assert!((seen.y - far).abs() < 0.01, "far at {distance}: {}", seen.y);
        }
    }

    #[test]
    fn the_plateau_mixes_near_and_far_by_max_without_the_sunlight() {
        let fog = Fog::Enabled(fog(red_near_green_far(), (500.0, 6000.0, 1.0, 0.5), true));
        let seen = fog.at(Vec3::ZERO, ahead(4000.0)).apply(Vec3::ZERO);
        let ((near_low, near_high), (far_low, far_high)) = MAX050_PLATEAU_COLOUR_WEIGHTS;
        assert!((near_low..=near_high).contains(&seen.x), "{seen}");
        assert!((far_low..=far_high).contains(&seen.y), "{seen}");
    }

    #[test]
    fn at_midnight_every_colour_takes_the_night_factor() {
        let fog = fog(grey(), DIST_BASE, false);
        assert!((factor(&fog, 9000.0) - NIGHT_FILL).abs() < 0.005);
        let x = fog.colour_weight(3401.0 * crate::rendering::world_streamer::SCALE);
        assert!((factor(&fog, 3401.0) - 0.48 * x).abs() < 1e-5);
    }

    fn plain_fog() -> FogParams {
        FogParams {
            colors: FogColors::uniform(Vec3::ONE),
            near: 10.0,
            far: 110.0,
            power: 2.0,
            max: 1.0,
            height: HeightBands::NONE,
            sunlight: SunlightTerm::NONE,
        }
    }

    #[test]
    fn the_distance_is_radial_from_the_camera() {
        let fog = Fog::Enabled(plain_fog());
        let camera = Vec3::new(5.0, 7.0, -3.0);
        let ahead = fog.at(camera, camera + Vec3::new(0.0, 0.0, -60.0));
        let aside = fog.at(camera, camera + Vec3::new(36.0, 0.0, -48.0));
        assert!((ahead.opacity - 0.25).abs() < 1e-6);
        assert!((aside.opacity - ahead.opacity).abs() < 1e-6);
    }

    #[test]
    fn there_is_no_fog_before_near_and_full_fog_from_far() {
        let fog = Fog::Enabled(plain_fog());
        assert_eq!(fog.at(Vec3::ZERO, Vec3::new(0.0, 0.0, -9.0)).opacity, 0.0);
        assert_eq!(fog.at(Vec3::ZERO, Vec3::new(0.0, 0.0, -110.0)).opacity, 1.0);
    }

    #[test]
    fn a_zero_power_does_not_fog_before_near() {
        let fog = Fog::Enabled(FogParams {
            power: 0.0,
            ..plain_fog()
        });
        assert_eq!(fog.at(Vec3::ZERO, Vec3::new(0.0, 0.0, -5.0)).opacity, 0.0);
    }

    #[test]
    fn disabled_fog_leaves_every_point_alone() {
        assert_eq!(
            Fog::Disabled.at(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0e6)),
            FogBlend::NONE
        );
    }

    fn height_fog() -> FogParams {
        FogParams {
            colors: FogColors {
                near: Vec3::X,
                far: Vec3::Z,
                near_high: Vec3::Y,
                far_high: Vec3::ONE,
            },
            near: 0.0,
            far: 100.0,
            power: 1.0,
            max: 1.0,
            height: HeightBands {
                near: HeightBand::new(20.0, 10.0),
                far: HeightBand::new(50.0, 25.0),
                high_density_scale: 0.5,
            },
            sunlight: SunlightTerm::NONE,
        }
    }

    #[test]
    fn one_height_share_lerped_by_the_colour_weight_turns_both_colours_and_the_density() {
        let fog = Fog::Enabled(height_fog());
        let point = Vec3::new(40.0, 30.0, 0.0);
        // d 50: x 0.5. Near band share 1, far band share 0.1: the shared share is 0.55.
        let blend = fog.at(Vec3::ZERO, point);
        let share = 0.55;
        let near = Vec3::X.lerp(Vec3::Y, share);
        let far = Vec3::Z.lerp(Vec3::ONE, share);
        assert!(
            blend.color.abs_diff_eq(near.lerp(far, 0.5), 1e-5),
            "{blend:?}"
        );
        assert!((blend.opacity - 0.5 * (1.0 - 0.5 * share)).abs() < 1e-5);
    }

    #[test]
    fn the_fill_beyond_far_takes_the_far_bands_share() {
        let fog = Fog::Enabled(height_fog());
        let blend = fog.at(Vec3::ZERO, Vec3::new(0.0, 50.0, -100.0));
        assert_eq!(blend.opacity, 1.0);
        assert!(blend.color.abs_diff_eq(Vec3::new(0.5, 0.5, 1.0), 1e-5));
    }

    #[test]
    fn the_capped_branch_scales_its_plateau_by_the_density() {
        let fog = Fog::Enabled(FogParams {
            max: 0.25,
            ..height_fog()
        });
        // Above both bands at distance 90, half way up the ramp from 80 to full fog at 100.
        let point = Vec3::new(0.0, 78.0, -(8100.0_f32 - 78.0 * 78.0).sqrt());
        let blend = fog.at(Vec3::ZERO, point);
        assert_eq!(cap_ramp_start(100.0), 80.0);
        let cap = 0.25 * 0.5;
        assert!(
            (blend.opacity - (cap + (1.0 - cap) * 0.5)).abs() < 1e-5,
            "{blend:?}"
        );
    }

    #[test]
    fn the_sunlight_term_reaches_the_normal_branch_only() {
        let sunlit = FogParams {
            colors: DAY_COLOUR_FACTORS.apply(FogColors::uniform(Vec3::ONE)),
            sunlight: SunlightTerm {
                kept: 0.5,
                added: Vec3::splat(0.1),
            },
            max: 0.5,
            ..plain_fog()
        };
        let fog = Fog::Enabled(sunlit);
        let normal = fog.at(Vec3::ZERO, Vec3::new(0.0, 0.0, -60.0));
        let x = 0.25;
        let unlit = 0.47 + 0.53 * x;
        assert!((normal.color.x - (0.5 * unlit + 0.1)).abs() < 1e-5);
        let capped = fog.at(Vec3::ZERO, Vec3::new(0.0, 0.0, -85.0));
        assert!((capped.color.x - (0.47 + 0.53 * 0.5)).abs() < 1e-5);
        let fill = fog.at(Vec3::ZERO, Vec3::new(0.0, 0.0, -200.0));
        assert_eq!(fill.color, Vec3::ONE);
    }

    #[test]
    fn a_density_above_one_never_makes_the_fog_more_than_full() {
        let dense = |max: f32| {
            Fog::Enabled(FogParams {
                max,
                height: HeightBands {
                    near: HeightBand::new(0.0, 1.0),
                    far: HeightBand::new(0.0, 1.0),
                    high_density_scale: 3.0,
                },
                ..plain_fog()
            })
        };
        let high = Vec3::new(0.0, 50.0, -80.0);
        // x 0.71 at d 94.3, all of it above the bands: normal under Max 1, capped under Max 0.5.
        assert_eq!(dense(1.0).at(Vec3::ZERO, high).opacity, 1.0);
        assert_eq!(dense(0.5).at(Vec3::ZERO, high).opacity, 1.0);
    }

    #[test]
    fn a_ramp_shorter_than_the_minimum_divides_by_the_minimum() {
        let fog = FogParams {
            near: 10.0,
            far: 10.0005,
            power: 1.0,
            ..plain_fog()
        };
        assert!((fog.colour_weight(10.0003) - 0.3).abs() < 1e-3);
        assert_eq!(ramp(5.0, 0.0, 0.0), 1.0);
        assert_eq!(ramp(-5.0, 0.0, 0.0), 0.0);
    }
}
