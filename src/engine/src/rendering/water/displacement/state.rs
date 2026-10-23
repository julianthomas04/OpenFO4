//! The CPU side of Fallout 4's water displacement: the per-frame driver
//! (quad re-snap, splat collection, wading actor map, bullet splats), reduced to
//! a pure state machine that turns one frame of inputs into the list of passes
//! to run.
//!
//! All positions are Bethesda game units; segments are in the NDC of the
//! 2048-unit wading quad.

use std::collections::BTreeMap;

use glam::Vec2;

/// `fWadingWaterQuadSize`.
pub const QUAD_SIZE: f32 = 2048.0;
/// Game units covered by one NDC unit (the quad spans `[-1, 1]`).
const NDC_UNIT: f32 = QUAD_SIZE * 0.5;
/// Whether an impact at `position` is close enough to `player` to splat
/// (within `QUAD_SIZE`).
pub fn impact_in_range(position: Vec2, player: Vec2) -> bool {
    (position - player).length_squared() <= QUAD_SIZE * QUAD_SIZE
}

/// The quad re-snaps when the player is further than this from its centre on
/// either axis.
const RESNAP_DISTANCE: f32 = 4.0;
/// The quad centre lies on this grid (`floor(p * 0.25) * 4`).
const SNAP_GRID: f32 = 4.0;
/// Fixed simulation step.
pub const STEP_SECONDS: f32 = 0.025;
/// Seconds without a splat after which the simulation stops and the field is
/// cleared on its next use.
pub const IDLE_LIMIT_SECONDS: f32 = 10.0;
/// The wave step constants: stiffness, velocity scale, damping, and an unused
/// fourth lane.
pub const WADING_VARS: [f32; 4] = [0.2, 1.7, 0.99, 0.01];
/// `bUseBulletWaterDisplacements`.
const BULLET_DISPLACEMENTS_ENABLED: bool = true;
/// The size every non-actor producer gives its impact splat.
pub const IMPACT_SIZE: f32 = 0.01;
/// Wave steps per frame are bounded so a long hitch cannot stall the GPU.
pub const MAX_STEPS_PER_FRAME: u32 = 64;
/// Splat quads per frame.
/// Splats beyond it are dropped in a defined order: impacts first in the order
/// they arrived, then wading actors by ascending handle, so the survivors do
/// not depend on hashing.
pub const MAX_SPLATS_PER_FRAME: usize = 512;

/// One splat segment in quad NDC: the quad runs from `previous` to `current`
/// and is 0.01 wide on the left of that direction.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Splat {
    pub current: Vec2,
    pub previous: Vec2,
}

impl Splat {
    /// The two rows of the splat vertex shader's affine transform (xyz of
    /// `cb2[1]` and `cb2[2]`): `(d.x, -n.y, B.x)` and `(d.y, n.x, B.y)` with
    /// `d = A - B` and `n = normalize(d)`, zero for a segment of no length.
    pub fn rows(&self) -> ([f32; 3], [f32; 3]) {
        let delta = self.current - self.previous;
        let length = delta.length();
        let normal = if length > 1.0e-6 {
            delta / length
        } else {
            Vec2::ZERO
        };
        (
            [delta.x, -normal.y, self.previous.x],
            [delta.y, normal.x, self.previous.y],
        )
    }
}

/// What the render passes of one frame do, in order.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Pass {
    /// Clear one target to the neutral 0.5.
    Clear { target: usize },
    /// Copy `source` into `destination` shifted by the frame's texture offset
    /// and border-faded, then draw this frame's splats onto it.
    Advect { source: usize, destination: usize },
    /// One wave step.
    Step { source: usize, destination: usize },
}

/// Everything the GPU has to do this frame.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FramePlan {
    pub passes: Vec<Pass>,
    /// The texture offset for the advect pass.
    pub texture_offset: Vec2,
    pub splats: Vec<Splat>,
    pub steps: u32,
    /// The simulation ran this frame (the field is live and may be visible).
    pub active: bool,
    /// The field was cleared this frame.
    pub cleared: bool,
}

/// An actor's last segment, already in quad NDC.
#[derive(Debug, Copy, Clone)]
struct ActorEntry {
    segment: Splat,
    /// The actor position saved as the next frame's previous position.
    last_position: Vec2,
}

