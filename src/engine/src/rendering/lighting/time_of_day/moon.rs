use super::sky::{SkyGameSettings, linear_fade};
use super::unmeasured::MOON_PHASE_AT_DAY_ZERO;

const PHASE_COUNT: u64 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MoonError {
    #[error("the moon's phase length is 0 days")]
    ZeroPhaseLength,
}

/// The eight moon phases in the order a lunar cycle visits them, starting full.
///
/// The names match the `*_{full,three_wan,half_wan,one_wan,new,one_wax,half_wax,three_wax}`
/// textures: the lit fraction shrinks 4/4, 3/4, 2/4, 1/4 while waning, is 0 at new,
/// then grows 1/4, 2/4, 3/4 while waxing back to full.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MoonPhase {
    Full,
    ThreeWan,
    HalfWan,
    OneWan,
    New,
    OneWax,
    HalfWax,
    ThreeWax,
}

impl MoonPhase {
    pub const ALL: [Self; 8] = [
        Self::Full,
        Self::ThreeWan,
        Self::HalfWan,
        Self::OneWan,
        Self::New,
        Self::OneWax,
        Self::HalfWax,
        Self::ThreeWax,
    ];

    /// The position in [`Self::ALL`].
    pub const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoonKind {
    Masser,
    Secunda,
}

/// The cycle length in days: eight phases of `phase_length` days.
pub(super) fn cycle_days(phase_length: u32) -> Result<u64, MoonError> {
    if phase_length == 0 {
        return Err(MoonError::ZeroPhaseLength);
    }
    Ok(u64::from(phase_length) * PHASE_COUNT)
}

/// The phase after `days_passed` completed days when each phase lasts `phase_length` days.
/// The phase on day 0 is [`MOON_PHASE_AT_DAY_ZERO`].
pub fn moon_phase(days_passed: u32, phase_length: u32) -> Result<MoonPhase, MoonError> {
    cycle_days(phase_length)?;
    let phases_elapsed = u64::from(days_passed) / u64::from(phase_length);
    let index = (MOON_PHASE_AT_DAY_ZERO.index() as u64 + phases_elapsed) % PHASE_COUNT;
    Ok(MoonPhase::ALL[index as usize])
}

/// The texture file stem under `textures/sky` (extension `.DDS`) of a moon in a phase.
pub const fn texture_stem(kind: MoonKind, phase: MoonPhase) -> &'static str {
    match (kind, phase) {
        (MoonKind::Masser, MoonPhase::Full) => "Masser_full",
        (MoonKind::Masser, MoonPhase::ThreeWan) => "Masser_three_wan",
        (MoonKind::Masser, MoonPhase::HalfWan) => "Masser_half_wan",
        (MoonKind::Masser, MoonPhase::OneWan) => "Masser_one_wan",
        (MoonKind::Masser, MoonPhase::New) => "Masser_new",
        (MoonKind::Masser, MoonPhase::OneWax) => "Masser_one_wax",
        (MoonKind::Masser, MoonPhase::HalfWax) => "Masser_half_wax",
        (MoonKind::Masser, MoonPhase::ThreeWax) => "Masser_three_wax",
        (MoonKind::Secunda, MoonPhase::Full) => "Secunda_full",
        (MoonKind::Secunda, MoonPhase::ThreeWan) => "Secunda_three_wan",
        (MoonKind::Secunda, MoonPhase::HalfWan) => "Secunda_half_wan",
        (MoonKind::Secunda, MoonPhase::OneWan) => "Secunda_one_wan",
        (MoonKind::Secunda, MoonPhase::New) => "Secunda_new",
        (MoonKind::Secunda, MoonPhase::OneWax) => "Secunda_one_wax",
        (MoonKind::Secunda, MoonPhase::HalfWax) => "Secunda_half_wax",
        (MoonKind::Secunda, MoonPhase::ThreeWax) => "Secunda_three_wax",
    }
}

/// 0 while the moon is at or below the fade start elevation, 1 at or above the fade end.
pub fn moon_visibility(elevation_deg: f32, settings: &SkyGameSettings) -> f32 {
    let (start, end) = settings.masser_angle_fade();
    linear_fade(elevation_deg, start, end)
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals parsed or copied, not computed"
)]
mod tests {
    use super::super::test_support::vanilla_settings;
    use super::*;

