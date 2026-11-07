//! The pure rules of how a loading screen's model moves: which way the held pan keys push it,
//! where the slow spin has turned it to and when the spin runs.

const HALF_TURN_DEGREES: f32 = 180.0;

/// The four directions the model can be moved across the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanKey {
    Up,
    Down,
    Left,
    Right,
}

/// Which pan keys are held.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PanKeys {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
}

impl PanKeys {
    pub fn set(&mut self, key: PanKey, held: bool) {
        match key {
            PanKey::Up => self.up = held,
            PanKey::Down => self.down = held,
            PanKey::Left => self.left = held,
            PanKey::Right => self.right = held,
        }
    }

    /// The direction the held keys push the model, `(right, up)`, each of -1, 0 or 1: keys of
    /// opposite directions cancel.
    pub fn direction(self) -> (f32, f32) {
        let axis =
            |positive: bool, negative: bool| f32::from(i8::from(positive) - i8::from(negative));
        (axis(self.right, self.left), axis(self.up, self.down))
    }
}

/// Whether `limits` let the model turn a whole way round, so that turning never has to stop.
pub fn is_full_turn(limits: (f32, f32)) -> bool {
    limits.0 <= -HALF_TURN_DEGREES && limits.1 >= HALF_TURN_DEGREES
}

/// `degrees` brought into `[-180, 180)`.
pub fn wrap_half_turns(degrees: f32) -> f32 {
    (degrees + HALF_TURN_DEGREES).rem_euclid(2.0 * HALF_TURN_DEGREES) - HALF_TURN_DEGREES
}

/// Where a model that starts at `start` degrees and spins `travelled` degrees in the positive
/// direction stands, within `limits`. With limits of a full turn it keeps going round; with any
/// narrower ones it turns back at each end, so it sweeps between them for ever. A model whose
/// limits leave it no room stays at its limit.
pub fn spun_yaw(start: f32, travelled: f32, limits: (f32, f32)) -> f32 {
    let (min, max) = limits;
    if is_full_turn(limits) {
        return wrap_half_turns(start + travelled);
    }
    let span = max - min;
    if span <= 0.0 {
        return min;
    }
    let along = (start.clamp(min, max) - min + travelled).rem_euclid(2.0 * span);
    min + if along <= span {
        along
    } else {
        2.0 * span - along
    }
}

/// Whether the model's own spin runs: it does from the start, stops while the viewer drags and
/// runs again once a delay has passed after the drag ended. A pinned model never spins again.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpinGate {
    resume_delay_seconds: f32,
    state: SpinState,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SpinState {
    Running,
    Dragged,
    Resting { seconds: f32 },
    Pinned,
}

impl SpinGate {
    /// A gate that lets the spin run `resume_delay_seconds` after a drag ends; a delay that is
    /// not a finite number of seconds, 0 or more, counts as none.
    pub fn new(resume_delay_seconds: f32) -> Self {
        Self {
            resume_delay_seconds: sanitized_delay(resume_delay_seconds),
            state: SpinState::Running,
        }
    }

    pub fn is_running(self) -> bool {
        self.state == SpinState::Running
    }

    /// The viewer moved the model by hand.
    pub fn drag_moved(&mut self) {
        if self.state != SpinState::Pinned {
            self.state = SpinState::Dragged;
        }
    }

    /// The viewer let go of the model; the delay counts from here. Without a drag, it does nothing.
    pub fn drag_ended(&mut self) {
        if self.state == SpinState::Dragged {
            self.state = SpinState::Resting { seconds: 0.0 };
            self.settle();
        }
    }

    /// Lets `seconds` of the delay pass.
    pub fn tick(&mut self, seconds: f32) {
        if let SpinState::Resting { seconds: rested } = &mut self.state {
            *rested += seconds;
            self.settle();
        }
    }

    /// Stops the spin for good.
    pub fn pin(&mut self) {
        self.state = SpinState::Pinned;
    }

    fn settle(&mut self) {
        if let SpinState::Resting { seconds } = self.state
            && seconds >= self.resume_delay_seconds
        {
            self.state = SpinState::Running;
        }
    }
}

fn sanitized_delay(seconds: f32) -> f32 {
    if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the expected values are exact for these inputs"
)]
mod tests {
    use super::*;