/// The wading state: quad placement, the wading actor map, the splat queue,
/// the idle timer and the ping-pong roles.
#[derive(Debug, Clone)]
pub struct DisplacementState {
    centre: Vec2,
    /// Time accumulated towards the next fixed step.
    pending_seconds: f32,
    idle_seconds: f32,
    needs_clear: bool,
    actors: BTreeMap<u64, ActorEntry>,
    impacts: Vec<Splat>,
    /// Which of the two targets holds the current field.
    current: usize,
}

/// One frame of inputs.
#[derive(Debug, Copy, Clone)]
pub struct FrameInput<'a> {
    pub dt: f32,
    /// The player's game-space xy: the origin of every splat.
    pub player: Vec2,
    /// Wading actors this frame: the handle and the position, or `None` for
    /// an actor that stopped wading.
    pub actors: &'a [(u64, Option<Vec2>)],
    /// Bullets, projectiles and bodies entering the water (game-space xy).
    pub impacts: &'a [Vec2],
}

impl Default for DisplacementState {
    fn default() -> Self {
        Self::new()
    }
}

impl DisplacementState {
    /// A state with no history: inactive, the field to be cleared on first use.
    pub fn new() -> Self {
        Self {
            centre: Vec2::ZERO,
            pending_seconds: 0.0,
            idle_seconds: IDLE_LIMIT_SECONDS,
            needs_clear: true,
            actors: BTreeMap::new(),
            impacts: Vec::new(),
            current: 0,
        }
    }

    /// The snapped centre of the wading quad.
    pub fn centre(&self) -> Vec2 {
        self.centre
    }

    pub fn idle_seconds(&self) -> f32 {
        self.idle_seconds
    }

    /// Whether the field is live: splats arrived within the last 10 seconds.
    pub fn is_live(&self) -> bool {
        self.idle_seconds < IDLE_LIMIT_SECONDS
    }

    /// The target holding the field the water shader reads.
    pub fn current_target(&self) -> usize {
        self.current
    }

    /// Runs the driver for one frame.
    pub fn frame(&mut self, input: &FrameInput<'_>) -> FramePlan {
        let texture_offset = self.resnap(input.player);
        for &(handle, position) in input.actors {
            self.update_actor(handle, position, input.player);
        }
        for &position in input.impacts {
            self.add_impact(position, input.player);
        }
        self.collect(input.dt, texture_offset)
    }

    /// Moves the quad when the player left its 4-unit window and returns the
    /// shift of the field content as the advect pass's texture offset.
    fn resnap(&mut self, player: Vec2) -> Vec2 {
        let away = (player - self.centre).abs();
        if away.x <= RESNAP_DISTANCE && away.y <= RESNAP_DISTANCE {
            return Vec2::ZERO;
        }
        let snapped = (player / SNAP_GRID).floor() * SNAP_GRID;
        let moved = snapped - self.centre;
        self.centre = snapped;
        Vec2::new(moved.x / QUAD_SIZE, -moved.y / QUAD_SIZE)
    }

    /// A new actor starts as a zero-length segment, a known one
    /// continues from the position it was last seen at. Positions are made
    /// relative to the player, not to the snapped quad centre, so a splat can
    /// land up to 8 units off the position it describes.
    fn update_actor(&mut self, handle: u64, position: Option<Vec2>, player: Vec2) {
        let Some(position) = position else {
            self.actors.remove(&handle);
            return;
        };
        // An actor update wakes an idle field like an impact splat does; without
        // it a field that went idle could never wake for a wading actor.
        self.idle_seconds = 0.0;
        let current = (position - player) / NDC_UNIT;
        let previous = self
            .actors
            .get(&handle)
            .map_or(current, |entry| (entry.last_position - player) / NDC_UNIT);
        self.actors.insert(
            handle,
            ActorEntry {
                segment: Splat { current, previous },
                last_position: position,
            },
        );
    }

    /// A fixed diagonal segment of `(0.005, 0.005)` NDC, only
    /// within `QUAD_SIZE` of the player, and it wakes the simulation.
    fn add_impact(&mut self, position: Vec2, player: Vec2) {
        if !BULLET_DISPLACEMENTS_ENABLED || !impact_in_range(position, player) {
            return;
        }
        self.idle_seconds = 0.0;
        let previous = (position - player) / NDC_UNIT;
        self.impacts.push(Splat {
            current: previous + Vec2::splat(IMPACT_SIZE * 0.5),
            previous,
        });
    }

