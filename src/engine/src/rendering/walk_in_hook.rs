//! `OPENFO4_AUTO_WALK_IN="ref,ref,..."`: a dev hook that walks the player into the listed walk-in
//! doors so the real detector fires, where OS input cannot be injected. Pure state machine; the
//! renderer feeds it per-frame facts and carries out and logs what it asks for.

use std::collections::VecDeque;

/// Frames to let a teleport or placement settle (respawn onto the ground) before judging it.
const SETTLE_FRAMES: u32 = 30;
/// Frames the detector gets to fire after the player crossed a trigger.
const FIRE_TIMEOUT_FRAMES: u32 = 120;
/// Frames the arrival after a fire may take before it is reported as missing.
const ARRIVAL_TIMEOUT_FRAMES: u32 = 1800;

/// What the renderer observed this frame.
#[derive(Debug, Clone, Copy)]
pub(super) struct HookFrame<'a> {
    pub player_spawned: bool,
    pub teleport_pending: bool,
    /// The walk-in door the real detector fired for this frame.
    pub fired: Option<&'a str>,
}

/// What the renderer must do or log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HookAction {
    None,
    /// Put the player on the approach side of the door's trigger.
    PlaceAtApproach {
        source_ref: String,
    },
    /// Put the player on the far side, so the move crosses the trigger.
    CrossThrough {
        source_ref: String,
    },
    Fired {
        source_ref: String,
    },
    DidNotFire {
        source_ref: String,
    },
    Arrived {
        source_ref: String,
    },
    DidNotArrive {
        source_ref: String,
    },
    /// A walk-in fired during the quiet period after arrival.
    FiredAfterArrival {
        arrived_ref: String,
        fired_ref: String,
    },
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Ready { cooldown: u32 },
    Placed { waited: u32 },
    Crossing { waited: u32 },
    Fired { waited: u32 },
    Quiet { remaining: u32 },
    Done,
}

#[derive(Debug)]
pub(super) struct WalkInDevHook {
    pending: VecDeque<String>,
    current: Option<String>,
    phase: Phase,
    quiet_frames: u32,
}

impl WalkInDevHook {
    /// `list` is the environment variable: comma-separated refs, blanks ignored. `quiet_frames`
    /// is both the wait before each hop and the no-refire window after each arrival.
    pub(super) fn from_list(list: &str, quiet_frames: u32) -> Self {
        Self {
            pending: list
                .split(',')
                .map(str::trim)
                .filter(|source_ref| !source_ref.is_empty())
                .map(str::to_owned)
                .collect(),
            current: None,
            phase: Phase::Ready { cooldown: 0 },
            quiet_frames,
        }
    }

    pub(super) fn is_active(&self) -> bool {
        self.phase != Phase::Done
    }

    /// The renderer found no trigger for the current door: give up on it.
    pub(super) fn skip_current(&mut self) {
        self.current = None;
        self.phase = Phase::Ready { cooldown: 0 };
    }

    pub(super) fn tick(&mut self, frame: HookFrame<'_>) -> HookAction {
        match self.phase {
            Phase::Done => HookAction::None,
            Phase::Ready { cooldown } if cooldown > 0 => {
                self.phase = Phase::Ready {
                    cooldown: cooldown - 1,
                };
                HookAction::None
            }
            Phase::Ready { .. } => self.begin_next(frame),
            Phase::Placed { waited } => self.after_placement(waited, frame),
            Phase::Crossing { waited } => self.while_crossing(waited, frame),
            Phase::Fired { waited } => self.while_arriving(waited, frame),
            Phase::Quiet { remaining } => self.while_quiet(remaining, frame),
        }
    }

    fn begin_next(&mut self, frame: HookFrame<'_>) -> HookAction {
        if !frame.player_spawned || frame.teleport_pending {
            return HookAction::None;
        }
        let Some(source_ref) = self.pending.pop_front() else {
            self.phase = Phase::Done;
            return HookAction::Finished;
        };
        self.current = Some(source_ref.clone());
        self.phase = Phase::Placed { waited: 0 };
        HookAction::PlaceAtApproach { source_ref }
    }

    fn after_placement(&mut self, waited: u32, frame: HookFrame<'_>) -> HookAction {
        let waited = waited + 1;
        if waited < SETTLE_FRAMES || !frame.player_spawned {
            self.phase = Phase::Placed { waited };
            return HookAction::None;
        }
        self.phase = Phase::Crossing { waited: 0 };
        HookAction::CrossThrough {
            source_ref: self.current_ref(),
        }
    }

    fn while_crossing(&mut self, waited: u32, frame: HookFrame<'_>) -> HookAction {
        let source_ref = self.current_ref();
        if frame.fired == Some(source_ref.as_str()) {
            self.phase = Phase::Fired { waited: 0 };
            return HookAction::Fired { source_ref };
        }
        let waited = waited + 1;
        if waited >= FIRE_TIMEOUT_FRAMES {
            self.phase = Phase::Ready {
                cooldown: self.quiet_frames,
            };
            return HookAction::DidNotFire { source_ref };
        }
        self.phase = Phase::Crossing { waited };
        HookAction::None
    }

    fn while_arriving(&mut self, waited: u32, frame: HookFrame<'_>) -> HookAction {
        let waited = waited + 1;
        if waited >= SETTLE_FRAMES && frame.player_spawned && !frame.teleport_pending {
            self.phase = Phase::Quiet {
                remaining: self.quiet_frames,
            };
            return HookAction::Arrived {
                source_ref: self.current_ref(),
            };
        }
        if waited >= ARRIVAL_TIMEOUT_FRAMES {
            self.phase = Phase::Ready {
                cooldown: self.quiet_frames,
            };
            return HookAction::DidNotArrive {
                source_ref: self.current_ref(),
            };
        }
        self.phase = Phase::Fired { waited };
        HookAction::None
    }