    fn close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-3,
            "expected about {expected}, got {actual}"
        );
    }

    #[test]
    fn no_key_pushes_nowhere_and_each_key_pushes_its_way() {
        let mut keys = PanKeys::default();
        assert_eq!(keys.direction(), (0.0, 0.0));
        for (key, direction) in [
            (PanKey::Up, (0.0, 1.0)),
            (PanKey::Down, (0.0, -1.0)),
            (PanKey::Left, (-1.0, 0.0)),
            (PanKey::Right, (1.0, 0.0)),
        ] {
            keys.set(key, true);
            assert_eq!(keys.direction(), direction, "{key:?}");
            keys.set(key, false);
        }
    }

    #[test]
    fn keys_of_opposite_directions_cancel_and_diagonals_combine() {
        let mut keys = PanKeys::default();
        keys.set(PanKey::Left, true);
        keys.set(PanKey::Right, true);
        assert_eq!(keys.direction(), (0.0, 0.0));
        keys.set(PanKey::Up, true);
        assert_eq!(keys.direction(), (0.0, 1.0));
        keys.set(PanKey::Right, false);
        assert_eq!(keys.direction(), (-1.0, 1.0));
    }

    #[test]
    fn a_full_turn_needs_both_ends_of_the_half_turn() {
        assert!(is_full_turn((-180.0, 180.0)));
        assert!(!is_full_turn((-180.0, 90.0)));
        assert!(!is_full_turn((-90.0, 180.0)));
    }

    #[test]
    fn a_full_turn_keeps_going_round() {
        let limits = (-180.0, 180.0);
        close(spun_yaw(0.0, 90.0, limits), 90.0);
        close(spun_yaw(0.0, 200.0, limits), -160.0);
        close(spun_yaw(0.0, 360.0 * 3.0 + 10.0, limits), 10.0);
        close(spun_yaw(170.0, 20.0, limits), -170.0);
    }

    #[test]
    fn narrower_limits_turn_back_at_each_end() {
        let limits = (-30.0, 45.0);
        close(spun_yaw(0.0, 20.0, limits), 20.0);
        close(spun_yaw(0.0, 45.0, limits), 45.0);
        close(spun_yaw(0.0, 55.0, limits), 35.0);
        close(spun_yaw(0.0, 45.0 + 75.0, limits), -30.0);
        close(spun_yaw(0.0, 45.0 + 75.0 + 10.0, limits), -20.0);
        close(spun_yaw(0.0, 215.0, limits), 25.0);
    }

    #[test]
    fn limits_on_one_side_of_the_half_turn_sweep_between_their_ends() {
        let limits = (-180.0, 90.0);
        close(spun_yaw(0.0, 90.0, limits), 90.0);
        close(spun_yaw(0.0, 100.0, limits), 80.0);
        close(spun_yaw(0.0, 90.0 + 270.0 + 5.0, limits), -175.0);
    }

    #[test]
    fn a_start_outside_the_limits_begins_at_the_nearest_end() {
        close(spun_yaw(0.0, 0.0, (10.0, 50.0)), 10.0);
        close(spun_yaw(0.0, 5.0, (10.0, 50.0)), 15.0);
    }

    #[test]
    fn limits_with_no_room_hold_the_model_at_the_limit() {
        assert_eq!(spun_yaw(0.0, 100.0, (20.0, 20.0)), 20.0);
        assert_eq!(spun_yaw(0.0, 100.0, (20.0, 10.0)), 20.0);
    }

    #[test]
    fn spinning_backwards_in_time_gives_the_mirror_position() {
        let limits = (-30.0, 45.0);
        close(spun_yaw(0.0, -10.0, limits), -10.0);
        close(spun_yaw(0.0, -40.0, limits), -20.0);
    }

    #[test]
    fn a_fresh_gate_runs_and_a_drag_stops_it() {
        let mut gate = SpinGate::new(1.0);
        assert!(gate.is_running());
        gate.drag_moved();
        assert!(!gate.is_running());
        gate.tick(100.0);
        assert!(!gate.is_running(), "time alone does not resume a held drag");
    }

    #[test]
    fn the_spin_resumes_once_the_delay_has_passed_after_the_drag_ends() {
        let mut gate = SpinGate::new(1.0);
        gate.drag_moved();
        gate.drag_ended();
        gate.tick(0.6);
        assert!(!gate.is_running());
        gate.tick(0.3);
        assert!(!gate.is_running(), "still short of the delay");
        gate.tick(0.2);
        assert!(gate.is_running());
    }

    #[test]
    fn a_new_drag_during_the_delay_starts_the_delay_over() {
        let mut gate = SpinGate::new(1.0);
        gate.drag_moved();
        gate.drag_ended();
        gate.tick(0.9);
        gate.drag_moved();
        gate.drag_ended();
        gate.tick(0.9);
        assert!(!gate.is_running());
        gate.tick(0.2);
        assert!(gate.is_running());
    }

    #[test]
    fn without_a_delay_the_spin_resumes_as_the_drag_ends() {
        let mut gate = SpinGate::new(0.0);
        gate.drag_moved();
        gate.drag_ended();
        assert!(gate.is_running());
    }

    #[test]
    fn a_delay_that_is_not_a_usable_number_counts_as_none() {
        for delay in [f32::NAN, f32::INFINITY, -3.0] {
            let mut gate = SpinGate::new(delay);
            gate.drag_moved();
            gate.drag_ended();
            assert!(gate.is_running(), "{delay}");
        }
    }

    #[test]
    fn a_release_without_a_drag_changes_nothing() {
        let mut gate = SpinGate::new(1.0);
        gate.drag_ended();
        assert!(gate.is_running());
    }

    #[test]
    fn a_pinned_gate_never_runs_again() {
        let mut gate = SpinGate::new(0.0);
        gate.pin();
        gate.drag_moved();
        gate.drag_ended();
        gate.tick(10.0);
        assert!(!gate.is_running());
    }
}