    /// Collects the frame's splats and plans its passes.
    fn collect(&mut self, dt: f32, texture_offset: Vec2) -> FramePlan {
        if self.idle_seconds >= IDLE_LIMIT_SECONDS {
            self.needs_clear = true;
            self.impacts.clear();
            return FramePlan::default();
        }
        let mut passes = Vec::new();
        let cleared = std::mem::take(&mut self.needs_clear);
        if cleared {
            passes.push(Pass::Clear {
                target: self.current,
            });
        }
        let mut splats = std::mem::take(&mut self.impacts);
        splats.extend(self.actors.values().map(|entry| entry.segment));
        if splats.is_empty() {
            self.idle_seconds += dt;
        } else {
            self.idle_seconds = 0.0;
        }
        splats.truncate(MAX_SPLATS_PER_FRAME);

        self.pending_seconds += dt;
        passes.push(Pass::Advect {
            source: self.current,
            destination: 1 - self.current,
        });
        self.current = 1 - self.current;

        let mut steps = 0;
        while self.pending_seconds > STEP_SECONDS {
            self.pending_seconds -= STEP_SECONDS;
            if steps < MAX_STEPS_PER_FRAME {
                passes.push(Pass::Step {
                    source: self.current,
                    destination: 1 - self.current,
                });
                self.current = 1 - self.current;
                steps += 1;
            }
        }
        FramePlan {
            passes,
            texture_offset,
            splats,
            steps,
            active: true,
            cleared,
        }
    }
}

/// Plans frames of a [`DisplacementState`] without committing them: the state
/// only advances in [`Self::commit`], which the caller runs when the plan's GPU
/// passes are encoded. A plan that is never encoded (a debug skip, an early
/// return) therefore leaves the CPU state matching the field the GPU holds; the
/// time and the impacts of such frames are carried into the next plan, and the
/// quad re-snap is repeated from the committed centre so its texture offset is
/// not lost.
#[derive(Debug, Default)]
pub struct DisplacementDriver {
    committed: DisplacementState,
    planned: DisplacementState,
    plan: FramePlan,
    pending: bool,
    carried_seconds: f32,
    carried_impacts: Vec<Vec2>,
}

impl DisplacementDriver {
    pub fn new() -> Self {
        Self::default()
    }

    /// The state as this frame's plan leaves it: what the water shader reads.
    pub fn state(&self) -> &DisplacementState {
        &self.planned
    }

    pub fn plan(&self) -> &FramePlan {
        &self.plan
    }

    /// Plans one frame from the committed state and the frames that were never
    /// committed since.
    pub fn plan_frame(&mut self, input: &FrameInput<'_>) -> &FramePlan {
        self.carried_seconds += input.dt;
        self.carried_impacts.extend_from_slice(input.impacts);
        let excess = self
            .carried_impacts
            .len()
            .saturating_sub(MAX_SPLATS_PER_FRAME);
        self.carried_impacts.drain(..excess);
        self.planned = self.committed.clone();
        self.plan = self.planned.frame(&FrameInput {
            dt: self.carried_seconds,
            impacts: &self.carried_impacts,
            ..*input
        });
        self.pending = true;
        &self.plan
    }

    /// Makes the last plan the committed state; a plan commits once.
    pub fn commit(&mut self) {
        if !std::mem::take(&mut self.pending) {
            return;
        }
        self.committed = self.planned.clone();
        self.carried_seconds = 0.0;
        self.carried_impacts.clear();
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals or exactly representable results"
)]
mod tests {
    use super::*;

    const ORIGIN: Vec2 = Vec2::ZERO;

    fn frame(state: &mut DisplacementState, dt: f32, player: Vec2, impacts: &[Vec2]) -> FramePlan {
        state.frame(&FrameInput {
            dt,
            player,
            actors: &[],
            impacts,
        })
    }

    #[test]
    fn a_fresh_state_is_inactive_and_plans_nothing() {
        let mut state = DisplacementState::new();
        let plan = frame(&mut state, 0.016, ORIGIN, &[]);
        assert_eq!(plan, FramePlan::default());
        assert!(!state.is_live());
    }

    #[test]
    fn the_quad_stays_while_the_player_is_within_four_units_on_both_axes() {
        let mut state = DisplacementState::new();
        assert_eq!(state.resnap(Vec2::new(4.0, -4.0)), Vec2::ZERO);
        assert_eq!(state.centre(), Vec2::ZERO);
    }

