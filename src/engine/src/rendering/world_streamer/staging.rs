//! Cells that are integrated but not drawn yet. A cell stays staged until every texture it
//! uses is loaded and bound, so a mesh never shows a stand-in while its texture streams in;
//! the twin it replaces keeps drawing meanwhile. A staged full cell's static collision is
//! registered already and its grass is held hidden; its dynamic objects spawn when it is
//! promoted.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::{CellResources, GpuPlacedLight, LightModelRef, NavMesh, ShadowLampSource};
use crate::rendering::animated_objects::AnimatedRefr;
use crate::rendering::ragdoll_objects::RagdollRefr;
use crate::rendering::texture_streaming::residency::TextureLoad;

/// How long a staged cell may wait for its textures before the load counts as hung.
pub(super) const TEXTURE_READY_DEADLINE: Duration = Duration::from_secs(30);

/// The longest step one update adds to a staged cell's wait, so a stall of the whole process
/// (a debugger, a suspended window) is not taken for a hung texture.
const MAX_WAIT_STEP: Duration = Duration::from_millis(250);

/// How far one texture a staged cell uses has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TextureReadiness {
    /// Its file is still being read, or it was found broken and the game is ending.
    Loading,
    /// Loaded, or settled on its stand-in because its file is missing, but the material bind
    /// group still samples the view the slot held before.
    Unbound,
    Ready,
}

impl TextureReadiness {
    pub(super) fn of(load: Option<TextureLoad>, view_bound: bool) -> Self {
        match load {
            Some(TextureLoad::Resident | TextureLoad::Missing) if view_bound => Self::Ready,
            Some(TextureLoad::Resident | TextureLoad::Missing) => Self::Unbound,
            Some(TextureLoad::Pending | TextureLoad::Broken) | None => Self::Loading,
        }
    }
}

/// What to do with a staged cell this update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Verdict {
    Promote,
    /// Some texture is not ready yet. `only_unbound`: every one that is not is merely waiting
    /// for the next bind group rebuild.
    Wait {
        only_unbound: bool,
    },
    /// Some texture is still not ready after `TEXTURE_READY_DEADLINE`.
    Overdue,
}

pub(super) fn verdict(
    textures: impl IntoIterator<Item = TextureReadiness>,
    waited: Duration,
) -> Verdict {
    let mut ready = true;
    let mut only_unbound = true;
    for texture in textures {
        match texture {
            TextureReadiness::Ready => {}
            TextureReadiness::Unbound => ready = false,
            TextureReadiness::Loading => {
                ready = false;
                only_unbound = false;
            }
        }
    }
    if ready {
        Verdict::Promote
    } else if waited >= TEXTURE_READY_DEADLINE {
        Verdict::Overdue
    } else {
        Verdict::Wait { only_unbound }
    }
}

/// What a full cell hands over only once it is drawn: what lights, simulates or is walked on by
/// the AI follows what the player sees.
#[derive(Default)]
pub(super) struct DeferredSpawn {
    pub(super) mesh_groups: usize,
    pub(super) lights: Vec<GpuPlacedLight>,
    pub(super) shadow_lamps: Vec<ShadowLampSource>,
    pub(super) light_models: Vec<LightModelRef>,
    pub(super) addon_lights: u32,
    pub(super) navmeshes: Vec<NavMesh>,
    pub(super) animated_refs: Vec<AnimatedRefr>,
    pub(super) ragdoll_refs: Vec<RagdollRefr>,
}

pub(super) struct StagedCell {
    pub(super) res: CellResources,
    pub(super) spawn: DeferredSpawn,
    waited: Duration,
    /// Staging order; ready cells are promoted oldest first.
    sequence: u64,
}

impl StagedCell {
    pub(super) fn waited(&self) -> Duration {
        self.waited
    }
}

/// Which ring a staged cell belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Ring {
    Full,
    Lod,
}

#[derive(Default)]
pub(super) struct Staging {
    full: HashMap<(i32, i32), StagedCell>,
    lod: HashMap<(i32, i32), StagedCell>,
    next_sequence: u64,
    last_update: Option<Instant>,
}

impl Staging {
    pub(super) fn cells(&self, ring: Ring) -> &HashMap<(i32, i32), StagedCell> {
        match ring {
            Ring::Full => &self.full,
            Ring::Lod => &self.lod,
        }
    }

    fn cells_mut(&mut self, ring: Ring) -> &mut HashMap<(i32, i32), StagedCell> {
        match ring {
            Ring::Full => &mut self.full,
            Ring::Lod => &mut self.lod,
        }
    }

    pub(super) fn get(&self, ring: Ring, grid: (i32, i32)) -> Option<&StagedCell> {
        self.cells(ring).get(&grid)
    }

    pub(super) fn holds(&self, ring: Ring, grid: (i32, i32)) -> bool {
        self.cells(ring).contains_key(&grid)
    }

    pub(super) fn len(&self, ring: Ring) -> usize {
        self.cells(ring).len()
    }

