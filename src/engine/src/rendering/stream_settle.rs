//! Applies a frame's streamer output to the simulation and reports what that left ready.
//!
//! The load gate must only see the colliders as settled after the frame's commands were applied
//! and the budgeted drain ran, otherwise a frame that completes the full-detail ring while colliders are
//! still queued could release the physics fold hold early. [`readiness`] therefore takes the
//! [`ColliderSettle`] that only [`settle_colliders`] produces.

use std::time::Duration;

use crate::game_flow::{PlayerWait, ProgressCounts, Readiness, StreamCounts};
use crate::sim::{SimCommand, SimWorld};

/// Proof that a frame's collider commands were applied and the queue drained within budget.
#[derive(Debug)]
pub(super) struct ColliderSettle(());

/// Applies the streamer's collider commands, drains the queue for at most `budget` (one entry at
/// least) and folds the broad phase the way a streaming frame does.
pub(super) fn settle_colliders(
    sim: &mut SimWorld,
    commands: Vec<SimCommand>,
    budget: Duration,
) -> ColliderSettle {
    if commands.is_empty() && !sim.has_pending_colliders() {
        let _span = tracing::info_span!("flush_colliders").entered();
        sim.flush_colliders_per_frame();
    } else {
        let _span = tracing::info_span!("sim_fold", commands = commands.len()).entered();
        for command in commands {
            sim.apply(command);
        }
        sim.drain_pending_colliders(budget);
        sim.flush_colliders_per_frame();
    }
    ColliderSettle(())
}

/// Applies the commands of a streamer teardown (worldspace switch or interior enter) and folds
/// the broad phase. Collider work still queued belongs to the world being left: the teardown's
/// removes already cover every registered cell, so the queue is dropped instead of applied.
pub(super) fn apply_teardown(sim: &mut SimWorld, commands: Vec<SimCommand>) {
    for command in commands {
        sim.apply(command);
    }
    sim.discard_pending_colliders();
    sim.flush_colliders();
}

/// What the streamer says about the load after this frame.
#[derive(Debug, Clone, Copy)]
pub(super) struct StreamReport {
    /// The streamer reported the full-detail ring complete on this frame.
    pub full_ring_complete: bool,
    pub counts: StreamCounts,
    /// The grid cell the player spawns in, whose collision the spawn waits for.
    pub spawn_cell: (i32, i32),
}

