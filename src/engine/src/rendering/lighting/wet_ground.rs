//! How dark the landscape turns in the rain, as the probe measured it on vanilla's own scenes: the
//! same weather with and without its rain, at 12:00 and 01:00, from the bridge and the onset views.
//! The real terrain keeps 0.86 to 0.90 of its dry light in every one of them, near and far alike,
//! and by day and night alike, so the rain darkens the landscape's albedo, not a light.
//!
//! The probe boards with plain, rock and wood materials changed by their own factors (from 0.74 to
//! 1.8), so the rule is for the landscape alone.

/// The share of its dry albedo the landscape keeps in full rain (the median of the measured views).
pub const WET_LANDSCAPE_ALBEDO: f32 = 0.88;

/// The share of its dry albedo the landscape keeps while it rains `rain` hard, `0..=1`: the rain
/// darkens it in proportion as the drops fall.
pub fn landscape_albedo_scale(rain: f32) -> f32 {
    1.0 - (1.0 - WET_LANDSCAPE_ALBEDO) * rain.clamp(0.0, 1.0)
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "the ends of the ramp are exact")]
mod tests {
    use super::*;

    #[test]
    fn dry_ground_keeps_its_albedo_and_full_rain_leaves_the_measured_share() {
        assert_eq!(landscape_albedo_scale(0.0), 1.0);
        assert_eq!(landscape_albedo_scale(1.0), WET_LANDSCAPE_ALBEDO);
    }

    #[test]
    fn the_ground_darkens_in_proportion_as_the_rain_comes_in() {
        let half = landscape_albedo_scale(0.5);
        assert!(
            (half - f32::midpoint(1.0, WET_LANDSCAPE_ALBEDO)).abs() < 1e-6,
            "{half}"
        );
    }

    #[test]
    fn rain_outside_its_range_is_held_to_it() {
        assert_eq!(landscape_albedo_scale(-1.0), 1.0);
        assert_eq!(landscape_albedo_scale(3.0), WET_LANDSCAPE_ALBEDO);
    }
}
