//! The black overlay's fade-out after a load finishes.

use super::capped_step;

/// Seconds the overlay takes to go from opaque to transparent.
///
/// The fade starts once the full-detail ring is loaded, while the LOD ring is still loading.
/// Warm loads measured the LOD finishing about 1.2 s after the full ring; the length is the
/// user's choice, so the distant terrain is in place by the time the fade ends.
pub const FADE_DURATION: f32 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Fade {
    elapsed: f32,
}

impl Fade {
    pub fn advance(&mut self, dt: f32) {
        self.elapsed = (self.elapsed + capped_step(dt)).min(FADE_DURATION);
    }

    /// Overlay opacity: 1 when the fade starts, 0 once it has finished.
    pub fn alpha(self) -> f32 {
        1.0 - self.elapsed / FADE_DURATION
    }

    pub fn is_finished(self) -> bool {
        self.elapsed >= FADE_DURATION
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the endpoints of the fade are exact by construction"
)]
mod tests {
    use super::super::MAX_FRAME_STEP_SECONDS;
    use super::*;

    #[test]
    fn a_new_fade_is_fully_opaque() {
        assert_eq!(Fade::default().alpha(), 1.0);
        assert!(!Fade::default().is_finished());
    }

    #[test]
    fn alpha_never_increases_and_stays_within_zero_and_one() {
        let mut fade = Fade::default();
        let mut previous = fade.alpha();
        for dt in [0.016, 0.0, 0.2, -1.0, f32::NAN, 0.033, 5.0, 0.016, 0.016] {
            fade.advance(dt);
            let alpha = fade.alpha();
            assert!((0.0..=1.0).contains(&alpha), "alpha {alpha} after dt {dt}");
            assert!(alpha <= previous, "alpha rose from {previous} to {alpha}");
            previous = alpha;
        }
    }

    #[test]
    fn a_huge_frame_time_is_capped_to_one_step() {
        let mut fade = Fade::default();
        fade.advance(10.0);
        assert!((fade.alpha() - (1.0 - MAX_FRAME_STEP_SECONDS / FADE_DURATION)).abs() < 1e-6);
        assert!(!fade.is_finished());
    }

    #[test]
    fn the_fade_finishes_after_its_duration_of_capped_frames() {
        let mut fade = Fade::default();
        let mut steps = 0;
        while !fade.is_finished() {
            fade.advance(1.0);
            steps += 1;
            assert!(steps <= 41, "the fade must end within its duration");
        }
        assert!(
            steps >= 40,
            "the fade must not end early, ended after {steps}"
        );
        assert_eq!(fade.alpha(), 0.0);
    }
}