    pub(super) fn stage(
        &mut self,
        ring: Ring,
        grid: (i32, i32),
        res: CellResources,
        spawn: DeferredSpawn,
    ) {
        self.next_sequence += 1;
        let cell = StagedCell {
            res,
            spawn,
            waited: Duration::ZERO,
            sequence: self.next_sequence,
        };
        let replaced = self.cells_mut(ring).insert(grid, cell);
        debug_assert!(replaced.is_none(), "a grid is staged at most once per ring");
    }

    pub(super) fn take(&mut self, ring: Ring, grid: (i32, i32)) -> Option<StagedCell> {
        self.cells_mut(ring).remove(&grid)
    }

    /// Removes and returns the cells of `ring` that `keep` rejects.
    pub(super) fn drain_unwanted(
        &mut self,
        ring: Ring,
        keep: impl Fn((i32, i32)) -> bool,
    ) -> Vec<CellResources> {
        let cells = self.cells_mut(ring);
        let unwanted: Vec<(i32, i32)> = cells.keys().copied().filter(|g| !keep(*g)).collect();
        unwanted
            .into_iter()
            .filter_map(|grid| cells.remove(&grid))
            .map(|cell| cell.res)
            .collect()
    }

    /// Every staged cell's resources, for the bookkeeping that must cover hidden cells too.
    pub(super) fn resources_mut(&mut self) -> impl Iterator<Item = &mut CellResources> {
        self.full
            .values_mut()
            .chain(self.lod.values_mut())
            .map(|cell| &mut cell.res)
    }

    /// Removes every staged cell.
    pub(super) fn drain_all(&mut self) -> Vec<CellResources> {
        self.full
            .drain()
            .chain(self.lod.drain())
            .map(|(_, cell)| cell.res)
            .collect()
    }

    /// Adds the time since the previous update to every staged cell's wait.
    pub(super) fn advance(&mut self, now: Instant) {
        let step = self
            .last_update
            .map_or(Duration::ZERO, |last| now.saturating_duration_since(last))
            .min(MAX_WAIT_STEP);
        self.last_update = Some(now);
        for cell in self.full.values_mut().chain(self.lod.values_mut()) {
            cell.waited += step;
        }
    }

    /// Forgets the previous update, for frames on which the streamer does not run (menus).
    pub(super) fn suspend(&mut self) {
        self.last_update = None;
    }