    #[test]
    fn the_quad_resnaps_to_the_four_unit_grid_and_reports_the_content_shift() {
        let mut state = DisplacementState::new();
        let offset = state.resnap(Vec2::new(9.5, -5.0));
        assert_eq!(state.centre(), Vec2::new(8.0, -8.0));
        assert_eq!(offset, Vec2::new(8.0 / 2048.0, 8.0 / 2048.0));
        let next = state.resnap(Vec2::new(9.5, -5.0));
        assert_eq!(
            next,
            Vec2::ZERO,
            "the offset applies only the frame the quad moved"
        );
    }

    #[test]
    fn resnapping_on_one_axis_alone_moves_both_to_the_grid() {
        let mut state = DisplacementState::new();
        state.resnap(Vec2::new(0.0, 0.0));
        let offset = state.resnap(Vec2::new(5.1, 3.9));
        assert_eq!(state.centre(), Vec2::new(4.0, 0.0));
        assert_eq!(offset, Vec2::new(4.0 / 2048.0, 0.0));
    }

    #[test]
    fn an_impact_wakes_the_field_clears_it_and_runs_the_advect_pass_and_one_step() {
        let mut state = DisplacementState::new();
        let plan = frame(&mut state, 0.03, ORIGIN, &[Vec2::new(100.0, 0.0)]);
        assert!(plan.active && plan.cleared);
        assert_eq!(
            plan.passes,
            [
                Pass::Clear { target: 0 },
                Pass::Advect {
                    source: 0,
                    destination: 1
                },
                Pass::Step {
                    source: 1,
                    destination: 0
                },
            ]
        );
        assert_eq!(plan.steps, 1);
        assert_eq!(state.current_target(), 0);
    }

    #[test]
    fn an_impact_is_a_fixed_diagonal_of_half_the_size_relative_to_the_player() {
        let mut state = DisplacementState::new();
        let plan = frame(
            &mut state,
            0.016,
            Vec2::new(100.0, 200.0),
            &[Vec2::new(612.0, 200.0)],
        );
        let splat = plan.splats[0];
        assert_eq!(splat.previous, Vec2::new(0.5, 0.0));
        assert_eq!(splat.current, Vec2::new(0.505, 0.005));
    }

    #[test]
    fn impacts_beyond_the_quad_size_from_the_player_are_dropped() {
        let mut state = DisplacementState::new();
        let plan = frame(&mut state, 0.016, ORIGIN, &[Vec2::new(2049.0, 0.0)]);
        assert_eq!(plan, FramePlan::default());
        let plan = frame(&mut state, 0.016, ORIGIN, &[Vec2::new(2048.0, 0.0)]);
        assert_eq!(plan.splats.len(), 1, "the radius check is inclusive");
    }

    #[test]
    fn steps_are_fixed_at_forty_hertz_with_the_remainder_carried_over() {
        let mut state = DisplacementState::new();
        let mut total_steps = 0;
        let mut impacts: &[Vec2] = &[Vec2::new(1.0, 1.0)];
        for _ in 0..100 {
            total_steps += frame(&mut state, 0.01, ORIGIN, impacts).steps;
            impacts = &[];
        }
        // 1.0 s at 25 ms per step: the strict `>` makes it 39 or 40.
        assert!((39..=40).contains(&total_steps), "{total_steps}");
    }

    #[test]
    fn a_step_needs_strictly_more_than_the_step_length() {
        let mut state = DisplacementState::new();
        let plan = frame(&mut state, STEP_SECONDS, ORIGIN, &[Vec2::ONE]);
        assert_eq!(plan.steps, 0);
    }

    #[test]
    fn a_hitch_runs_at_most_the_step_cap() {
        let mut state = DisplacementState::new();
        let plan = frame(&mut state, 5.0, ORIGIN, &[Vec2::ONE]);
        assert_eq!(plan.steps, MAX_STEPS_PER_FRAME);
    }

    #[test]
    fn the_field_goes_idle_ten_seconds_after_the_last_splat_and_is_cleared_on_next_use() {
        let mut state = DisplacementState::new();
        frame(&mut state, 0.016, ORIGIN, &[Vec2::ONE]);
        let mut elapsed = 0.0;
        let mut last_active = true;
        while elapsed < 11.0 {
            let plan = frame(&mut state, 0.5, ORIGIN, &[]);
            if !plan.active {
                last_active = false;
                break;
            }
            elapsed += 0.5;
        }
        assert!(!last_active);
        assert!(
            (10.0..=11.0).contains(&elapsed),
            "went idle after {elapsed} s"
        );
        let woken = frame(&mut state, 0.016, ORIGIN, &[Vec2::ONE]);
        assert!(woken.cleared, "the idle period cleared the field");
        let after = frame(&mut state, 0.016, ORIGIN, &[]);
        assert!(!after.cleared);
    }

