//! Replays the footstep marks of the first-person movement clips against the
//! simulated player. The player has no animation of its own, so the clock
//! walks a gait's cycle while the player really moves on the ground and
//! reports the marks it crosses; the world samples the surface per event.

use thiserror::Error;

use super::FIXED_DT;
use super::player::{COYOTE_TICKS, JUMP_WINDUP_TICKS, MOVE_EPSILON};
use crate::footsteps::{FootTag, Gait, GaitCycle, GaitTimings};

/// What the fixed step did to the player this tick.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TickObservation {
    /// The step ran to its end; false while a spawn or teleport is pending or collision is loading.
    pub completed: bool,
    pub grounded: bool,
    pub crouching: bool,
    pub sprint: bool,
    pub horizontal_displacement: f32,
    pub launched: bool,
}

#[derive(Debug, Default)]
pub(crate) struct ClockOutput {
    pub gait: Option<Gait>,
    pub tags: Vec<FootTag>,
}

#[derive(Debug, Clone, Copy)]
struct Stride {
    gait: Gait,
    phase: f32,
}

#[derive(Debug, Default)]
pub(crate) struct FootstepClock {
    timings: Option<GaitTimings>,
    logged_missing_timings: bool,
    stride: Option<Stride>,
    airborne_ticks: u32,
    settled: bool,
    jump_down_ticks_left: Option<u32>,
}

pub(crate) fn gait_for(crouching: bool, sprint: bool) -> Gait {
    if crouching {
        Gait::Sneak
    } else if sprint {
        Gait::Sprint
    } else {
        Gait::Run
    }
}

fn ticks_for(seconds: f32) -> u32 {
    (seconds / FIXED_DT).round() as u32
}

impl FootstepClock {
    pub fn install(&mut self, timings: GaitTimings) {
        self.timings = Some(timings);
        self.reset();
    }

    fn reset(&mut self) {
        self.stride = None;
        self.airborne_ticks = 0;
        self.settled = false;
        self.jump_down_ticks_left = None;
    }

    pub fn update(&mut self, obs: &TickObservation) -> ClockOutput {
        let mut output = ClockOutput::default();
        let Some(timings) = self.timings.take() else {
            if !self.logged_missing_timings {
                self.logged_missing_timings = true;
                tracing::debug!("footsteps: no gait timings installed, the player is silent");
            }
            return output;
        };
        if obs.completed {
            let gait = gait_for(obs.crouching, obs.sprint);
            output.gait = Some(gait);
            self.advance(&timings, obs, gait, &mut output.tags);
        } else {
            self.reset();
        }
        self.timings = Some(timings);
        output
    }

    fn advance(
        &mut self,
        timings: &GaitTimings,
        obs: &TickObservation,
        gait: Gait,
        tags: &mut Vec<FootTag>,
    ) {
        if obs.launched {
            tags.push(FootTag::JumpUp);
            self.stride = None;
            self.jump_down_ticks_left = None;
            return;
        }
        if !obs.grounded {
            self.jump_down_ticks_left = None;
            self.airborne_ticks = self.airborne_ticks.saturating_add(1);
            if self.airborne_ticks > COYOTE_TICKS {
                self.stride = None;
            }
            return;
        }
        let landed = self.settled && self.airborne_ticks > COYOTE_TICKS;
        self.airborne_ticks = 0;
        self.settled = true;
        self.count_down_jump_down(tags);
        if landed {
            self.jump_down_ticks_left = Some(ticks_for(timings.jump_down_offset()));
            if self.jump_down_ticks_left == Some(0) {
                self.jump_down_ticks_left = None;
                tags.push(FootTag::JumpDown);
            }
        }
        if obs.horizontal_displacement > MOVE_EPSILON {
            self.advance_stride(timings, gait, tags);
        } else {
            self.stride = None;
        }
    }

    fn count_down_jump_down(&mut self, tags: &mut Vec<FootTag>) {
        let Some(left) = self.jump_down_ticks_left else {
            return;
        };
        let left = left - 1;
        if left == 0 {
            self.jump_down_ticks_left = None;
            tags.push(FootTag::JumpDown);
        } else {
            self.jump_down_ticks_left = Some(left);
        }
    }

