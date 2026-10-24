//! The ready sequence of a load: which readiness signals release the physics fold hold and
//! when the loading screen may reveal the world.

use std::fmt;

use super::capped_step;

/// Consecutive frames the player must be ready before the world is revealed.
pub const READY_FRAMES: u32 = 2;
/// Seconds without any change in the load's progress after which the load counts as stalled.
pub const NO_PROGRESS_TIMEOUT_SECONDS: f32 = 120.0;

/// How far the streamer has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StreamCounts {
    pub pending_cells: usize,
    pub pending_lod_cells: usize,
    pub loaded_cells: usize,
    pub loaded_lod_cells: usize,
}

/// Everything whose change counts as the load making progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProgressCounts {
    pub stream: StreamCounts,
    /// Colliders in the physics world; it moves while the budgeted collider queue drains.
    pub installed_colliders: usize,
}

/// What the simulation still waits for before the player stands in the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlayerWait {
    /// The spawn is pending its ground raycast, which waits for the collision of
    /// `spawn_cell` to be installed.
    pub waiting_for_ground: bool,
    pub spawn_cell: (i32, i32),
}

/// One frame's readiness signals from the streamer and the simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Readiness {
    /// The streamer reported the full-detail ring complete on this frame.
    pub full_ring_complete: bool,
    pub colliders_pending: bool,
    pub player_ready: bool,
    pub player: PlayerWait,
    pub progress: ProgressCounts,
}

/// The step of the ready sequence a load is stuck on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// The streamer has not reported the full-detail ring complete.
    Stream,
    /// The full-detail ring is complete but the collider queue has not drained.
    Colliders,
    /// The fold hold is released but the player has not spawned.
    Player,
}

/// A load that made no progress for [`NO_PROGRESS_TIMEOUT_SECONDS`], reported once per stall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stall {
    pub gate: Gate,
    pub readiness: Readiness,
}