    #[test]
    fn a_wading_actor_keeps_the_field_alive_and_draws_every_frame() {
        let mut state = DisplacementState::new();
        for step in 0..40 {
            let plan = state.frame(&FrameInput {
                dt: 0.5,
                player: ORIGIN,
                actors: &[(1, Some(Vec2::new(step as f32, 0.0)))],
                impacts: &[],
            });
            assert!(plan.active, "frame {step}");
            assert_eq!(plan.splats.len(), 1);
        }
    }

    #[test]
    fn an_actor_segment_runs_from_where_it_was_to_where_it_is() {
        let mut state = DisplacementState::new();
        let at = |state: &mut DisplacementState, x: f32| {
            state.frame(&FrameInput {
                dt: 0.016,
                player: ORIGIN,
                actors: &[(7, Some(Vec2::new(x, 0.0)))],
                impacts: &[],
            })
        };
        let first = at(&mut state, 10.0);
        assert_eq!(
            first.splats[0].current, first.splats[0].previous,
            "a new actor is a zero-length segment"
        );
        let second = at(&mut state, 30.0);
        assert_eq!(second.splats[0].previous, Vec2::new(10.0 / 1024.0, 0.0));
        assert_eq!(second.splats[0].current, Vec2::new(30.0 / 1024.0, 0.0));
    }

    #[test]
    fn actor_segments_are_relative_to_the_player_not_the_snapped_centre() {
        let mut state = DisplacementState::new();
        state.resnap(Vec2::new(7.0, 0.0));
        assert_eq!(state.centre(), Vec2::new(4.0, 0.0));
        let plan = state.frame(&FrameInput {
            dt: 0.016,
            player: Vec2::new(7.0, 0.0),
            actors: &[(1, Some(Vec2::new(7.0, 0.0)))],
            impacts: &[],
        });
        assert_eq!(plan.splats[0].current, Vec2::ZERO);
    }

    #[test]
    fn an_actor_that_stops_wading_is_forgotten() {
        let mut state = DisplacementState::new();
        let mut input = FrameInput {
            dt: 0.016,
            player: ORIGIN,
            actors: &[(1, Some(Vec2::ZERO))],
            impacts: &[],
        };
        state.frame(&input);
        input.actors = &[(1, None)];
        let plan = state.frame(&input);
        assert!(plan.splats.is_empty());
    }

    #[test]
    fn the_splat_rows_follow_the_vertex_shader_inputs() {
        let splat = Splat {
            current: Vec2::new(0.5, 0.3),
            previous: Vec2::new(0.1, -0.1),
        };
        let (row_x, row_y) = splat.rows();
        let delta = Vec2::new(0.4, 0.4);
        let normal = delta.normalize();
        assert_eq!(row_x, [0.4, -normal.y, 0.1]);
        assert_eq!(row_y, [0.4, normal.x, -0.1]);
    }

    #[test]
    fn a_zero_length_segment_has_degenerate_rows() {
        let splat = Splat {
            current: Vec2::new(0.2, 0.2),
            previous: Vec2::new(0.2, 0.2),
        };
        let (row_x, row_y) = splat.rows();
        assert_eq!(row_x, [0.0, 0.0, 0.2]);
        assert_eq!(row_y, [0.0, 0.0, 0.2]);
    }

    fn driver_frame(driver: &mut DisplacementDriver, dt: f32, player: Vec2, impacts: &[Vec2]) {
        driver.plan_frame(&FrameInput {
            dt,
            player,
            actors: &[],
            impacts,
        });
    }

    #[test]
    fn an_uncommitted_plan_leaves_the_state_where_it_was() {
        let mut driver = DisplacementDriver::new();
        driver_frame(&mut driver, 0.03, ORIGIN, &[Vec2::ONE]);
        assert!(driver.plan().cleared);
        driver_frame(&mut driver, 0.03, ORIGIN, &[]);
        assert!(
            driver.plan().cleared,
            "the clear the first plan carried was never encoded, so it is planned again"
        );
        assert_eq!(
            driver.plan().passes[0],
            Pass::Clear { target: 0 },
            "the second plan starts from the committed ping-pong roles"
        );
    }