    fn advance_stride(&mut self, timings: &GaitTimings, gait: Gait, tags: &mut Vec<FootTag>) {
        let Some(cycle) = timings.cycle(gait) else {
            return;
        };
        let phase = match self.stride {
            None => {
                collect_marks(cycle, 0.0, FIXED_DT, true, tags);
                FIXED_DT
            }
            Some(stride) => {
                let from = if stride.gait == gait {
                    stride.phase
                } else {
                    let old_period = timings
                        .cycle(stride.gait)
                        .map_or(cycle.period(), GaitCycle::period);
                    stride.phase / old_period * cycle.period()
                };
                let to = from + FIXED_DT;
                if to >= cycle.period() {
                    let wrapped = to - cycle.period();
                    collect_marks(cycle, from, f32::INFINITY, false, tags);
                    collect_marks(cycle, 0.0, wrapped, true, tags);
                    wrapped
                } else {
                    collect_marks(cycle, from, to, false, tags);
                    to
                }
            }
        };
        self.stride = Some(Stride { gait, phase });
    }
}

fn collect_marks(
    cycle: &GaitCycle,
    from: f32,
    to: f32,
    inclusive_from: bool,
    tags: &mut Vec<FootTag>,
) {
    tags.extend(
        cycle
            .marks()
            .iter()
            .filter(|(time, _)| (*time > from || (inclusive_from && *time >= from)) && *time <= to)
            .map(|&(_, tag)| tag),
    );
}

/// The `JumpUp` mark of the jump clip and the sim's jump wind-up disagree by more than a tick.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
#[error(
    "jump wind-up of {windup_seconds:.4} s does not match the JumpUp mark at {jump_up_offset:.4} s (tolerance {tolerance:.4} s)"
)]
pub struct JumpWindupMismatch {
    pub windup_seconds: f32,
    pub jump_up_offset: f32,
    pub tolerance: f32,
}

