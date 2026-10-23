//! A time-budgeted FIFO for the collider commands that would otherwise land in
//! one frame: cell adds, each a burst of broad-phase inserts, and the physics
//! centre moves that must stay behind the adds they follow. The queue holds
//! plain data and is generic over the add payload, so its ordering, budget
//! and cancellation rules run without a physics world.

use std::collections::VecDeque;
use std::time::Duration;

use super::CellGrid;

pub(super) enum Deferred<A> {
    AddCell { grid: CellGrid, payload: A },
    PhysicsCenter { grid: CellGrid, radius: i32 },
}

pub(super) struct DeferredQueue<A> {
    entries: VecDeque<Deferred<A>>,
}

impl<A> DeferredQueue<A> {
    pub(super) fn new() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn push_add(&mut self, grid: CellGrid, payload: A) {
        self.entries.push_back(Deferred::AddCell { grid, payload });
    }

    /// A centre move supersedes the one queued right before it: nothing sits
    /// between them to observe the older value.
    pub(super) fn push_physics_center(&mut self, grid: CellGrid, radius: i32) {
        if matches!(self.entries.back(), Some(Deferred::PhysicsCenter { .. })) {
            self.entries.pop_back();
        }
        self.entries
            .push_back(Deferred::PhysicsCenter { grid, radius });
    }

    /// Drops the queued adds of `grid`.
    pub(super) fn cancel_add(&mut self, grid: CellGrid) {
        self.entries.retain(|entry| match entry {
            Deferred::AddCell { grid: queued, .. } => *queued != grid,
            Deferred::PhysicsCenter { .. } => true,
        });
    }