    fn while_quiet(&mut self, remaining: u32, frame: HookFrame<'_>) -> HookAction {
        if let Some(fired) = frame.fired {
            return HookAction::FiredAfterArrival {
                arrived_ref: self.current_ref(),
                fired_ref: fired.to_owned(),
            };
        }
        self.phase = match remaining.checked_sub(1) {
            Some(left) if left > 0 => Phase::Quiet { remaining: left },
            _ => Phase::Ready { cooldown: 0 },
        };
        HookAction::None
    }

    fn current_ref(&self) -> String {
        self.current
            .clone()
            .expect("a hook phase past Ready has a current door")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUIET: u32 = 5;

    fn idle() -> HookFrame<'static> {
        HookFrame {
            player_spawned: true,
            teleport_pending: false,
            fired: None,
        }
    }

    fn firing(source_ref: &str) -> HookFrame<'_> {
        HookFrame {
            fired: Some(source_ref),
            ..idle()
        }
    }

    fn hook(list: &str) -> WalkInDevHook {
        WalkInDevHook::from_list(list, QUIET)
    }

    /// Ticks until something other than `None` is asked for, within a frame budget.
    fn next_action(hook: &mut WalkInDevHook, frame: HookFrame<'_>) -> HookAction {
        for _ in 0..5000 {
            let action = hook.tick(frame);
            if action != HookAction::None {
                return action;
            }
        }
        panic!("the hook asked for nothing within 5000 frames");
    }

    fn placed_and_crossing(hook: &mut WalkInDevHook) {
        assert!(matches!(
            next_action(hook, idle()),
            HookAction::PlaceAtApproach { .. }
        ));
        assert!(matches!(
            next_action(hook, idle()),
            HookAction::CrossThrough { .. }
        ));
    }

    fn action(name: &str, source_ref: &str) -> HookAction {
        let source_ref = source_ref.to_owned();
        match name {
            "place" => HookAction::PlaceAtApproach { source_ref },
            "cross" => HookAction::CrossThrough { source_ref },
            "fired" => HookAction::Fired { source_ref },
            "no_fire" => HookAction::DidNotFire { source_ref },
            "arrived" => HookAction::Arrived { source_ref },
            "no_arrival" => HookAction::DidNotArrive { source_ref },
            other => panic!("unknown action {other}"),
        }
    }

    #[test]
    fn the_list_is_split_on_commas_ignoring_blanks_and_spaces() {
        assert_eq!(hook(" a , ,b,").pending, ["a", "b"]);
    }

    #[test]
    fn an_empty_list_finishes_at_once() {
        let mut hook = hook("");
        assert_eq!(hook.tick(idle()), HookAction::Finished);
        assert!(!hook.is_active());
        assert_eq!(hook.tick(idle()), HookAction::None);
    }

    #[test]
    fn it_places_then_crosses_then_reports_the_fire_and_the_arrival() {
        let mut hook = hook("A");
        assert_eq!(hook.tick(idle()), action("place", "A"));
        assert_eq!(next_action(&mut hook, idle()), action("cross", "A"));
        assert_eq!(hook.tick(firing("A")), action("fired", "A"));
        assert_eq!(next_action(&mut hook, idle()), action("arrived", "A"));
    }

    #[test]
    fn it_waits_for_a_spawned_player_without_a_pending_teleport() {
        let mut hook = hook("A");
        let unspawned = HookFrame {
            player_spawned: false,
            ..idle()
        };
        let pending = HookFrame {
            teleport_pending: true,
            ..idle()
        };
        assert_eq!(hook.tick(unspawned), HookAction::None);
        assert_eq!(hook.tick(pending), HookAction::None);
        assert_eq!(hook.tick(idle()), action("place", "A"));
    }

    #[test]
    fn a_door_that_never_fires_is_reported_and_the_next_one_follows() {
        let mut hook = hook("A,B");
        placed_and_crossing(&mut hook);
        assert_eq!(next_action(&mut hook, idle()), action("no_fire", "A"));
        assert_eq!(next_action(&mut hook, idle()), action("place", "B"));
    }

    #[test]
    fn a_different_door_firing_while_crossing_does_not_count() {
        let mut hook = hook("A");
        placed_and_crossing(&mut hook);
        assert_eq!(hook.tick(firing("OTHER")), HookAction::None);
    }

    #[test]
    fn a_fire_during_the_quiet_period_after_arrival_is_reported() {
        let mut hook = hook("A");
        placed_and_crossing(&mut hook);
        hook.tick(firing("A"));
        next_action(&mut hook, idle());
        assert_eq!(
            hook.tick(firing("B")),
            HookAction::FiredAfterArrival {
                arrived_ref: "A".into(),
                fired_ref: "B".into()
            }
        );
    }

    #[test]
    fn a_quiet_period_without_fires_ends_with_finished_after_the_last_door() {
        let mut hook = hook("A");
        placed_and_crossing(&mut hook);
        hook.tick(firing("A"));
        next_action(&mut hook, idle());
        assert_eq!(next_action(&mut hook, idle()), HookAction::Finished);
    }

    #[test]
    fn an_arrival_that_never_completes_is_reported() {
        let mut hook = hook("A");
        placed_and_crossing(&mut hook);
        hook.tick(firing("A"));
        let stuck = HookFrame {
            teleport_pending: true,
            ..idle()
        };
        assert_eq!(next_action(&mut hook, stuck), action("no_arrival", "A"));
    }

    #[test]
    fn skipping_a_door_without_a_trigger_moves_on() {
        let mut hook = hook("A,B");
        hook.tick(idle());
        hook.skip_current();
        assert_eq!(hook.tick(idle()), action("place", "B"));
    }
}