    #[test]
    fn a_committed_plan_advances_the_state() {
        let mut driver = DisplacementDriver::new();
        driver_frame(&mut driver, 0.03, ORIGIN, &[Vec2::ONE]);
        driver.commit();
        let source = driver.committed.current_target();
        driver_frame(&mut driver, 0.016, ORIGIN, &[]);
        assert!(!driver.plan().cleared);
        assert_eq!(
            driver.plan().passes[0],
            Pass::Advect {
                source,
                destination: 1 - source
            }
        );
    }

    #[test]
    fn the_time_and_impacts_of_uncommitted_frames_are_carried_into_the_next_plan() {
        let mut driver = DisplacementDriver::new();
        driver_frame(&mut driver, 0.02, ORIGIN, &[Vec2::new(10.0, 0.0)]);
        driver_frame(&mut driver, 0.02, ORIGIN, &[Vec2::new(20.0, 0.0)]);
        assert_eq!(driver.plan().splats.len(), 2, "both impacts arrive");
        assert_eq!(driver.plan().steps, 1, "0.04 s make one 25 ms step");
        driver.commit();
        driver_frame(&mut driver, 0.0, ORIGIN, &[]);
        assert!(driver.plan().splats.is_empty(), "nothing is carried twice");
    }

    #[test]
    fn a_skipped_resnap_is_repeated_so_its_texture_offset_is_not_lost() {
        let mut driver = DisplacementDriver::new();
        let far = Vec2::new(100.0, 0.0);
        driver_frame(&mut driver, 0.016, far, &[far]);
        let offset = driver.plan().texture_offset;
        assert_ne!(offset, Vec2::ZERO);
        driver_frame(&mut driver, 0.016, far, &[]);
        assert_eq!(driver.plan().texture_offset, offset);
        driver.commit();
        driver_frame(&mut driver, 0.016, far, &[]);
        assert_eq!(driver.plan().texture_offset, Vec2::ZERO);
    }

    #[test]
    fn committing_twice_does_not_advance_twice() {
        let mut driver = DisplacementDriver::new();
        driver_frame(&mut driver, 0.03, ORIGIN, &[Vec2::ONE]);
        driver.commit();
        let target = driver.committed.current_target();
        driver.commit();
        assert_eq!(driver.committed.current_target(), target);
    }

    #[test]
    fn the_carried_impacts_are_bounded() {
        let mut driver = DisplacementDriver::new();
        let batch = vec![Vec2::ONE; MAX_SPLATS_PER_FRAME];
        for _ in 0..3 {
            driver_frame(&mut driver, 0.001, ORIGIN, &batch);
        }
        assert_eq!(driver.carried_impacts.len(), MAX_SPLATS_PER_FRAME);
        assert_eq!(driver.plan().splats.len(), MAX_SPLATS_PER_FRAME);
    }

    #[test]
    fn splats_beyond_the_cap_drop_the_highest_actor_handles() {
        let mut state = DisplacementState::new();
        let actors: Vec<(u64, Option<Vec2>)> = (1..=MAX_SPLATS_PER_FRAME as u64 + 8)
            .map(|handle| (handle, Some(Vec2::new(handle as f32, 0.0))))
            .collect();
        let plan = state.frame(&FrameInput {
            dt: 0.016,
            player: ORIGIN,
            actors: &actors,
            impacts: &[],
        });
        assert_eq!(plan.splats.len(), MAX_SPLATS_PER_FRAME);
        let last = plan.splats.last().unwrap();
        assert_eq!(last.current.x, MAX_SPLATS_PER_FRAME as f32 / NDC_UNIT);
    }

    #[test]
    fn the_ping_pong_ends_on_the_target_the_last_pass_wrote() {
        let mut state = DisplacementState::new();
        let plan = frame(&mut state, 0.2, ORIGIN, &[Vec2::ONE]);
        let last_destination = plan
            .passes
            .iter()
            .rev()
            .find_map(|pass| match pass {
                Pass::Advect { destination, .. } | Pass::Step { destination, .. } => {
                    Some(*destination)
                }
                Pass::Clear { .. } => None,
            })
            .unwrap();
        assert_eq!(state.current_target(), last_destination);
    }
}
