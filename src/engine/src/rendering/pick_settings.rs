//! The values that decide what E reaches and how long a hold grabs: the original game's three
//! settings for it, `fActivatePickLength`, `fActivatePickRadius` (both `[Interface]`, game units)
//! and `fZKeyDelay` (`[Controls]`, seconds). The original has them built in and none of its INIs
//! sets them, so the built-in defaults are carried here and a game INI that sets one overrides it,
//! as in the original.

use std::time::Duration;

use crate::assets::ini::{GameIni, IniError, IniKey};
use crate::bethesda_space::UNITS_TO_ENGINE;

const PICK_LENGTH: IniKey = IniKey::new("Interface", "fActivatePickLength");
const PICK_RADIUS: IniKey = IniKey::new("Interface", "fActivatePickRadius");
const Z_KEY_DELAY: IniKey = IniKey::new("Controls", "fZKeyDelay");

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PickSettings {
    /// How far (engine units) the crosshair line reaches, from the eye to the point it meets an
    /// item's surface: `fActivatePickLength`. The ball does not extend it along the aim.
    pub reach: f32,
    /// The radius (engine units) of the ball swept along the crosshair line to find placed
    /// items: `fActivatePickRadius`. Doors, containers and other activators keep the exact ray:
    /// the original was only measured on an item.
    pub radius: f32,
    /// How long E must stay down on a loose body before it is grabbed: `fZKeyDelay`. A hold of
    /// exactly this long still takes.
    pub grab_hold: Duration,
}

const fn engine_units(game_units: f32) -> f32 {
    game_units * UNITS_TO_ENGINE
}

/// The built-in defaults of the original's settings, read back in its console and confirmed by
/// the behaviour changing when they are overridden (2026-11-13): 180 and 16 game units, 0.20 s.
pub(super) const ORIGINAL_PICK_DEFAULTS: PickSettings = PickSettings {
    reach: engine_units(180.0),
    radius: engine_units(16.0),
    grab_hold: Duration::from_millis(200),
};

impl PickSettings {
    /// A setting no INI sets takes its [`ORIGINAL_PICK_DEFAULTS`] value, with a note in the log;
    /// one that is set must be a finite number above 0, the radius 0 or more (0 is the exact
    /// ray).
    pub fn from_ini(ini: &GameIni) -> Result<Self, IniError> {
        Ok(Self {
            reach: length_or(ini, PICK_LENGTH, ORIGINAL_PICK_DEFAULTS.reach)?,
            radius: radius_or(ini, PICK_RADIUS, ORIGINAL_PICK_DEFAULTS.radius)?,
            grab_hold: delay_or(ini, Z_KEY_DELAY, ORIGINAL_PICK_DEFAULTS.grab_hold)?,
        })
    }
}

fn note_default(key: IniKey, default: impl std::fmt::Debug) {
    tracing::info!(
        "{} is not set in any game INI; the original's built-in default {default:?} applies",
        key.name
    );
}

/// The setting in game units as engine units.
fn length_or(ini: &GameIni, key: IniKey, default: f32) -> Result<f32, IniError> {
    Ok(game_units_or(ini.optional_positive(key)?, key, default))
}

/// [`length_or`] where 0 is a real value.
fn radius_or(ini: &GameIni, key: IniKey, default: f32) -> Result<f32, IniError> {
    Ok(game_units_or(ini.optional_non_negative(key)?, key, default))
}

fn game_units_or(read: Option<f32>, key: IniKey, default: f32) -> f32 {
    read.map_or_else(
        || {
            note_default(key, default);
            default
        },
        engine_units,
    )
}

/// The setting in seconds as a duration.
fn delay_or(ini: &GameIni, key: IniKey, default: Duration) -> Result<Duration, IniError> {
    let seconds = ini.optional_valid::<f32>(key, "a number of seconds above 0", |seconds| {
        seconds.is_finite() && *seconds > 0.0 && Duration::try_from_secs_f32(*seconds).is_ok()
    })?;
    Ok(seconds.map_or_else(
        || {
            note_default(key, default);
            default
        },
        Duration::from_secs_f32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Result<PickSettings, IniError> {
        PickSettings::from_ini(&GameIni::from_texts([text]))
    }

    #[test]
    fn the_defaults_are_the_originals_in_engine_units() {
        let defaults = ORIGINAL_PICK_DEFAULTS;
        assert!((defaults.reach - 18.0).abs() < 1e-5, "{}", defaults.reach);
        assert!((defaults.radius - 1.6).abs() < 1e-5, "{}", defaults.radius);
        assert_eq!(defaults.grab_hold, Duration::from_millis(200));
    }

    #[test]
    fn settings_no_ini_sets_take_the_defaults() {
        assert_eq!(
            read(
                "[Display]
iSize=1
"
            )
            .unwrap(),
            ORIGINAL_PICK_DEFAULTS
        );
    }

    #[test]
    fn each_setting_an_ini_sets_overrides_its_default_and_leaves_the_others() {
        let all = read(
            "[Interface]
fActivatePickLength=300
fActivatePickRadius=40
[Controls]
fZKeyDelay=1.0
",
        )
        .unwrap();
        assert!((all.reach - 30.0).abs() < 1e-5);
        assert!((all.radius - 4.0).abs() < 1e-5);
        assert_eq!(all.grab_hold, Duration::from_secs(1));

        let only_radius = read(
            "[Interface]
fActivatePickRadius=40
",
        )
        .unwrap();
        assert_eq!(
            only_radius,
            PickSettings {
                radius: engine_units(40.0),
                ..ORIGINAL_PICK_DEFAULTS
            }
        );
    }

    #[test]
    fn a_setting_that_is_not_a_finite_number_above_zero_is_an_error_naming_it() {
        for (key, section) in [
            ("fActivatePickLength", "Interface"),
            ("fActivatePickRadius", "Interface"),
            ("fZKeyDelay", "Controls"),
        ] {
            for value in ["abc", "-1", "0", "inf", "NaN"] {
                if key == "fActivatePickRadius" && value == "0" {
                    continue;
                }
                let error = read(&format!(
                    "[{section}]
{key}={value}
"
                ))
                .expect_err(&format!("{key}={value}"));
                assert!(error.to_string().contains(key), "{key}={value}: {error}");
            }
        }
    }

    #[test]
    fn a_radius_of_zero_loads_as_the_exact_ray() {
        let settings = read(
            "[Interface]
fActivatePickRadius=0
",
        )
        .unwrap();
        assert_eq!(settings.radius.to_bits(), 0.0_f32.to_bits());
        assert_eq!(settings.reach, ORIGINAL_PICK_DEFAULTS.reach);
    }

    #[test]
    fn a_delay_too_long_for_a_duration_is_an_error() {
        assert!(
            read(
                "[Controls]
fZKeyDelay=1e30
"
            )
            .is_err()
        );
    }
}
