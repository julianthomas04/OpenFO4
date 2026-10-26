use glam::Vec3;

use super::{ClimateTimes, DayTime, SkyGameConfig, SkyGameSettings};

/// The Commonwealth's `DefaultClimate`: sunrise 05:00-09:00, sunset 17:00-21:00.
pub(super) fn default_climate() -> ClimateTimes {
    ClimateTimes::new(5.0, 9.0, 17.0, 21.0).unwrap()
}

/// Goodneighbor's climate: sunrise 05:30-08:00, sunset 16:00-20:30, an 11.5 hour day.
pub(super) fn goodneighbor_climate() -> ClimateTimes {
    ClimateTimes::new(5.5, 8.0, 16.0, 20.5).unwrap()
}

/// The vanilla game settings the sky reads.
pub(super) fn vanilla_settings() -> SkyGameSettings {
    SkyGameSettings::new(
        600.0,
        -325.0,
        1.0027,
        Vec3::new(-0.09, 0.35, 1.0),
        5.0,
        10.0,
    )
    .unwrap()
}

/// The vanilla settings with the vanilla sun alpha transition time and moon sizes.
pub(crate) fn vanilla_game_config() -> SkyGameConfig {
    SkyGameConfig::new(vanilla_settings(), 0.15, 90.0, 75.0).unwrap()
}

pub(super) fn at(days_passed: u32, hour: f32) -> DayTime {
    DayTime::new(days_passed, hour).unwrap()
}

pub(super) fn at_hour(hour: f32) -> DayTime {
    at(0, hour)
}
