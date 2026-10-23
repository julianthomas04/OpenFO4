//! Distance fade and draw distance of the grass (`fGrassStartFadeDistance`, the fade range,
//! `uGridsToLoad`), kept in engine units.

use glam::Vec3;

use crate::assets::ini::{GameIni, IniError, IniKey};
use crate::assets::world_settings::LoadedGrid;
use crate::bethesda_space::UNITS_TO_ENGINE;
use crate::rendering::frustum::Aabb;

/// The fade range (`fGrassFadeRange`); no INI sets it, so it is the game's built-in value.
const RANGE_GAME_UNITS: f32 = 1000.0;
const START_FADE: IniKey = IniKey::new("Grass", "fGrassStartFadeDistance");
const MIN_START_FADE: IniKey = IniKey::new("Grass", "fGrassMinStartFadeDistance");
const MAX_START_FADE: IniKey = IniKey::new("Grass", "fGrassMaxStartFadeDistance");
/// Fade range slider bounds; vanilla defines none, the upper end lets the grid cap bite.
pub const RANGE_RANGE_GAME_UNITS: (f32, f32) = (100.0, 15000.0);

/// What the INI sets the fade to and bounds it by: the configured start, the start slider's
/// range and the loaded cell block (`uGridsToLoad`) that caps the draw distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FadeLimits {
    /// `fGrassStartFadeDistance`, clamped into `start_game_units`.
    pub configured_start_game_units: f32,
    pub start_game_units: (f32, f32),
    pub grid_cap_game_units: f32,
}