/// What the streamer and the simulation have ready after this frame's colliders settled.
pub(super) fn readiness(
    _settled: ColliderSettle,
    sim: &SimWorld,
    stream: StreamReport,
) -> Readiness {
    Readiness {
        full_ring_complete: stream.full_ring_complete,
        colliders_pending: sim.has_pending_colliders(),
        player_ready: sim.player_eye().is_some(),
        player: PlayerWait {
            waiting_for_ground: sim.player_debug().waiting_for_ground,
            spawn_cell: stream.spawn_cell,
        },
        progress: ProgressCounts {
            stream: stream.counts,
            installed_colliders: sim.stats().collider_count,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::sim::collision::{CellColliderBatch, TerrainColliderSpec, TerrainCollision};
    use crate::sim::{FIXED_DT, PlayerMovement, WorldClock};
    use glam::Vec3;

    const SPAWN_CELL: (i32, i32) = (-3, 12);

    fn held_sim() -> SimWorld {
        let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
        sim.set_fold_hold(true);
        sim
    }

    fn add_cell(grid: (i32, i32)) -> SimCommand {
        SimCommand::AddCellColliders {
            grid,
            batch: Arc::new(CellColliderBatch::default()),
            baked: None,
        }
    }

    /// A cell with one terrain quad, so installing it adds a collider.
    fn ground_cell(grid: (i32, i32)) -> SimCommand {
        let terrain = TerrainColliderSpec {
            vertices: vec![
                Vec3::ZERO,
                Vec3::new(10.0, 0.0, 0.0),
                Vec3::new(10.0, 0.0, 10.0),
                Vec3::new(0.0, 0.0, 10.0),
            ],
            indices: vec![[0, 1, 2], [0, 2, 3]],
        };
        SimCommand::AddCellColliders {
            grid,
            batch: Arc::new(CellColliderBatch {
                terrain: TerrainCollision::Present(terrain),
                objects: Vec::new(),
            }),
            baked: None,
        }
    }

    fn readiness_after(
        sim: &mut SimWorld,
        commands: Vec<SimCommand>,
        full_ring_complete: bool,
        budget: Duration,
    ) -> Readiness {
        let settled = settle_colliders(sim, commands, budget);
        readiness(
            settled,
            sim,
            StreamReport {
                full_ring_complete,
                counts: StreamCounts::default(),
                spawn_cell: SPAWN_CELL,
            },
        )
    }

    #[test]
    fn the_readiness_reports_the_spawn_cell_and_whether_the_player_waits_for_ground() {
        let mut sim = held_sim();
        let before = readiness_after(&mut sim, Vec::new(), false, Duration::ZERO);
        assert!(!before.player.waiting_for_ground);
        assert_eq!(before.player.spawn_cell, SPAWN_CELL);

        sim.apply(SimCommand::SpawnPlayer { pos: Vec3::ZERO });
        let after = readiness_after(&mut sim, Vec::new(), false, Duration::ZERO);

        assert!(after.player.waiting_for_ground);
    }

    #[test]
    fn a_spawn_into_a_cell_without_terrain_is_ready_once_its_collision_is_installed() {
        let mut sim = held_sim();
        sim.apply(SimCommand::SpawnPlayer { pos: Vec3::ZERO });
        readiness_after(&mut sim, vec![add_cell((0, 0))], false, Duration::MAX);
        sim.set_fold_hold(false);
        sim.tick(FIXED_DT);
        sim.tick(FIXED_DT);

        let after = readiness_after(&mut sim, Vec::new(), false, Duration::ZERO);

        assert!(after.player_ready);
        assert!(!after.player.waiting_for_ground);
    }

    #[test]
    fn the_progress_follows_the_stream_counts_and_the_installed_colliders() {
        let mut sim = held_sim();
        let counts = StreamCounts {
            pending_cells: 4,
            pending_lod_cells: 3,
            loaded_cells: 2,
            loaded_lod_cells: 1,
        };
        let report = |full_ring_complete| StreamReport {
            full_ring_complete,
            counts,
            spawn_cell: SPAWN_CELL,
        };
        let settled = settle_colliders(&mut sim, Vec::new(), Duration::ZERO);
        let empty = readiness(settled, &sim, report(false));
        assert_eq!(empty.progress.stream, counts);

        let settled = settle_colliders(&mut sim, vec![ground_cell((0, 0))], Duration::MAX);
        let filled = readiness(settled, &sim, report(false));

        assert!(filled.progress.installed_colliders > empty.progress.installed_colliders);
    }

    #[test]
    fn a_frame_that_completes_the_stream_reports_the_colliders_it_left_queued() {
        let mut sim = held_sim();
        let commands = vec![add_cell((0, 0)), add_cell((1, 0)), add_cell((2, 0))];

        let frame = readiness_after(&mut sim, commands, true, Duration::ZERO);

        assert!(frame.full_ring_complete);
        assert!(
            frame.colliders_pending,
            "a zero budget drains one entry, two remain queued"
        );
    }

    #[test]
    fn the_following_frames_drain_the_rest_and_report_it_settled() {
        let mut sim = held_sim();
        let commands = vec![add_cell((0, 0)), add_cell((1, 0)), add_cell((2, 0))];
        readiness_after(&mut sim, commands, true, Duration::ZERO);

        let second = readiness_after(&mut sim, Vec::new(), false, Duration::ZERO);
        assert!(second.colliders_pending);
        let third = readiness_after(&mut sim, Vec::new(), false, Duration::ZERO);
        assert!(!third.colliders_pending);
    }

    #[test]
    fn a_frame_without_commands_and_without_a_queue_is_settled() {
        let mut sim = held_sim();
        let frame = readiness_after(&mut sim, Vec::new(), false, Duration::ZERO);
        assert!(!frame.colliders_pending);
        assert!(!frame.full_ring_complete);
        assert!(!frame.player_ready);
    }

    #[test]
    fn a_teardown_drops_the_collider_work_queued_for_the_world_being_left() {
        let mut sim = held_sim();
        settle_colliders(
            &mut sim,
            vec![add_cell((0, 0)), add_cell((1, 0))],
            Duration::ZERO,
        );
        assert!(sim.has_pending_colliders());

        apply_teardown(&mut sim, Vec::new());

        assert!(!sim.has_pending_colliders());
        assert_eq!(
            sim.stats().cells_with_colliders,
            1,
            "only the entry drained before the teardown stays installed"
        );
    }

    #[test]
    fn a_teardown_applies_its_commands_before_it_discards() {
        let mut sim = held_sim();
        settle_colliders(
            &mut sim,
            vec![add_cell((0, 0)), add_cell((1, 0))],
            Duration::ZERO,
        );

        apply_teardown(
            &mut sim,
            vec![SimCommand::RemoveCellColliders { grid: (0, 0) }],
        );

        assert_eq!(sim.stats().cells_with_colliders, 0);
        assert!(!sim.has_pending_colliders());
    }

    #[test]
    fn commands_are_applied_before_the_queue_is_inspected() {
        let mut sim = held_sim();
        let frame = readiness_after(&mut sim, vec![add_cell((0, 0))], true, Duration::MAX);
        assert!(
            !frame.colliders_pending,
            "the add of this very frame was applied and drained before readiness was read"
        );
    }
}