    #[test]
    fn phases_advance_every_phase_length_days_in_texture_order() {
        use MoonPhase::*;
        let expected = [
            Full, ThreeWan, HalfWan, OneWan, New, OneWax, HalfWax, ThreeWax,
        ];
        for (index, phase) in expected.into_iter().enumerate() {
            let first_day = index as u32 * 4;
            for day in first_day..first_day + 4 {
                assert_eq!(moon_phase(day, 4).unwrap(), phase, "day {day}");
            }
        }
    }

    #[test]
    fn the_cycle_wraps_after_eight_phase_lengths() {
        for phase_length in [1, 3, 4, 63] {
            let cycle = 8 * phase_length;
            assert_eq!(
                moon_phase(cycle - 1, phase_length).unwrap(),
                MoonPhase::ThreeWax
            );
            assert_eq!(moon_phase(cycle, phase_length).unwrap(), MoonPhase::Full);
            assert_eq!(
                moon_phase(cycle + phase_length, phase_length).unwrap(),
                MoonPhase::ThreeWan
            );
        }
    }

    #[test]
    fn phase_indices_follow_the_cycle_order() {
        for (index, phase) in MoonPhase::ALL.into_iter().enumerate() {
            assert_eq!(phase.index(), index);
        }
    }

    #[test]
    fn a_phase_length_of_one_changes_the_phase_every_day() {
        let phases: Vec<MoonPhase> = (0..8).map(|day| moon_phase(day, 1).unwrap()).collect();
        assert_eq!(phases, MoonPhase::ALL);
    }

    #[test]
    fn the_largest_day_count_does_not_overflow() {
        assert!(moon_phase(u32::MAX, 1).is_ok());
        assert!(moon_phase(u32::MAX, u32::MAX).is_ok());
        assert_eq!(cycle_days(u32::MAX).unwrap(), u64::from(u32::MAX) * 8);
    }

    #[test]
    fn a_zero_phase_length_is_an_error() {
        assert_eq!(moon_phase(10, 0), Err(MoonError::ZeroPhaseLength));
        assert_eq!(cycle_days(0), Err(MoonError::ZeroPhaseLength));
    }

    #[test]
    fn texture_stems_match_the_installed_file_names() {
        let masser: Vec<&str> = MoonPhase::ALL
            .iter()
            .map(|phase| texture_stem(MoonKind::Masser, *phase))
            .collect();
        assert_eq!(
            masser,
            [
                "Masser_full",
                "Masser_three_wan",
                "Masser_half_wan",
                "Masser_one_wan",
                "Masser_new",
                "Masser_one_wax",
                "Masser_half_wax",
                "Masser_three_wax"
            ]
        );
        for phase in MoonPhase::ALL {
            let secunda = texture_stem(MoonKind::Secunda, phase);
            let masser = texture_stem(MoonKind::Masser, phase);
            assert_eq!(secunda.replace("Secunda", "Masser"), masser);
        }
    }

    #[test]
    fn visibility_is_zero_below_the_fade_start_and_one_above_the_fade_end() {
        let settings = vanilla_settings();
        for elevation in [-90.0, -0.1, 0.0, 4.99, 5.0] {
            assert_eq!(moon_visibility(elevation, &settings), 0.0, "{elevation}");
        }
        for elevation in [10.0, 10.01, 45.0, 90.0] {
            assert_eq!(moon_visibility(elevation, &settings), 1.0, "{elevation}");
        }
    }

    #[test]
    fn visibility_ramps_linearly_between_the_fade_angles() {
        let settings = vanilla_settings();
        assert!((moon_visibility(6.0, &settings) - 0.2).abs() < 1e-6);
        assert_eq!(moon_visibility(7.5, &settings), 0.5);
        assert!((moon_visibility(9.0, &settings) - 0.8).abs() < 1e-6);
    }

    #[test]
    fn visibility_never_leaves_zero_to_one_and_never_decreases_with_elevation() {
        let settings = vanilla_settings();
        let mut previous = 0.0;
        for step in -900..=900 {
            let visibility = moon_visibility(step as f32 / 10.0, &settings);
            assert!((0.0..=1.0).contains(&visibility));
            assert!(visibility >= previous);
            previous = visibility;
        }
    }
}