impl fmt::Display for Stall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let progress = &self.readiness.progress;
        match self.gate {
            Gate::Stream => write!(
                f,
                "the streamer has not completed the full-detail ring (pending_cells={} \
                 pending_lod_cells={} loaded_cells={} loaded_lod_cells={})",
                progress.stream.pending_cells,
                progress.stream.pending_lod_cells,
                progress.stream.loaded_cells,
                progress.stream.loaded_lod_cells,
            ),
            Gate::Colliders => write!(
                f,
                "the full-detail ring is complete but the collider queue has not drained \
                 (installed_colliders={})",
                progress.installed_colliders,
            ),
            Gate::Player => {
                let player = &self.readiness.player;
                write!(
                    f,
                    "the physics hold is released but the player has not spawned \
                     (waiting_for_ground={} spawn_cell=({}, {}) installed_colliders={})",
                    player.waiting_for_ground,
                    player.spawn_cell.0,
                    player.spawn_cell.1,
                    progress.installed_colliders,
                )
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GateStep {
    /// The physics fold hold is to be released now; true on exactly one frame of a load.
    pub release_fold_hold: bool,
    /// The world is ready to be revealed.
    pub reveal: bool,
    pub stalled: Option<Stall>,
}

#[derive(Debug, Clone, Default)]
pub struct LoadGate {
    full_ring_complete: bool,
    fold_hold_released: bool,
    consecutive_ready_frames: u32,
    since_progress: f32,
    last_progress: Option<ProgressCounts>,
    stall_reported: bool,
}

impl LoadGate {
    /// Advances the timers by one frame. The time without progress counts a long frame, such
    /// as one spent suspended, as one capped step.
    pub fn tick(&mut self, dt: f32) {
        self.since_progress += capped_step(dt);
    }

    pub fn update(&mut self, readiness: Readiness) -> GateStep {
        self.full_ring_complete |= readiness.full_ring_complete;
        let stalled = self.check_progress(readiness);

        if !self.fold_hold_released {
            let release = self.full_ring_complete && !readiness.colliders_pending;
            self.fold_hold_released = release;
            return GateStep {
                release_fold_hold: release,
                reveal: false,
                stalled,
            };
        }

        self.consecutive_ready_frames = if readiness.player_ready {
            self.consecutive_ready_frames.saturating_add(1)
        } else {
            0
        };
        GateStep {
            release_fold_hold: false,
            reveal: self.consecutive_ready_frames >= READY_FRAMES,
            stalled,
        }
    }

    fn check_progress(&mut self, readiness: Readiness) -> Option<Stall> {
        if self.last_progress != Some(readiness.progress) {
            self.last_progress = Some(readiness.progress);
            self.since_progress = 0.0;
            self.stall_reported = false;
            return None;
        }
        if self.stall_reported || self.since_progress < NO_PROGRESS_TIMEOUT_SECONDS {
            return None;
        }
        self.stall_reported = true;
        Some(Stall {
            gate: self.blocking_gate(),
            readiness,
        })
    }

    fn blocking_gate(&self) -> Gate {
        if !self.full_ring_complete {
            Gate::Stream
        } else if !self.fold_hold_released {
            Gate::Colliders
        } else {
            Gate::Player
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::MAX_FRAME_STEP_SECONDS;
    use super::*;

    const FRAME: f32 = 1.0 / 60.0;

    fn counts(pending_cells: usize, installed_colliders: usize) -> ProgressCounts {
        ProgressCounts {
            stream: StreamCounts {
                pending_cells,
                pending_lod_cells: 1,
                loaded_cells: 7,
                loaded_lod_cells: 2,
            },
            installed_colliders,
        }
    }

    fn streaming() -> Readiness {
        Readiness {
            progress: counts(3, 0),
            ..Readiness::default()
        }
    }

    fn streamed() -> Readiness {
        Readiness {
            full_ring_complete: true,
            ..Readiness::default()
        }
    }

    fn with_player(readiness: Readiness) -> Readiness {
        Readiness {
            player_ready: true,
            ..readiness
        }
    }

    /// Ticks the gate in frames of the capped step until `seconds` of stepped time passed.
    fn stepped(gate: &mut LoadGate, seconds: f32) {
        let frames = (seconds / MAX_FRAME_STEP_SECONDS).ceil() as u32;
        for _ in 0..frames {
            gate.tick(MAX_FRAME_STEP_SECONDS);
        }
    }

    /// Runs frames of `readiness` until a stall is reported; `None` if none is in `seconds`.
    fn stall_within(gate: &mut LoadGate, readiness: Readiness, seconds: f32) -> Option<Stall> {
        let frames = (seconds / MAX_FRAME_STEP_SECONDS).ceil() as u32;
        (0..frames).find_map(|_| {
            gate.tick(MAX_FRAME_STEP_SECONDS);
            gate.update(readiness).stalled
        })
    }

    #[test]
    fn nothing_is_released_before_the_full_ring_completes() {
        let mut gate = LoadGate::default();
        for _ in 0..5 {
            let step = gate.update(with_player(streaming()));
            assert_eq!(step, GateStep::default());
        }
    }

    #[test]
    fn a_stream_completing_with_colliders_pending_waits_for_them() {
        let mut gate = LoadGate::default();
        let pending = Readiness {
            colliders_pending: true,
            ..streamed()
        };
        assert!(!gate.update(pending).release_fold_hold);
        assert!(gate.update(Readiness::default()).release_fold_hold);
    }

    #[test]
    fn the_stream_completion_is_latched_when_it_is_reported_once() {
        let mut gate = LoadGate::default();
        let pending = Readiness {
            colliders_pending: true,
            ..streamed()
        };
        gate.update(pending);
        let later = Readiness {
            full_ring_complete: false,
            colliders_pending: false,
            ..Readiness::default()
        };
        assert!(gate.update(later).release_fold_hold);
    }

    #[test]
    fn the_fold_hold_is_released_exactly_once() {
        let mut gate = LoadGate::default();
        let releases = (0..10)
            .filter(|_| gate.update(streamed()).release_fold_hold)
            .count();
        assert_eq!(releases, 1);
    }

    #[test]
    fn the_release_frame_itself_does_not_count_as_a_ready_frame() {
        let mut gate = LoadGate::default();
        assert!(gate.update(with_player(streamed())).release_fold_hold);
        assert!(!gate.update(with_player(streamed())).reveal);
        assert!(gate.update(with_player(streamed())).reveal);
    }

    #[test]
    fn a_missing_player_frame_resets_the_consecutive_count() {
        let mut gate = LoadGate::default();
        gate.update(streamed());
        assert!(!gate.update(with_player(streamed())).reveal);
        assert!(!gate.update(streamed()).reveal);
        assert!(!gate.update(with_player(streamed())).reveal);
        assert!(gate.update(with_player(streamed())).reveal);
    }

    #[test]
    fn the_world_is_revealed_without_waiting_for_a_minimum_display_time() {
        let mut gate = LoadGate::default();
        gate.update(streamed());
        assert!(!gate.update(with_player(streamed())).reveal);
        assert!(gate.update(with_player(streamed())).reveal);
    }

    #[test]
    fn a_stalled_load_is_reported_once_with_the_readiness_it_stalled_on() {
        let mut gate = LoadGate::default();
        let mut reports = Vec::new();
        let frames = (NO_PROGRESS_TIMEOUT_SECONDS / MAX_FRAME_STEP_SECONDS) as u32 + 100;
        for _ in 0..frames {
            gate.tick(MAX_FRAME_STEP_SECONDS);
            reports.extend(gate.update(streaming()).stalled);
        }
        assert_eq!(
            reports,
            vec![Stall {
                gate: Gate::Stream,
                readiness: streaming(),
            }]
        );
    }

    #[test]
    fn changing_counts_keep_the_load_from_stalling() {
        let mut gate = LoadGate::default();
        for cells in (0..3000).rev() {
            gate.tick(MAX_FRAME_STEP_SECONDS * 4.0);
            let readiness = Readiness {
                progress: counts(cells, 0),
                ..Readiness::default()
            };
            assert_eq!(gate.update(readiness).stalled, None);
        }
    }

    #[test]
    fn installing_colliders_while_the_queue_drains_is_progress() {
        let mut gate = LoadGate::default();
        for installed in 0..3000 {
            gate.tick(MAX_FRAME_STEP_SECONDS);
            let draining = Readiness {
                full_ring_complete: true,
                colliders_pending: true,
                progress: counts(0, installed),
                ..Readiness::default()
            };
            assert_eq!(gate.update(draining).stalled, None);
        }
    }

    #[test]
    fn loading_more_cells_is_progress_even_when_the_pending_counts_stay_put() {
        let mut gate = LoadGate::default();
        for loaded in 0..3000 {
            gate.tick(MAX_FRAME_STEP_SECONDS);
            let mut readiness = streaming();
            readiness.progress.stream.loaded_cells = loaded;
            assert_eq!(gate.update(readiness).stalled, None);
        }
    }

    #[test]
    fn no_stall_is_reported_before_the_timeout() {
        let mut gate = LoadGate::default();
        gate.update(streaming());
        stepped(&mut gate, NO_PROGRESS_TIMEOUT_SECONDS - 1.0);
        assert_eq!(gate.update(streaming()).stalled, None);
        stepped(&mut gate, 1.1);
        assert!(gate.update(streaming()).stalled.is_some());
    }

    #[test]
    fn a_stall_that_resumes_and_stalls_again_is_reported_again() {
        let mut gate = LoadGate::default();
        gate.update(streaming());
        let first = stall_within(&mut gate, streaming(), NO_PROGRESS_TIMEOUT_SECONDS + 5.0);
        assert!(first.is_some());
        assert_eq!(
            stall_within(&mut gate, streaming(), NO_PROGRESS_TIMEOUT_SECONDS),
            None,
            "the same stall is reported once"
        );

        gate.tick(FRAME);
        let moved = Readiness {
            progress: counts(2, 0),
            ..Readiness::default()
        };
        assert_eq!(gate.update(moved).stalled, None);

        let second = stall_within(&mut gate, moved, NO_PROGRESS_TIMEOUT_SECONDS + 5.0);
        assert_eq!(second.map(|stall| stall.readiness), Some(moved));
    }

    #[test]
    fn a_long_frame_such_as_a_suspended_machine_counts_as_one_capped_step() {
        let mut gate = LoadGate::default();
        gate.update(streaming());
        gate.tick(3600.0);
        assert_eq!(gate.update(streaming()).stalled, None);
    }

    #[test]
    fn the_stall_names_the_stream_while_it_has_not_completed() {
        let mut gate = LoadGate::default();
        let stall = stall_within(&mut gate, streaming(), NO_PROGRESS_TIMEOUT_SECONDS + 5.0);
        assert_eq!(stall.map(|stall| stall.gate), Some(Gate::Stream));
    }

    #[test]
    fn the_stall_names_the_colliders_when_the_queue_never_drains() {
        let mut gate = LoadGate::default();
        let draining = Readiness {
            full_ring_complete: true,
            colliders_pending: true,
            ..Readiness::default()
        };
        let stall = stall_within(&mut gate, draining, NO_PROGRESS_TIMEOUT_SECONDS + 5.0);
        assert_eq!(stall.map(|stall| stall.gate), Some(Gate::Colliders));
    }

    #[test]
    fn the_stall_names_the_player_once_the_hold_is_released() {
        let mut gate = LoadGate::default();
        let waiting = Readiness {
            full_ring_complete: true,
            player: PlayerWait {
                waiting_for_ground: true,
                spawn_cell: (-3, 12),
            },
            ..Readiness::default()
        };
        let stall = stall_within(&mut gate, waiting, NO_PROGRESS_TIMEOUT_SECONDS + 5.0);
        assert_eq!(stall.map(|stall| stall.gate), Some(Gate::Player));
    }

    #[test]
    fn the_stream_stall_message_lists_the_cell_counts() {
        let message = Stall {
            gate: Gate::Stream,
            readiness: streaming(),
        }
        .to_string();
        assert_eq!(
            message,
            "the streamer has not completed the full-detail ring (pending_cells=3 \
             pending_lod_cells=1 loaded_cells=7 loaded_lod_cells=2)"
        );
    }

    #[test]
    fn the_collider_stall_message_lists_the_installed_colliders() {
        let message = Stall {
            gate: Gate::Colliders,
            readiness: Readiness {
                progress: counts(0, 42),
                ..Readiness::default()
            },
        }
        .to_string();
        assert_eq!(
            message,
            "the full-detail ring is complete but the collider queue has not drained \
             (installed_colliders=42)"
        );
    }

    #[test]
    fn the_player_stall_message_lists_the_spawn_cell_and_what_it_waits_for() {
        let message = Stall {
            gate: Gate::Player,
            readiness: Readiness {
                player: PlayerWait {
                    waiting_for_ground: true,
                    spawn_cell: (-3, 12),
                },
                progress: counts(0, 42),
                ..Readiness::default()
            },
        }
        .to_string();
        assert_eq!(
            message,
            "the physics hold is released but the player has not spawned \
             (waiting_for_ground=true spawn_cell=(-3, 12) installed_colliders=42)"
        );
    }

    #[test]
    fn a_full_load_reveals_after_release_two_ready_frames() {
        let mut gate = LoadGate::default();
        let mut revealed_at = None;
        for frame in 0..200 {
            gate.tick(FRAME);
            let readiness = if frame < 30 {
                streaming()
            } else {
                with_player(streamed())
            };
            if gate.update(readiness).reveal {
                revealed_at = Some(frame);
                break;
            }
        }
        let frame = revealed_at.expect("the load must reveal");
        assert!(
            frame >= 32,
            "released at 30, two ready frames after, got {frame}"
        );
    }
}
