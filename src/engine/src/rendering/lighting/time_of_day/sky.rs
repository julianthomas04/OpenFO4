use glam::Vec3;

use super::wrap::wrap_degrees;
use crate::bethesda_space::axes_to_engine;

#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum SkySettingsError {
    #[error("{field} is {value}, which is not a finite number")]
    NotFinite { field: &'static str, value: f32 },
    #[error("fStarsRotateDays is {0}, it must be a positive number of days")]
    InvalidStarRotationDays(f32),
    #[error("the star rotation axis {0:?} has no direction")]
    DegenerateStarAxis(Vec3),
    #[error("the moon fade angles run backwards (start {start}, end {end})")]
    InvertedMoonFade { start: f32, end: f32 },
}

/// The game settings the sky maths reads, validated on construction so the
/// maths built on them cannot fail. Bethesda units and axes on the way in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkyGameSettings {
    sun_x_extreme: f32,
    sun_y_extreme: f32,
    stars_rotate_days: f32,
    stars_axis_engine: Vec3,
    masser_angle_fade_start: f32,
    masser_angle_fade_end: f32,
}

impl SkyGameSettings {
    /// `stars_axis` is `(fStarsRotateXAxis, fStarsRotateYAxis, fStarsRotateZAxis)`.
    pub fn new(
        sun_x_extreme: f32,
        sun_y_extreme: f32,
        stars_rotate_days: f32,
        stars_axis: Vec3,
        masser_angle_fade_start: f32,
        masser_angle_fade_end: f32,
    ) -> Result<Self, SkySettingsError> {
        let scalars = [
            ("fSunXExtreme", sun_x_extreme),
            ("fSunYExtreme", sun_y_extreme),
            ("fStarsRotateDays", stars_rotate_days),
            ("fStarsRotateXAxis", stars_axis.x),
            ("fStarsRotateYAxis", stars_axis.y),
            ("fStarsRotateZAxis", stars_axis.z),
            ("fMasserAngleFadeStart", masser_angle_fade_start),
            ("fMasserAngleFadeEnd", masser_angle_fade_end),
        ];
        for (field, value) in scalars {
            if !value.is_finite() {
                return Err(SkySettingsError::NotFinite { field, value });
            }
        }
        if stars_rotate_days <= 0.0 {
            return Err(SkySettingsError::InvalidStarRotationDays(stars_rotate_days));
        }
        let stars_axis_engine = axes_to_engine(stars_axis.x, stars_axis.y, stars_axis.z)
            .try_normalize()
            .ok_or(SkySettingsError::DegenerateStarAxis(stars_axis))?;
        if masser_angle_fade_start > masser_angle_fade_end {
            return Err(SkySettingsError::InvertedMoonFade {
                start: masser_angle_fade_start,
                end: masser_angle_fade_end,
            });
        }
        Ok(Self {
            sun_x_extreme,
            sun_y_extreme,
            stars_rotate_days,
            stars_axis_engine,
            masser_angle_fade_start,
            masser_angle_fade_end,
        })
    }

    pub fn sun_x_extreme(&self) -> f32 {
        self.sun_x_extreme
    }

    pub fn sun_y_extreme(&self) -> f32 {
        self.sun_y_extreme
    }

    pub fn stars_rotate_days(&self) -> f32 {
        self.stars_rotate_days
    }

    /// The star rotation axis as a unit vector in engine space.
    pub fn stars_axis_engine(&self) -> Vec3 {
        self.stars_axis_engine
    }

    /// Elevations in degrees at which the moon starts and finishes fading in.
    pub fn masser_angle_fade(&self) -> (f32, f32) {
        (self.masser_angle_fade_start, self.masser_angle_fade_end)
    }
}

/// A body's place in the sky in the convention of `SunSettings`: azimuth 0 is north
/// (engine -Z) and grows clockwise towards east (+X), elevation is above the horizon.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkyPosition {
    pub azimuth_deg: f32,
    pub elevation_deg: f32,
}

impl SkyPosition {
    /// From a direction in Bethesda axes (X east, Y north, Z up); need not be normalised.
    pub(super) fn from_bethesda_direction(direction: Vec3) -> Self {
        let horizontal = direction.x.hypot(direction.y);
        Self {
            azimuth_deg: wrap_degrees(direction.x.atan2(direction.y).to_degrees()),
            elevation_deg: direction.z.atan2(horizontal).to_degrees(),
        }
    }

    /// The unit vector from the viewer towards the body, in engine axes.
    pub fn direction(&self) -> Vec3 {
        let azimuth = self.azimuth_deg.to_radians();
        let elevation = self.elevation_deg.to_radians();
        axes_to_engine(
            azimuth.sin() * elevation.cos(),
            azimuth.cos() * elevation.cos(),
            elevation.sin(),
        )
    }
}

/// 0 at or below `start`, 1 at or above `end`, linear in between. A range that
/// does not increase degrades to a step at `start`.
pub(super) fn linear_fade(value: f32, start: f32, end: f32) -> f32 {
    if end > start {
        ((value - start) / (end - start)).clamp(0.0, 1.0)
    } else {
        f32::from(value >= start)
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals parsed or copied, not computed"
)]
mod tests {
    use super::*;
    use crate::rendering::lighting::state::SunSettings;

    fn settings(
        days: f32,
        axis: Vec3,
        fade: (f32, f32),
    ) -> Result<SkyGameSettings, SkySettingsError> {
        SkyGameSettings::new(600.0, -325.0, days, axis, fade.0, fade.1)
    }