impl FadeLimits {
    pub fn from_ini(ini: &GameIni, loaded_grid: LoadedGrid) -> Result<Self, IniError> {
        let min = ini.require_non_negative(MIN_START_FADE)?;
        let max = ini.require_valid::<f32>(
            MAX_START_FADE,
            &format!("a finite number, at least {} ({min})", MIN_START_FADE.name),
            |max| max.is_finite() && *max >= min,
        )?;
        let start = ini.require_non_negative(START_FADE)?;
        if start < min || start > max {
            tracing::warn!(
                "INI setting {}={start} is outside {} to {} ({min} to {max}); using the nearest bound",
                START_FADE.name,
                MIN_START_FADE.name,
                MAX_START_FADE.name,
            );
        }
        Ok(Self {
            configured_start_game_units: start.clamp(min, max),
            start_game_units: (min, max),
            grid_cap_game_units: loaded_grid.extent_game_units(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrassFade {
    pub start: f32,
    pub range: f32,
}

impl GrassFade {
    /// The fade the INI configures.
    pub fn configured(limits: &FadeLimits) -> Self {
        Self {
            start: limits.configured_start_game_units * UNITS_TO_ENGINE,
            range: RANGE_GAME_UNITS * UNITS_TO_ENGINE,
        }
    }

    /// Beyond this distance every blade has faded out:
    /// `min(start + range, uGridsToLoad * 4096)`.
    pub fn draw_distance(self, limits: &FadeLimits) -> f32 {
        (self.start + self.range).min(limits.grid_cap_game_units * UNITS_TO_ENGINE)
    }
}

/// The panel's fade override, in Bethesda units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FadeOverride {
    pub start_game_units: f32,
    pub range_game_units: f32,
}

impl FadeOverride {
    /// The fade the INI configures, which the panel starts at and resets to.
    pub fn configured(limits: &FadeLimits) -> Self {
        Self {
            start_game_units: limits.configured_start_game_units,
            range_game_units: RANGE_GAME_UNITS,
        }
    }

    /// The fade the stage applies, with start and range clamped to their slider
    /// bounds, plus the grid-capped draw distance in Bethesda units.
    pub fn resolve(self, limits: &FadeLimits) -> ResolvedFade {
        let start = self
            .start_game_units
            .clamp(limits.start_game_units.0, limits.start_game_units.1);
        let range = self
            .range_game_units
            .clamp(RANGE_RANGE_GAME_UNITS.0, RANGE_RANGE_GAME_UNITS.1);
        let fade = GrassFade {
            start: start * UNITS_TO_ENGINE,
            range: range * UNITS_TO_ENGINE,
        };
        let wanted = start + range;
        let cap = limits.grid_cap_game_units;
        ResolvedFade {
            fade,
            draw_distance_game_units: wanted.min(cap),
            cap_game_units: cap,
            clamped: wanted > cap,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedFade {
    pub fade: GrassFade,
    pub draw_distance_game_units: f32,
    pub cap_game_units: f32,
    /// `start + range` exceeds what the loaded cell block supplies.
    pub clamped: bool,
}

/// Distance from `point` to the nearest point of `aabb`; 0 inside it.
pub fn distance_to_aabb(point: Vec3, aabb: &Aabb) -> f32 {
    let nearest = point.clamp(aabb.min, aabb.max);
    point.distance(nearest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::world_settings::test_support::grid_of_side;

    fn limits_of(text: &str, side: u32) -> Result<FadeLimits, IniError> {
        FadeLimits::from_ini(&GameIni::from_texts([text]), grid_of_side(side))
    }

    fn grass_ini(min: f32, max: f32, start: f32) -> String {
        format!(
            "fGrassMinStartFadeDistance={min}\nfGrassMaxStartFadeDistance={max}\n\
             fGrassStartFadeDistance={start}\n"
        )
    }

    fn limits() -> FadeLimits {
        limits_of(&grass_ini(400.0, 7000.0, 3200.0), 5).unwrap()
    }

    #[test]
    fn limits_read_the_start_bounds_and_grid_cap_from_the_ini() {
        let limits = limits_of(&grass_ini(600.0, 9000.0, 5000.0), 7).unwrap();
        assert_eq!(limits.start_game_units, (600.0, 9000.0));
        close(limits.grid_cap_game_units, 28672.0);
    }

    #[test]
    fn the_configured_start_is_the_ini_value_inside_the_bounds() {
        close(limits().configured_start_game_units, 3200.0);
        for start in [400.0, 7000.0] {
            let limits = limits_of(&grass_ini(400.0, 7000.0, start), 5).unwrap();
            close(limits.configured_start_game_units, start);
        }
    }

    #[test]
    fn a_configured_start_outside_the_bounds_is_clamped_to_the_nearest_one() {
        for (start, expected) in [
            (0.0, 400.0),
            (399.0, 400.0),
            (7001.0, 7000.0),
            (20000.0, 7000.0),
        ] {
            let limits = limits_of(&grass_ini(400.0, 7000.0, start), 5).unwrap();
            close(limits.configured_start_game_units, expected);
        }
    }

    #[test]
    fn equal_start_bounds_are_accepted() {
        let limits = limits_of(&grass_ini(500.0, 500.0, 500.0), 5).unwrap();
        assert_eq!(limits.start_game_units, (500.0, 500.0));
    }

    #[test]
    fn inverted_start_bounds_are_an_error_naming_both_keys() {
        let error = limits_of(&grass_ini(8000.0, 100.0, 500.0), 5).unwrap_err();
        let message = error.to_string();
        assert!(matches!(error, IniError::Invalid { .. }), "{message}");
        assert!(message.contains("fGrassMaxStartFadeDistance"), "{message}");
        assert!(message.contains("fGrassMinStartFadeDistance"), "{message}");
    }

    #[test]
    fn an_absent_start_setting_is_an_error_naming_it() {
        for absent in [
            "fGrassMinStartFadeDistance",
            "fGrassMaxStartFadeDistance",
            "fGrassStartFadeDistance",
        ] {
            let text: String = grass_ini(400.0, 7000.0, 3200.0)
                .lines()
                .filter(|line| !line.starts_with(absent))
                .collect::<Vec<_>>()
                .join("\n");
            let error = limits_of(&text, 5).unwrap_err();
            assert!(
                matches!(&error, IniError::MissingKey { key, .. } if *key == absent),
                "{absent}: {error}"
            );
        }
    }

    #[test]
    fn a_negative_or_unparsable_start_setting_is_an_error_naming_it() {
        for (text, key) in [
            (
                grass_ini(-1.0, 7000.0, 3200.0),
                "fGrassMinStartFadeDistance",
            ),
            (grass_ini(400.0, 7000.0, -1.0), "fGrassStartFadeDistance"),
            (
                grass_ini(400.0, 7000.0, 3200.0).replace("=3200", "=far"),
                "fGrassStartFadeDistance",
            ),
        ] {
            let error = limits_of(&text, 5).unwrap_err();
            assert!(error.to_string().contains(key), "{text}: {error}");
        }
    }

    fn close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-3,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn the_configured_fade_is_the_ini_start_plus_the_fixed_range_in_engine_units() {
        let fade = GrassFade::configured(&limits());
        close(fade.start, 320.0);
        close(fade.range, 100.0);
    }

    #[test]
    fn draw_distance_is_start_plus_range_while_below_the_grid_cap() {
        close(
            GrassFade::configured(&limits()).draw_distance(&limits()),
            420.0,
        );
    }

    #[test]
    fn draw_distance_is_capped_by_the_loaded_grid_block() {
        let far = GrassFade {
            start: 7000.0,
            range: 1000.0,
        };
        close(far.draw_distance(&limits()), 2048.0);
    }

    #[test]
    fn the_configured_override_resolves_to_the_configured_fade_unclamped() {
        let limits = limits();
        let resolved = FadeOverride::configured(&limits).resolve(&limits);
        assert_eq!(resolved.fade, GrassFade::configured(&limits));
        close(resolved.draw_distance_game_units, 4200.0);
        close(resolved.cap_game_units, 20480.0);
        assert!(!resolved.clamped);
    }

    #[test]
    fn override_is_clamped_to_the_slider_bounds() {
        let resolved = FadeOverride {
            start_game_units: 10.0,
            range_game_units: 99999.0,
        }
        .resolve(&limits());
        close(resolved.fade.start, 40.0);
        close(resolved.fade.range, 1500.0);
    }

    #[test]
    fn draw_distance_is_capped_at_five_cells_and_reports_the_clamp() {
        let resolved = FadeOverride {
            start_game_units: 7000.0,
            range_game_units: 15000.0,
        }
        .resolve(&limits());
        close(resolved.draw_distance_game_units, 20480.0);
        assert!(resolved.clamped);
        close(resolved.fade.draw_distance(&limits()), 2048.0);
    }

    #[test]
    fn exactly_reaching_the_cap_is_not_a_clamp() {
        let resolved = FadeOverride {
            start_game_units: 5480.0,
            range_game_units: 15000.0,
        }
        .resolve(&limits());
        close(resolved.draw_distance_game_units, 20480.0);
        assert!(!resolved.clamped);
    }

    #[test]
    fn distance_to_aabb_is_zero_inside_and_euclidean_outside() {
        let aabb = Aabb {
            min: Vec3::new(0.0, 0.0, 0.0),
            max: Vec3::new(10.0, 10.0, 10.0),
        };
        close(distance_to_aabb(Vec3::new(5.0, 5.0, 5.0), &aabb), 0.0);
        close(distance_to_aabb(Vec3::new(13.0, 14.0, 5.0), &aabb), 5.0);
        close(distance_to_aabb(Vec3::new(-2.0, 5.0, 5.0), &aabb), 2.0);
    }
}