    /// The verdict on every staged cell, with the ready ones oldest first.
    pub(super) fn judge(
        &self,
        readiness: impl Fn(&CellResources) -> Vec<TextureReadiness>,
    ) -> Judgement {
        let mut ready: Vec<(u64, Ring, (i32, i32))> = Vec::new();
        let mut judgement = Judgement::default();
        for ring in [Ring::Full, Ring::Lod] {
            for (&grid, cell) in self.cells(ring) {
                match verdict(readiness(&cell.res), cell.waited) {
                    Verdict::Promote => ready.push((cell.sequence, ring, grid)),
                    Verdict::Wait { only_unbound } => {
                        judgement.waiting_only_for_binding |= only_unbound;
                    }
                    Verdict::Overdue => judgement.overdue.push((ring, grid)),
                }
            }
        }
        ready.sort_unstable_by_key(|&(sequence, ..)| sequence);
        judgement.ready = ready
            .into_iter()
            .map(|(_, ring, grid)| (ring, grid))
            .collect();
        judgement
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Judgement {
    /// Cells to promote, oldest first.
    pub(super) ready: Vec<(Ring, (i32, i32))>,
    pub(super) overdue: Vec<(Ring, (i32, i32))>,
    /// Some cell waits for nothing but the next bind group rebuild.
    pub(super) waiting_only_for_binding: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use TextureReadiness::{Loading, Ready, Unbound};

    #[test]
    fn a_texture_is_ready_once_loaded_or_settled_on_its_stand_in_and_bound() {
        for load in [TextureLoad::Resident, TextureLoad::Missing] {
            assert_eq!(TextureReadiness::of(Some(load), true), Ready);
            assert_eq!(TextureReadiness::of(Some(load), false), Unbound);
        }
    }

    #[test]
    fn a_pending_broken_or_untracked_texture_is_still_loading() {
        for load in [Some(TextureLoad::Pending), Some(TextureLoad::Broken), None] {
            assert_eq!(TextureReadiness::of(load, true), Loading);
        }
    }

    #[test]
    fn a_cell_without_textures_is_promoted_at_once() {
        assert_eq!(verdict([], Duration::ZERO), Verdict::Promote);
    }

    #[test]
    fn a_cell_is_promoted_only_when_every_texture_is_ready() {
        assert_eq!(verdict([Ready, Ready], Duration::ZERO), Verdict::Promote);
        assert_eq!(
            verdict([Ready, Loading], Duration::ZERO),
            Verdict::Wait {
                only_unbound: false
            }
        );
    }

    #[test]
    fn a_cell_waiting_only_for_the_bind_group_says_so() {
        assert_eq!(
            verdict([Ready, Unbound], Duration::ZERO),
            Verdict::Wait { only_unbound: true }
        );
        assert_eq!(
            verdict([Unbound, Loading], Duration::ZERO),
            Verdict::Wait {
                only_unbound: false
            }
        );
    }

    #[test]
    fn a_cell_still_waiting_at_the_deadline_is_overdue() {
        let just_before = TEXTURE_READY_DEADLINE.saturating_sub(Duration::from_millis(1));
        assert_eq!(
            verdict([Loading], just_before),
            Verdict::Wait {
                only_unbound: false
            }
        );
        assert_eq!(verdict([Loading], TEXTURE_READY_DEADLINE), Verdict::Overdue);
        assert_eq!(
            verdict([Ready], TEXTURE_READY_DEADLINE),
            Verdict::Promote,
            "a cell that became ready in time is not overdue"
        );
    }

    fn staged(staging: &mut Staging, ring: Ring, grid: (i32, i32), marker_count: u32) {
        let res = CellResources {
            marker_count,
            ..CellResources::default()
        };
        staging.stage(ring, grid, res, DeferredSpawn::default());
    }

    /// Readiness keyed on a marker count, which stands in for the cell's textures.
    fn by_marker(res: &CellResources) -> Vec<TextureReadiness> {
        match res.marker_count {
            0 => vec![Ready],
            1 => vec![Unbound],
            _ => vec![Loading],
        }
    }

    #[test]
    fn ready_cells_of_both_rings_are_promoted_oldest_first() {
        let mut staging = Staging::default();
        staged(&mut staging, Ring::Lod, (5, 5), 0);
        staged(&mut staging, Ring::Full, (0, 0), 2);
        staged(&mut staging, Ring::Full, (1, 0), 0);
        staged(&mut staging, Ring::Lod, (6, 5), 0);

        let judgement = staging.judge(by_marker);

        assert_eq!(
            judgement.ready,
            vec![
                (Ring::Lod, (5, 5)),
                (Ring::Full, (1, 0)),
                (Ring::Lod, (6, 5))
            ]
        );
        assert_eq!(judgement.overdue, Vec::new());
        assert!(!judgement.waiting_only_for_binding);
    }

    #[test]
    fn a_cell_waiting_for_the_bind_group_is_reported() {
        let mut staging = Staging::default();
        staged(&mut staging, Ring::Full, (0, 0), 1);

        assert!(staging.judge(by_marker).waiting_only_for_binding);
    }

    #[test]
    fn the_wait_grows_by_capped_steps_and_a_suspension_adds_nothing() {
        let mut staging = Staging::default();
        let start = Instant::now();
        staging.advance(start);
        staged(&mut staging, Ring::Full, (0, 0), 2);

        staging.advance(start + Duration::from_millis(100));
        staging.advance(start + Duration::from_secs(60));
        staging.suspend();
        staging.advance(start + Duration::from_secs(600));

        let waited = staging.get(Ring::Full, (0, 0)).map(StagedCell::waited);
        assert_eq!(
            waited,
            Some(Duration::from_millis(100) + MAX_WAIT_STEP),
            "a long gap counts one capped step, the first update after a suspension none"
        );
    }

    #[test]
    fn a_cell_that_keeps_loading_becomes_overdue() {
        let mut staging = Staging::default();
        let mut now = Instant::now();
        staging.advance(now);
        staged(&mut staging, Ring::Lod, (3, 4), 2);
        staged(&mut staging, Ring::Lod, (3, 5), 0);
        while staging.get(Ring::Lod, (3, 4)).map(StagedCell::waited) < Some(TEXTURE_READY_DEADLINE)
        {
            now += MAX_WAIT_STEP;
            staging.advance(now);
        }

        let judgement = staging.judge(by_marker);

        assert_eq!(judgement.overdue, vec![(Ring::Lod, (3, 4))]);
        assert_eq!(judgement.ready, vec![(Ring::Lod, (3, 5))]);
    }

    #[test]
    fn unwanted_cells_leave_the_staging_and_the_rest_stay() {
        let mut staging = Staging::default();
        staged(&mut staging, Ring::Full, (0, 0), 0);
        staged(&mut staging, Ring::Full, (9, 0), 0);
        staged(&mut staging, Ring::Lod, (9, 0), 0);

        let dropped = staging.drain_unwanted(Ring::Full, |grid| grid.0 < 5);

        assert_eq!(dropped.len(), 1);
        assert!(staging.holds(Ring::Full, (0, 0)));
        assert!(!staging.holds(Ring::Full, (9, 0)));
        assert!(
            staging.holds(Ring::Lod, (9, 0)),
            "the other ring is untouched"
        );
        assert_eq!(staging.drain_all().len(), 2);
        assert_eq!(staging.len(Ring::Full) + staging.len(Ring::Lod), 0);
    }
}