    #[test]
    fn valid_settings_expose_their_values_and_a_unit_engine_axis() {
        let settings = settings(1.0027, Vec3::new(-0.09, 0.35, 1.0), (5.0, 10.0)).unwrap();
        assert_eq!(settings.sun_x_extreme(), 600.0);
        assert_eq!(settings.sun_y_extreme(), -325.0);
        assert_eq!(settings.stars_rotate_days(), 1.0027);
        assert_eq!(settings.masser_angle_fade(), (5.0, 10.0));
        let expected = axes_to_engine(-0.09, 0.35, 1.0).normalize();
        assert!(settings.stars_axis_engine().abs_diff_eq(expected, 1e-6));
    }

    #[test]
    fn a_fade_range_that_is_a_single_angle_is_allowed() {
        assert!(settings(1.0, Vec3::Z, (7.0, 7.0)).is_ok());
    }

    #[test]
    fn non_finite_values_name_the_setting() {
        let result = SkyGameSettings::new(f32::NAN, 1.0, 1.0, Vec3::Z, 5.0, 10.0);
        assert!(matches!(
            result,
            Err(SkySettingsError::NotFinite {
                field: "fSunXExtreme",
                ..
            })
        ));
        let result =
            SkyGameSettings::new(1.0, 1.0, 1.0, Vec3::new(0.0, f32::INFINITY, 1.0), 5.0, 10.0);
        assert!(matches!(
            result,
            Err(SkySettingsError::NotFinite {
                field: "fStarsRotateYAxis",
                ..
            })
        ));
        let result = SkyGameSettings::new(1.0, 1.0, 1.0, Vec3::Z, 5.0, f32::NEG_INFINITY);
        assert!(matches!(
            result,
            Err(SkySettingsError::NotFinite {
                field: "fMasserAngleFadeEnd",
                ..
            })
        ));
    }

    #[test]
    fn a_star_rotation_period_that_is_not_positive_is_rejected() {
        for days in [0.0, -1.0] {
            assert!(
                matches!(
                    settings(days, Vec3::Z, (5.0, 10.0)),
                    Err(SkySettingsError::InvalidStarRotationDays(_))
                ),
                "{days}"
            );
        }
    }

    #[test]
    fn a_zero_star_axis_is_rejected() {
        assert!(matches!(
            settings(1.0, Vec3::ZERO, (5.0, 10.0)),
            Err(SkySettingsError::DegenerateStarAxis(_))
        ));
    }

    #[test]
    fn backwards_moon_fade_angles_are_rejected() {
        assert!(matches!(
            settings(1.0, Vec3::Z, (10.0, 5.0)),
            Err(SkySettingsError::InvertedMoonFade {
                start: 10.0,
                end: 5.0
            })
        ));
    }

    #[test]
    fn the_direction_agrees_with_the_sun_settings_convention() {
        for azimuth_deg in [0.0, 37.0, 90.0, 180.0, 220.0, 271.5, 359.0] {
            for elevation_deg in [-60.0, -5.0, 0.0, 12.5, 45.0, 89.0] {
                let sun = SunSettings {
                    azimuth_deg,
                    elevation_deg,
                    ..SunSettings::default()
                };
                let position = SkyPosition {
                    azimuth_deg,
                    elevation_deg,
                };
                assert!(
                    position
                        .direction()
                        .abs_diff_eq(sun.direction_to_sun(), 1e-6),
                    "az {azimuth_deg} el {elevation_deg}"
                );
            }
        }
    }

    #[test]
    fn compass_directions_land_on_the_expected_engine_axes() {
        let direction = |azimuth_deg| {
            SkyPosition {
                azimuth_deg,
                elevation_deg: 0.0,
            }
            .direction()
        };
        assert!(direction(0.0).abs_diff_eq(Vec3::NEG_Z, 1e-6));
        assert!(direction(90.0).abs_diff_eq(Vec3::X, 1e-6));
        assert!(direction(180.0).abs_diff_eq(Vec3::Z, 1e-6));
        assert!(direction(270.0).abs_diff_eq(Vec3::NEG_X, 1e-6));
    }

    #[test]
    fn a_bethesda_direction_round_trips_through_azimuth_and_elevation() {
        let bethesda = Vec3::new(3.0, -4.0, 5.0);
        let position = SkyPosition::from_bethesda_direction(bethesda);
        let expected = axes_to_engine(bethesda.x, bethesda.y, bethesda.z).normalize();
        assert!(position.direction().abs_diff_eq(expected, 1e-6));
        assert!((0.0..360.0).contains(&position.azimuth_deg));
    }

    #[test]
    fn straight_up_has_ninety_degrees_elevation() {
        let position = SkyPosition::from_bethesda_direction(Vec3::Z);
        assert!((position.elevation_deg - 90.0).abs() < 1e-4);
    }

    #[test]
    fn linear_fade_is_clamped_and_linear_between_its_bounds() {
        assert_eq!(linear_fade(-10.0, 5.0, 10.0), 0.0);
        assert_eq!(linear_fade(5.0, 5.0, 10.0), 0.0);
        assert_eq!(linear_fade(7.5, 5.0, 10.0), 0.5);
        assert_eq!(linear_fade(10.0, 5.0, 10.0), 1.0);
        assert_eq!(linear_fade(90.0, 5.0, 10.0), 1.0);
    }

    #[test]
    fn a_fade_range_that_does_not_increase_steps_at_its_start() {
        for (start, end) in [(5.0, 5.0), (10.0, 5.0)] {
            assert_eq!(linear_fade(4.999, start, end), 0.0);
            assert_eq!(linear_fade(start, start, end), 1.0);
        }
    }
}
