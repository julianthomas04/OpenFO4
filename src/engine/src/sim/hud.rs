//! Read-only HUD snapshot the render layer consumes once per frame. Pure data
//! (glam/std only) — the sim never reads it back. Mirrors the `player_debug()`
//! accessor pattern in [`super::SimWorld`]: gameplay values (max HP/AP,
//! percentages) are derived sim-side; the render layer sees only final numbers.
//!
//! This is the native-engine replacement for FO4's per-widget Scaleform value
//! feed (the `HudSnapshot`). New channels
//! (compass, stealth, markers, ...) are added here as the matching systems land.

/// One meter bar's render state: a fill fraction pre-clamped to `0..=1`
/// sim-side, so the render layer only scales a quad by it. (FO4's HUD bars are
/// unlabeled by value — the numeric HP/AP belong to the Pip-Boy, not the HUD
/// bar — and the vanilla bar does not pulse at low HP; the low-health cue is the
/// screen-edge vignette, a separate effect.)
#[derive(Clone, Copy, Debug)]
pub struct MeterFill {
    pub fraction: f32,
}

impl MeterFill {
    pub(super) fn new(current: f32, max: f32) -> Self {
        // Guard non-finite explicitly: `(NaN).clamp(..)` is NaN (NaN compares
        // false), and a NaN fraction would slip past the render-side `w <= 0`
        // emptiness check (NaN <= 0 is false) and emit garbage geometry.
        let fraction = if max > 0.0 && current.is_finite() && max.is_finite() {
            (current / max).clamp(0.0, 1.0)
        } else {
            0.0
        };
        MeterFill { fraction }
    }
}

/// Crosshair reticle state. The payload lives in the variant so impossible
/// combinations cannot be represented (a spread radius while hidden, a prompt
/// while showing the bare dot). Maps onto FO4's crosshair requested-state field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CrosshairState {
    /// No reticle (e.g. before the player has spawned).
    Hidden,
    /// Hip-fire / ADS dot. Reserved; not emitted sim-side in this version.
    Dot,
    /// The 4-tick reticle. `requested_radius` is the spread in virtual pixels.
    Standard { requested_radius: f32 },
    /// Looking at an interactable (selected render-side via the door-aim ray).
    Activate,
    // `Command` (companion-command mode) reserved; not emitted yet.
}

/// Everything the native HUD needs for one frame.
#[derive(Clone, Copy, Debug)]
pub struct HudSnapshot {
    /// Whether to show the HUD at all this frame (false until the player has
    /// spawned). Gates meters and crosshair together.
    pub visible: bool,
    pub hp: MeterFill,
    pub ap: MeterFill,
    pub crosshair: CrosshairState,
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use super::*;

    #[test]
    fn fraction_is_current_over_max() {
        assert_eq!(MeterFill::new(50.0, 200.0).fraction, 0.25);
    }

    #[test]
    fn fraction_is_clamped_to_the_unit_interval() {
        assert_eq!(MeterFill::new(500.0, 100.0).fraction, 1.0);
        assert_eq!(MeterFill::new(-5.0, 100.0).fraction, 0.0);
    }

    #[test]
    fn a_non_positive_or_non_finite_input_gives_an_empty_bar_never_nan() {
        for (current, max) in [
            (10.0, 0.0),
            (10.0, -1.0),
            (f32::NAN, 100.0),
            (10.0, f32::NAN),
            (f32::INFINITY, 100.0),
            (10.0, f32::INFINITY),
        ] {
            let fraction = MeterFill::new(current, max).fraction;
            assert_eq!(fraction, 0.0, "current {current} max {max}");
        }
    }
}