    /// Drops every queued add and centre move.
    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    /// Hands entries to `apply` in FIFO order until `elapsed` reports that the
    /// `budget` is spent, always handing over at least one entry so a single
    /// expensive add cannot starve the queue. Returns how many were applied.
    pub(super) fn drain_within(
        &mut self,
        budget: Duration,
        mut elapsed: impl FnMut() -> Duration,
        mut apply: impl FnMut(Deferred<A>),
    ) -> usize {
        let mut applied = 0;
        loop {
            if applied >= 1 && elapsed() >= budget {
                break;
            }
            let Some(entry) = self.entries.pop_front() else {
                break;
            };
            apply(entry);
            applied += 1;
        }
        applied
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    const BUDGET: Duration = Duration::from_millis(6);
    const ONE_ADD: Duration = Duration::from_millis(4);

    #[derive(Debug, PartialEq)]
    enum Applied {
        Add(CellGrid, &'static str),
        Center(CellGrid, i32),
    }

    fn queue_of(adds: &[(CellGrid, &'static str)]) -> DeferredQueue<&'static str> {
        let mut queue = DeferredQueue::new();
        for &(grid, payload) in adds {
            queue.push_add(grid, payload);
        }
        queue
    }

    /// Drains with a fake clock that advances by `cost` per applied entry.
    fn drain(
        queue: &mut DeferredQueue<&'static str>,
        budget: Duration,
        cost: Duration,
    ) -> Vec<Applied> {
        let clock = Cell::new(Duration::ZERO);
        let mut applied = Vec::new();
        queue.drain_within(
            budget,
            || clock.get(),
            |entry| {
                clock.set(clock.get() + cost);
                applied.push(match entry {
                    Deferred::AddCell { grid, payload } => Applied::Add(grid, payload),
                    Deferred::PhysicsCenter { grid, radius } => Applied::Center(grid, radius),
                });
            },
        );
        applied
    }

    #[test]
    fn entries_are_applied_in_the_order_they_were_queued() {
        let mut queue = queue_of(&[((0, 0), "a"), ((1, 0), "b")]);
        queue.push_physics_center((0, 0), 2);
        queue.push_add((2, 0), "c");
        let applied = drain(&mut queue, Duration::MAX, ONE_ADD);
        assert_eq!(
            applied,
            vec![
                Applied::Add((0, 0), "a"),
                Applied::Add((1, 0), "b"),
                Applied::Center((0, 0), 2),
                Applied::Add((2, 0), "c"),
            ]
        );
        assert!(queue.is_empty());
    }

    #[test]
    fn a_drain_stops_once_the_budget_is_spent_and_the_rest_waits_for_the_next_call() {
        let mut queue = queue_of(&[((0, 0), "a"), ((1, 0), "b"), ((2, 0), "c")]);
        let first = drain(&mut queue, BUDGET, ONE_ADD);
        assert_eq!(
            first,
            vec![Applied::Add((0, 0), "a"), Applied::Add((1, 0), "b")]
        );
        assert_eq!(queue.len(), 1);
        let second = drain(&mut queue, BUDGET, ONE_ADD);
        assert_eq!(second, vec![Applied::Add((2, 0), "c")]);
        assert!(queue.is_empty());
    }

    #[test]
    fn a_drain_applies_one_entry_even_when_it_alone_exceeds_the_budget() {
        let mut queue = queue_of(&[((0, 0), "a"), ((1, 0), "b")]);
        let applied = drain(&mut queue, BUDGET, Duration::from_secs(1));
        assert_eq!(applied, vec![Applied::Add((0, 0), "a")]);
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn a_zero_budget_still_makes_progress_one_entry_per_call() {
        let mut queue = queue_of(&[((0, 0), "a"), ((1, 0), "b")]);
        assert_eq!(drain(&mut queue, Duration::ZERO, ONE_ADD).len(), 1);
        assert_eq!(drain(&mut queue, Duration::ZERO, ONE_ADD).len(), 1);
        assert!(queue.is_empty());
    }

    #[test]
    fn draining_an_empty_queue_applies_nothing() {
        let mut queue = DeferredQueue::<&'static str>::new();
        assert!(drain(&mut queue, BUDGET, ONE_ADD).is_empty());
    }

    #[test]
    fn cancelling_a_queued_add_removes_only_that_cell() {
        let mut queue = queue_of(&[((0, 0), "a"), ((1, 0), "b"), ((2, 0), "c")]);
        queue.cancel_add((1, 0));
        let applied = drain(&mut queue, Duration::MAX, ONE_ADD);
        assert_eq!(
            applied,
            vec![Applied::Add((0, 0), "a"), Applied::Add((2, 0), "c")]
        );
    }

    #[test]
    fn cancelling_a_cell_that_is_not_queued_changes_nothing() {
        let mut queue = queue_of(&[((0, 0), "a")]);
        queue.cancel_add((5, 5));
        assert_eq!(
            drain(&mut queue, Duration::MAX, ONE_ADD),
            vec![Applied::Add((0, 0), "a")]
        );
    }

    #[test]
    fn cancelling_keeps_the_centre_moves_in_place() {
        let mut queue = queue_of(&[((0, 0), "a")]);
        queue.push_physics_center((0, 0), 2);
        queue.cancel_add((0, 0));
        let applied = drain(&mut queue, Duration::MAX, ONE_ADD);
        assert_eq!(applied, vec![Applied::Center((0, 0), 2)]);
    }

    #[test]
    fn clearing_drops_adds_and_centre_moves_alike() {
        let mut queue = queue_of(&[((0, 0), "a"), ((1, 0), "b")]);
        queue.push_physics_center((1, 0), 2);
        queue.clear();
        assert!(queue.is_empty());
        assert!(drain(&mut queue, Duration::MAX, ONE_ADD).is_empty());
    }

    #[test]
    fn a_centre_move_replaces_the_one_queued_right_before_it() {
        let mut queue = queue_of(&[((0, 0), "a")]);
        queue.push_physics_center((0, 0), 2);
        queue.push_physics_center((1, 0), 3);
        let applied = drain(&mut queue, Duration::MAX, ONE_ADD);
        assert_eq!(
            applied,
            vec![Applied::Add((0, 0), "a"), Applied::Center((1, 0), 3)]
        );
    }

    #[test]
    fn a_centre_move_does_not_replace_one_that_an_add_separates_from_it() {
        let mut queue = DeferredQueue::new();
        queue.push_physics_center((0, 0), 2);
        queue.push_add((1, 0), "a");
        queue.push_physics_center((1, 0), 2);
        assert_eq!(queue.len(), 3);
    }
}
