use thiserror::Error;

use super::events::{FootTag, Gait};

#[derive(Debug, Error, PartialEq)]
pub enum GaitTimingsError {
    #[error("{what}: cycle period {period} s must be finite and greater than zero")]
    InvalidPeriod { what: &'static str, period: f32 },
    #[error("{what}: mark {tag:?} at {time} s is outside the cycle [0, {period}) s")]
    MarkOutOfCycle {
        what: &'static str,
        tag: FootTag,
        time: f32,
        period: f32,
    },
    #[error("{what}: offset {offset} s must be finite and not negative")]
    InvalidOffset { what: &'static str, offset: f32 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct GaitCycle {
    period: f32,
    marks: Vec<(f32, FootTag)>,
}

impl GaitCycle {
    /// Marks are sorted by time; equal times keep their given order.
    pub fn new(
        what: &'static str,
        period: f32,
        mut marks: Vec<(f32, FootTag)>,
    ) -> Result<Self, GaitTimingsError> {
        if !period.is_finite() || period <= 0.0 {
            return Err(GaitTimingsError::InvalidPeriod { what, period });
        }
        if let Some(&(time, tag)) = marks
            .iter()
            .find(|(time, _)| !time.is_finite() || *time < 0.0 || *time >= period)
        {
            return Err(GaitTimingsError::MarkOutOfCycle {
                what,
                tag,
                time,
                period,
            });
        }
        marks.sort_by(|a, b| a.0.total_cmp(&b.0));
        Ok(Self { period, marks })
    }

    pub fn period(&self) -> f32 {
        self.period
    }

    pub fn marks(&self) -> &[(f32, FootTag)] {
        &self.marks
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GaitTimings {
    run: GaitCycle,
    sprint: GaitCycle,
    sneak: GaitCycle,
    jump_up_offset: f32,
    jump_down_offset: f32,
}

impl GaitTimings {
    /// `jump_up_offset` is the time of the `JumpUp` mark inside the jump clip and
    /// `jump_down_offset` the time of the `JumpDown` mark inside the landing clip.
    pub fn new(
        run: GaitCycle,
        sprint: GaitCycle,
        sneak: GaitCycle,
        jump_up_offset: f32,
        jump_down_offset: f32,
    ) -> Result<Self, GaitTimingsError> {
        for (what, offset) in [("jump up", jump_up_offset), ("jump down", jump_down_offset)] {
            if !offset.is_finite() || offset < 0.0 {
                return Err(GaitTimingsError::InvalidOffset { what, offset });
            }
        }
        Ok(Self {
            run,
            sprint,
            sneak,
            jump_up_offset,
            jump_down_offset,
        })
    }

    /// `None` for gaits the player never moves in (walk, swim).
    pub fn cycle(&self, gait: Gait) -> Option<&GaitCycle> {
        match gait {
            Gait::Run => Some(&self.run),
            Gait::Sprint => Some(&self.sprint),
            Gait::Sneak => Some(&self.sneak),
            Gait::Walk | Gait::Swim => None,
        }
    }

    pub fn jump_up_offset(&self) -> f32 {
        self.jump_up_offset
    }

    pub fn jump_down_offset(&self) -> f32 {
        self.jump_down_offset
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use super::*;

    #[test]
    fn marks_are_sorted_by_time() {
        let cycle = GaitCycle::new(
            "run",
            1.0,
            vec![(0.6, FootTag::FootLeft), (0.1, FootTag::FootRight)],
        )
        .unwrap();
        assert_eq!(
            cycle.marks(),
            &[(0.1, FootTag::FootRight), (0.6, FootTag::FootLeft)]
        );
    }

    #[test]
    fn non_positive_or_non_finite_period_is_rejected() {
        for period in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(matches!(
                GaitCycle::new("run", period, vec![]),
                Err(GaitTimingsError::InvalidPeriod { .. })
            ));
        }
    }

    #[test]
    fn mark_at_the_period_or_negative_is_rejected_but_zero_is_allowed() {
        assert!(GaitCycle::new("run", 1.0, vec![(0.0, FootTag::FootLeft)]).is_ok());
        for time in [1.0, 1.5, -0.1, f32::NAN] {
            assert!(matches!(
                GaitCycle::new("run", 1.0, vec![(time, FootTag::FootLeft)]),
                Err(GaitTimingsError::MarkOutOfCycle { .. })
            ));
        }
    }

    #[test]
    fn negative_or_non_finite_jump_offsets_are_rejected() {
        let cycle = || GaitCycle::new("c", 1.0, vec![]).unwrap();
        for bad in [-0.1, f32::NAN] {
            assert!(GaitTimings::new(cycle(), cycle(), cycle(), bad, 0.0).is_err());
            assert!(GaitTimings::new(cycle(), cycle(), cycle(), 0.0, bad).is_err());
        }
        assert!(GaitTimings::new(cycle(), cycle(), cycle(), 0.0, 0.0).is_ok());
    }

    #[test]
    fn only_run_sprint_and_sneak_have_cycles() {
        let cycle = |period| GaitCycle::new("c", period, vec![]).unwrap();
        let timings = GaitTimings::new(cycle(1.0), cycle(2.0), cycle(3.0), 0.0, 0.0).unwrap();
        assert_eq!(timings.cycle(Gait::Run).unwrap().period(), 1.0);
        assert_eq!(timings.cycle(Gait::Sprint).unwrap().period(), 2.0);
        assert_eq!(timings.cycle(Gait::Sneak).unwrap().period(), 3.0);
        assert!(timings.cycle(Gait::Walk).is_none());
        assert!(timings.cycle(Gait::Swim).is_none());
    }
}