pub fn jump_windup_mismatch(timings: &GaitTimings) -> Option<JumpWindupMismatch> {
    let windup_seconds = JUMP_WINDUP_TICKS as f32 * FIXED_DT;
    let jump_up_offset = timings.jump_up_offset();
    ((windup_seconds - jump_up_offset).abs() > FIXED_DT).then_some(JumpWindupMismatch {
        windup_seconds,
        jump_up_offset,
        tolerance: FIXED_DT,
    })
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use super::*;
    use crate::footsteps::FootTag::{
        FootLeft, FootRight, FootSprintLeft, FootSprintRight, JumpDown, JumpUp,
    };

    const STRIDE: f32 = 1.0;
    const JUMP_UP_OFFSET: f32 = 0.033;
    const JUMP_DOWN_OFFSET: f32 = 0.067;

    fn timings() -> GaitTimings {
        let cycle = |what, period, marks| GaitCycle::new(what, period, marks).unwrap();
        GaitTimings::new(
            cycle("run", 0.5, vec![(0.09, FootRight), (0.34, FootLeft)]),
            cycle(
                "sprint",
                1.0,
                vec![(0.2, FootSprintRight), (0.73, FootSprintLeft)],
            ),
            cycle("sneak", 0.8, vec![(0.11, FootRight), (0.51, FootLeft)]),
            JUMP_UP_OFFSET,
            JUMP_DOWN_OFFSET,
        )
        .unwrap()
    }

    fn installed() -> FootstepClock {
        let mut clock = FootstepClock::default();
        clock.install(timings());
        clock
    }

    fn moving() -> TickObservation {
        TickObservation {
            completed: true,
            grounded: true,
            crouching: false,
            sprint: false,
            horizontal_displacement: STRIDE,
            launched: false,
        }
    }

    fn standing() -> TickObservation {
        TickObservation {
            horizontal_displacement: 0.0,
            ..moving()
        }
    }

    fn airborne() -> TickObservation {
        TickObservation {
            grounded: false,
            ..moving()
        }
    }

    /// Runs `ticks` observations and lists (1-based tick, tag) of every event.
    fn run(
        clock: &mut FootstepClock,
        obs: &TickObservation,
        ticks: u32,
        first_tick: u32,
    ) -> Vec<(u32, FootTag)> {
        (first_tick..first_tick + ticks)
            .flat_map(|tick| {
                clock
                    .update(obs)
                    .tags
                    .into_iter()
                    .map(move |tag| (tick, tag))
            })
            .collect()
    }

    #[test]
    fn one_second_of_running_fires_each_mark_when_crossed_alternating_feet() {
        let events = run(&mut installed(), &moving(), 60, 1);
        assert_eq!(
            events,
            vec![
                (6, FootRight),
                (21, FootLeft),
                (36, FootRight),
                (51, FootLeft)
            ]
        );
    }

    #[test]
    fn nothing_fires_without_installed_timings() {
        let mut clock = FootstepClock::default();
        let output = clock.update(&moving());
        assert!(output.tags.is_empty() && output.gait.is_none());
        assert!(run(&mut clock, &moving(), 120, 1).is_empty());
    }

    #[test]
    fn standing_still_is_silent_and_starting_to_move_restarts_the_cycle() {
        let mut clock = installed();
        run(&mut clock, &moving(), 10, 1);
        assert!(run(&mut clock, &standing(), 40, 11).is_empty());
        let events = run(&mut clock, &moving(), 25, 51);
        assert_eq!(events, vec![(56, FootRight), (71, FootLeft)]);
    }

    #[test]
    fn a_mark_at_the_very_start_of_the_cycle_fires_on_the_first_moving_tick() {
        let mut clock = FootstepClock::default();
        let cycle = GaitCycle::new("c", 0.5, vec![(0.0, FootRight)]).unwrap();
        let other = GaitCycle::new("c", 0.5, vec![]).unwrap();
        clock.install(
            GaitTimings::new(
                cycle,
                other.clone(),
                other,
                JUMP_UP_OFFSET,
                JUMP_DOWN_OFFSET,
            )
            .unwrap(),
        );
        assert_eq!(
            run(&mut clock, &moving(), 31, 1),
            vec![(1, FootRight), (30, FootRight)]
        );
    }

    #[test]
    fn a_gait_change_keeps_the_phase_fraction_scaled_to_the_new_period() {
        let mut clock = installed();
        let early = run(&mut clock, &moving(), 12, 1);
        assert_eq!(early, vec![(6, FootRight)]);
        let sprint = TickObservation {
            sprint: true,
            ..moving()
        };
        // 0.2 s of a 0.5 s run is 0.4 of the cycle; the sprint cycle resumes at 0.4 s and
        // reaches its left mark at 0.73 s, 20 ticks later (restarting would take 44).
        let events = run(&mut clock, &sprint, 25, 13);
        assert_eq!(events, vec![(32, FootSprintLeft)]);
    }

    #[test]
    fn crouching_uses_the_sneak_cycle_even_while_sprint_is_requested() {
        let mut clock = installed();
        let sneaking = TickObservation {
            crouching: true,
            sprint: true,
            ..moving()
        };
        let output = clock.update(&sneaking);
        assert_eq!(output.gait, Some(Gait::Sneak));
        let events = run(&mut clock, &sneaking, 40, 2);
        assert_eq!(events, vec![(7, FootRight), (31, FootLeft)]);
    }

    #[test]
    fn gait_selection_follows_stance_then_sprint() {
        assert_eq!(gait_for(false, false), Gait::Run);
        assert_eq!(gait_for(false, true), Gait::Sprint);
        assert_eq!(gait_for(true, false), Gait::Sneak);
        assert_eq!(gait_for(true, true), Gait::Sneak);
    }

    #[test]
    fn airborne_ticks_are_silent_and_a_brief_flicker_keeps_the_stride() {
        let mut clock = installed();
        run(&mut clock, &moving(), 4, 1);
        assert!(run(&mut clock, &airborne(), 3, 5).is_empty());
        // Phase kept at 4 ticks: the right mark (0.09 s) comes at the 6th moving tick overall.
        assert_eq!(run(&mut clock, &moving(), 3, 8), vec![(9, FootRight)]);
    }

    #[test]
    fn a_pending_spawn_or_teleport_resets_the_clock() {
        let mut clock = installed();
        run(&mut clock, &moving(), 4, 1);
        let paused = TickObservation {
            completed: false,
            ..moving()
        };
        assert!(run(&mut clock, &paused, 3, 5).is_empty());
        assert_eq!(run(&mut clock, &moving(), 7, 8), vec![(13, FootRight)]);
    }

    #[test]
    fn a_launch_fires_jump_up_on_that_tick_and_the_cycle_restarts_on_landing() {
        let mut clock = installed();
        run(&mut clock, &moving(), 4, 1);
        let launch = TickObservation {
            launched: true,
            grounded: false,
            ..moving()
        };
        assert_eq!(run(&mut clock, &launch, 1, 5), vec![(5, JumpUp)]);
        assert!(run(&mut clock, &airborne(), 30, 6).is_empty());
        let after = run(&mut clock, &moving(), 12, 36);
        assert_eq!(after, vec![(40, JumpDown), (41, FootRight)]);
    }

    #[test]
    fn jump_down_fires_the_offset_after_the_touchdown_tick() {
        let mut clock = installed();
        run(&mut clock, &moving(), 1, 1);
        run(&mut clock, &airborne(), 20, 2);
        let landed = run(&mut clock, &standing(), 8, 22);
        assert_eq!(landed, vec![(26, JumpDown)]);
    }

    #[test]
    fn jump_down_is_cancelled_when_airborne_again_before_it_fires() {
        let mut clock = installed();
        run(&mut clock, &moving(), 1, 1);
        run(&mut clock, &airborne(), 20, 2);
        assert!(run(&mut clock, &standing(), 2, 22).is_empty());
        assert!(run(&mut clock, &airborne(), 1, 24).is_empty());
        assert!(run(&mut clock, &standing(), 8, 25).is_empty());
    }

    #[test]
    fn a_zero_jump_down_offset_fires_on_the_touchdown_tick() {
        let mut clock = FootstepClock::default();
        let cycle = GaitCycle::new("c", 1.0, vec![]).unwrap();
        clock.install(
            GaitTimings::new(cycle.clone(), cycle.clone(), cycle, JUMP_UP_OFFSET, 0.0).unwrap(),
        );
        run(&mut clock, &standing(), 1, 1);
        run(&mut clock, &airborne(), 20, 2);
        assert_eq!(run(&mut clock, &standing(), 1, 22), vec![(22, JumpDown)]);
    }

    #[test]
    fn the_first_touchdown_after_a_reset_is_silent() {
        let mut clock = installed();
        let paused = TickObservation {
            completed: false,
            ..moving()
        };
        run(&mut clock, &paused, 1, 1);
        run(&mut clock, &airborne(), 20, 2);
        assert!(run(&mut clock, &standing(), 10, 22).is_empty());
    }

    #[test]
    fn a_short_hop_inside_the_coyote_window_is_not_a_landing() {
        let mut clock = installed();
        run(&mut clock, &standing(), 1, 1);
        run(&mut clock, &airborne(), COYOTE_TICKS, 2);
        assert!(run(&mut clock, &standing(), 10, 20).is_empty());
    }

    #[test]
    fn the_jump_windup_matches_the_jump_up_mark_within_one_tick() {
        assert!(jump_windup_mismatch(&timings()).is_none());
        assert!((JUMP_WINDUP_TICKS as f32 * FIXED_DT - JUMP_UP_OFFSET).abs() <= FIXED_DT);
    }

    #[test]
    fn a_jump_up_mark_far_from_the_windup_is_reported() {
        let cycle = GaitCycle::new("c", 1.0, vec![]).unwrap();
        let far = GaitTimings::new(cycle.clone(), cycle.clone(), cycle, 0.2, 0.0).unwrap();
        let mismatch = jump_windup_mismatch(&far).expect("0.2 s is six ticks off");
        assert_eq!(mismatch.jump_up_offset, 0.2);
    }
}
