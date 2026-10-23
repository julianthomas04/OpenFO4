//! Turns the mouse wheel's fractional travel into whole list steps.

/// The most list rows one wheel event may move, so a runaway delta cannot overflow the list's
/// arithmetic.
pub const MAX_STEPS_PER_EVENT: i32 = 64;

/// Collects wheel travel in lines until it adds up to whole steps. A touchpad reports many small
/// deltas; each one alone would otherwise count as a full step.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WheelAccumulator {
    remainder: f32,
}

impl WheelAccumulator {
    /// Adds `lines` of travel (positive away from the user) and returns the whole steps that
    /// are now due, if any. Travel against the direction of the collected remainder discards
    /// the remainder, and a non-finite delta changes nothing.
    pub fn push(&mut self, lines: f32) -> Option<i32> {
        if !lines.is_finite() {
            return None;
        }
        if lines * self.remainder < 0.0 {
            self.remainder = 0.0;
        }
        let total = self.remainder + lines;
        let whole = total.trunc();
        self.remainder = total - whole;
        let steps = (whole as i32).clamp(-MAX_STEPS_PER_EVENT, MAX_STEPS_PER_EVENT);
        (steps != 0).then_some(steps)
    }

    pub fn reset(&mut self) {
        self.remainder = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_lines_pass_through_at_once() {
        let mut wheel = WheelAccumulator::default();
        assert_eq!(wheel.push(1.0), Some(1));
        assert_eq!(wheel.push(-3.0), Some(-3));
        assert_eq!(wheel.push(2.0), Some(2));
    }

    #[test]
    fn small_deltas_add_up_to_a_step_instead_of_counting_as_one_each() {
        let mut wheel = WheelAccumulator::default();
        let steps: Vec<_> = (0..10).filter_map(|_| wheel.push(0.25)).collect();
        assert_eq!(steps, [1, 1], "ten quarter lines are two and a half steps");
    }

    #[test]
    fn a_fractional_delta_alone_is_no_step() {
        let mut wheel = WheelAccumulator::default();
        assert_eq!(wheel.push(0.2), None);
        assert_eq!(wheel.push(-0.2), None);
    }

    #[test]
    fn the_fraction_of_a_larger_delta_is_kept_for_the_next_one() {
        let mut wheel = WheelAccumulator::default();
        assert_eq!(wheel.push(2.75), Some(2));
        assert_eq!(wheel.push(0.25), Some(1));
        assert_eq!(wheel.push(0.0), None);
    }

    #[test]
    fn reversing_the_direction_drops_the_collected_remainder() {
        let mut wheel = WheelAccumulator::default();
        assert_eq!(wheel.push(0.75), None);
        assert_eq!(wheel.push(-0.5), None, "not -0.5 + 0.75 = a quarter back");
        assert_eq!(wheel.push(-0.5), Some(-1));
    }

    #[test]
    fn a_reset_forgets_the_remainder() {
        let mut wheel = WheelAccumulator::default();
        wheel.push(0.75);
        wheel.reset();
        assert_eq!(wheel.push(0.5), None);
    }

    #[test]
    fn non_finite_deltas_change_nothing() {
        let mut wheel = WheelAccumulator::default();
        wheel.push(0.5);
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(wheel.push(bad), None);
        }
        assert_eq!(wheel.push(0.5), Some(1), "the collected half line survived");
    }

    #[test]
    fn a_huge_delta_is_clamped_to_the_step_limit() {
        let mut wheel = WheelAccumulator::default();
        assert_eq!(wheel.push(1.0e9), Some(MAX_STEPS_PER_EVENT));
        assert_eq!(wheel.push(-1.0e9), Some(-MAX_STEPS_PER_EVENT));
    }
}
