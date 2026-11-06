//! The carry controls, matched to measurements of vanilla Fallout 4: holding E on a loose body
//! grabs it and the grab latches; a later E tap drops it when that tap is let go; R charges a
//! throw while held and throws on release; the left and right mouse buttons spin the carried
//! body while held. Pure: events and their times in, [`GrabAction`]s and the spin out.

use std::time::{Duration, Instant};

use crate::sim::{EntityId, SimCommand};

/// How long E must stay down on a loose body before it is grabbed. Measured: 150 ms presses do
/// not grab, 175 ms and longer presses do.
pub(super) const GRAB_HOLD: Duration = Duration::from_millis(160);

/// What the crosshair rests on within activation reach, as the E key sees it.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(super) enum AimTarget {
    Nothing,
    /// A load door or an animated object: activated as soon as E goes down.
    Activatable,
    /// A loose body the player may try to grab.
    Grabbable(EntityId),
}

impl AimTarget {
    /// A door or animated object wins over a loose body, so its activation never waits for the
    /// grab hold.
    pub(super) fn resolve(
        activatable: bool,
        entity: EntityId,
        is_grabbable: impl FnOnce(EntityId) -> bool,
    ) -> Self {
        if activatable {
            Self::Activatable
        } else if is_grabbable(entity) {
            Self::Grabbable(entity)
        } else {
            Self::Nothing
        }
    }
}

/// What the carry controls ask the engine to do.
#[derive(Copy, Clone, PartialEq, Debug)]
pub(super) enum GrabAction {
    /// Activate the aimed door or animated object.
    Activate,
    Carry(CarryCommand),
}

/// A command for the carried body, sent to the sim as is.
#[derive(Copy, Clone, PartialEq, Debug)]
pub(super) enum CarryCommand {
    Grab(EntityId),
    Drop,
    Throw { charge_secs: f32 },
}

impl CarryCommand {
    pub(super) fn into_sim_command(self) -> SimCommand {
        match self {
            Self::Grab(entity) => SimCommand::GrabBody { entity },
            Self::Drop => SimCommand::DropCarried,
            Self::Throw { charge_secs } => SimCommand::ThrowCarried { charge_secs },
        }
    }
}

/// A mouse button that spins the carried body while held.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(super) enum SpinButton {
    /// Tips the top of the body away from the player.
    Left,
    /// Tips the top of the body towards the player.
    Right,
}

/// What the carry controls depend on outside themselves, read from the engine when needed.
#[derive(Copy, Clone, Debug, Default)]
pub(super) struct CarryContext {
    pub walk_mode: bool,
    pub gameplay_input: bool,
    pub preview_open: bool,
    pub pipboy_open: bool,
}

impl CarryContext {
    /// Whether the carry controls drive the player: walk-mode gameplay with neither the Pip-Boy
    /// nor the loading-screen preview up. The carried body follows the walking player's view,
    /// so outside these nothing may stay carried, charged or held.
    pub(super) fn controls_live(self) -> bool {
        self.walk_mode && self.gameplay_input && !self.preview_open && !self.pipboy_open
    }
}

/// Whether an E or R key event counts. A repeat is not a new press; a synthetic event is the
/// platform's guess at a key across a focus change, and focus changes must not drop, grab or
/// throw.
pub(super) fn carry_key_counts(controls_live: bool, is_repeat: bool, is_synthetic: bool) -> bool {
    controls_live && !is_repeat && !is_synthetic
}

/// Whether the held spin buttons reach the carried body.
pub(super) fn spin_gate_open(controls_live: bool, mouse_captured: bool, carrying: bool) -> bool {
    controls_live && mouse_captured && carrying
}

/// The E key between its press and its release.
#[derive(Copy, Clone, PartialEq, Debug, Default)]
enum ActivateKey {
    #[default]
    Up,
    /// Down on a loose body since `since`, counting towards the grab of that body.
    Holding { entity: EntityId, since: Instant },
    /// Pressed while carrying: its release drops the body.
    Dropping,
    /// Down with its press already spent: its release does nothing.
    Spent,
}

#[derive(Debug, Default)]
pub(super) struct GrabInput {
    activate: ActivateKey,
    /// When R went down, while a throw is charging.
    throw_since: Option<Instant>,
    left_held: bool,
    right_held: bool,
}

impl GrabInput {
    /// E went down at `now`. Every press starts afresh, so a key-up the platform never
    /// delivered cannot leave E dead. `carrying`: whether a body is carried right now.
    pub fn activate_pressed(
        &mut self,
        aim: AimTarget,
        carrying: bool,
        now: Instant,
    ) -> Option<GrabAction> {
        if carrying {
            self.activate = ActivateKey::Dropping;
            return None;
        }
        self.activate = ActivateKey::Spent;
        match aim {
            AimTarget::Activatable => Some(GrabAction::Activate),
            AimTarget::Grabbable(entity) => {
                self.activate = ActivateKey::Holding { entity, since: now };
                None
            }
            AimTarget::Nothing => None,
        }
    }

    /// E was let go at `now`. A press on a loose body let go before the grab hold does nothing:
    /// vanilla picks the item up there, and there is no inventory yet.
    pub fn activate_released(&mut self, carrying: bool, now: Instant) -> Option<GrabAction> {
        match std::mem::take(&mut self.activate) {
            ActivateKey::Holding { entity, since } => grab_if_held(entity, since, now),
            ActivateKey::Dropping => carrying.then_some(GrabAction::Carry(CarryCommand::Drop)),
            ActivateKey::Up | ActivateKey::Spent => None,
        }
    }

    /// R went down at `now`. A press while carrying starts a fresh charge; any other press
    /// charges nothing.
    pub fn throw_pressed(&mut self, carrying: bool, now: Instant) {
        self.throw_since = carrying.then_some(now);
    }

    /// R was let go at `now`: throws with the time it was held.
    pub fn throw_released(&mut self, carrying: bool, now: Instant) -> Option<GrabAction> {
        let since = self.throw_since.take()?;
        let charge_secs = now.saturating_duration_since(since).as_secs_f32();
        carrying.then_some(GrabAction::Carry(CarryCommand::Throw { charge_secs }))
    }

    /// A spin button's physical state, tracked whatever the gate: the gate applies to the spin
    /// it produces.
    pub fn spin_button(&mut self, button: SpinButton, pressed: bool) {
        match button {
            SpinButton::Left => self.left_held = pressed,
            SpinButton::Right => self.right_held = pressed,
        }
    }

    /// Grabs the body E went down on once E has been down for the grab hold at `now`. The
    /// target is the one aimed at on the press; the sim validates it again.
    pub fn advance(&mut self, now: Instant) -> Option<GrabAction> {
        let ActivateKey::Holding { entity, since } = self.activate else {
            return None;
        };
        let grab = grab_if_held(entity, since, now);
        if grab.is_some() {
            self.activate = ActivateKey::Spent;
        }
        grab
    }

    /// The spin for the player input: +1 for the left button, -1 for the right. Both buttons
    /// held cancel out to 0, the same as neither; a closed gate spins nothing.
    pub fn spin(&self, gate_open: bool) -> i8 {
        if !gate_open {
            return 0;
        }
        i8::from(self.left_held) - i8::from(self.right_held)
    }

    /// Forgets E and R, as if both were let go without effect. The spin buttons keep their
    /// physical state.
    pub fn release_keys(&mut self) {
        self.activate = ActivateKey::Up;
        self.throw_since = None;
    }

    /// Forgets every key and button, for when their releases can no longer be seen.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

fn grab_if_held(entity: EntityId, since: Instant, now: Instant) -> Option<GrabAction> {
    (now.saturating_duration_since(since) >= GRAB_HOLD)
        .then_some(GrabAction::Carry(CarryCommand::Grab(entity)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: EntityId = EntityId(42);
    const OTHER_BODY: EntityId = EntityId(43);

    /// A clock for one test: `at(ms)` is that many milliseconds after its start.
    struct Clock(Instant);

    impl Clock {
        fn new() -> Self {
            Self(Instant::now())
        }

        fn at(&self, ms: u64) -> Instant {
            self.0 + Duration::from_millis(ms)
        }
    }

    const GRAB_BODY: Option<GrabAction> = Some(GrabAction::Carry(CarryCommand::Grab(BODY)));
    const GRAB_OTHER_BODY: Option<GrabAction> =
        Some(GrabAction::Carry(CarryCommand::Grab(OTHER_BODY)));

    const DROP: Option<GrabAction> = Some(GrabAction::Carry(CarryCommand::Drop));

    /// An input that has grabbed `BODY` and had E let go, at 300 ms.
    fn carrying_input(clock: &Clock) -> GrabInput {
        let mut input = GrabInput::default();
        input.activate_pressed(AimTarget::Grabbable(BODY), false, clock.at(0));
        assert_eq!(input.advance(clock.at(200)), GRAB_BODY);
        assert_eq!(input.activate_released(true, clock.at(300)), None);
        input
    }

    #[test]
    fn a_150_ms_press_on_a_loose_body_does_nothing() {
        let clock = Clock::new();
        let mut input = GrabInput::default();
        input.activate_pressed(AimTarget::Grabbable(BODY), false, clock.at(0));
        assert_eq!(input.advance(clock.at(100)), None);
        assert_eq!(input.advance(clock.at(150)), None);
        assert_eq!(input.activate_released(false, clock.at(150)), None);
    }

    #[test]
    fn a_170_ms_hold_grabs_the_body_once() {
        let clock = Clock::new();
        let mut input = GrabInput::default();
        input.activate_pressed(AimTarget::Grabbable(BODY), false, clock.at(0));
        assert_eq!(input.advance(clock.at(170)), GRAB_BODY);
        assert_eq!(input.advance(clock.at(500)), None);
        assert_eq!(input.activate_released(false, clock.at(600)), None);
    }

    #[test]
    fn a_long_frame_before_the_press_does_not_count_towards_the_grab_hold() {
        let clock = Clock::new();
        let mut input = GrabInput::default();
        input.activate_pressed(AimTarget::Grabbable(BODY), false, clock.at(250));
        assert_eq!(input.advance(clock.at(260)), None);
        assert_eq!(input.activate_released(false, clock.at(390)), None);
    }

    #[test]
    fn a_hold_let_go_before_any_frame_saw_it_still_grabs_on_release() {
        let clock = Clock::new();
        let mut input = GrabInput::default();
        input.activate_pressed(AimTarget::Grabbable(BODY), false, clock.at(0));
        assert_eq!(input.activate_released(false, clock.at(400)), GRAB_BODY);
    }

    #[test]
    fn releasing_e_after_the_grab_keeps_the_body_held() {
        let clock = Clock::new();
        let mut input = GrabInput::default();
        input.activate_pressed(AimTarget::Grabbable(BODY), false, clock.at(0));
        input.advance(clock.at(200));
        assert_eq!(input.activate_released(true, clock.at(1000)), None);
    }

    #[test]
    fn an_e_tap_while_carrying_drops_the_body_on_its_release_and_not_on_its_press() {
        let clock = Clock::new();
        let mut input = carrying_input(&clock);
        assert_eq!(
            input.activate_pressed(AimTarget::Grabbable(OTHER_BODY), true, clock.at(400)),
            None
        );
        assert_eq!(input.advance(clock.at(1400)), None);
        assert_eq!(input.activate_released(true, clock.at(1500)), DROP);
    }

    #[test]
    fn an_e_press_while_carrying_ignores_a_door_under_the_crosshair() {
        let clock = Clock::new();
        let mut input = carrying_input(&clock);
        assert_eq!(
            input.activate_pressed(AimTarget::Activatable, true, clock.at(400)),
            None
        );
        assert_eq!(input.activate_released(true, clock.at(500)), DROP);
    }

    #[test]
    fn a_drop_tap_whose_body_was_let_go_meanwhile_does_nothing_on_release() {
        let clock = Clock::new();
        let mut input = carrying_input(&clock);
        input.activate_pressed(AimTarget::Nothing, true, clock.at(400));
        assert_eq!(input.activate_released(false, clock.at(500)), None);
    }

    #[test]
    fn a_door_press_activates_at_once_and_its_release_does_nothing() {
        let clock = Clock::new();
        let mut input = GrabInput::default();
        assert_eq!(
            input.activate_pressed(AimTarget::Activatable, false, clock.at(0)),
            Some(GrabAction::Activate)
        );
        assert_eq!(input.advance(clock.at(1000)), None);
        assert_eq!(input.activate_released(false, clock.at(1000)), None);
    }

    #[test]
    fn a_press_on_nothing_does_nothing() {
        let clock = Clock::new();
        let mut input = GrabInput::default();
        assert_eq!(
            input.activate_pressed(AimTarget::Nothing, false, clock.at(0)),
            None
        );
        assert_eq!(input.advance(clock.at(1000)), None);
        assert_eq!(input.activate_released(false, clock.at(1000)), None);
    }

    #[test]
    fn a_press_after_a_lost_key_up_is_a_fresh_press() {
        let clock = Clock::new();
        let mut door = GrabInput::default();
        door.activate_pressed(AimTarget::Grabbable(BODY), false, clock.at(0));
        assert_eq!(
            door.activate_pressed(AimTarget::Activatable, false, clock.at(100)),
            Some(GrabAction::Activate)
        );

        let mut body = GrabInput::default();
        body.activate_pressed(AimTarget::Activatable, false, clock.at(0));
        body.activate_pressed(AimTarget::Grabbable(OTHER_BODY), false, clock.at(1000));
        assert_eq!(body.advance(clock.at(1100)), None);
        assert_eq!(body.advance(clock.at(1160)), GRAB_OTHER_BODY);
    }

    #[test]
    fn a_door_wins_over_a_grabbable_body_and_only_a_grabbable_body_is_grabbable() {
        assert_eq!(
            AimTarget::resolve(true, BODY, |_| true),
            AimTarget::Activatable
        );
        assert_eq!(
            AimTarget::resolve(false, BODY, |entity| entity == BODY),
            AimTarget::Grabbable(BODY)
        );
        assert_eq!(
            AimTarget::resolve(false, BODY, |_| false),
            AimTarget::Nothing
        );
    }

    #[test]
    fn r_held_while_carrying_throws_on_release_with_the_time_it_was_held() {
        let clock = Clock::new();
        let mut input = carrying_input(&clock);
        input.throw_pressed(true, clock.at(1000));
        assert_eq!(
            input.throw_released(true, clock.at(1750)),
            Some(GrabAction::Carry(CarryCommand::Throw { charge_secs: 0.75 }))
        );
        assert_eq!(input.throw_released(true, clock.at(1800)), None);
    }

    #[test]
    fn a_second_r_press_restarts_the_charge() {
        let clock = Clock::new();
        let mut input = carrying_input(&clock);
        input.throw_pressed(true, clock.at(1000));
        input.throw_pressed(true, clock.at(2000));
        assert_eq!(
            input.throw_released(true, clock.at(2500)),
            Some(GrabAction::Carry(CarryCommand::Throw { charge_secs: 0.5 }))
        );
    }

    #[test]
    fn r_pressed_while_not_carrying_does_nothing() {
        let clock = Clock::new();
        let mut input = GrabInput::default();
        input.throw_pressed(false, clock.at(0));
        assert_eq!(input.throw_released(true, clock.at(1000)), None);
    }

    #[test]
    fn a_throw_whose_body_was_let_go_meanwhile_does_nothing_on_release() {
        let clock = Clock::new();
        let mut input = carrying_input(&clock);
        input.throw_pressed(true, clock.at(1000));
        assert_eq!(input.throw_released(false, clock.at(1500)), None);
    }

    #[test]
    fn the_left_button_spins_forward_the_right_backward_and_both_cancel_out() {
        let mut input = GrabInput::default();
        assert_eq!(input.spin(true), 0);
        input.spin_button(SpinButton::Left, true);
        assert_eq!(input.spin(true), 1);
        input.spin_button(SpinButton::Right, true);
        assert_eq!(input.spin(true), 0);
        input.spin_button(SpinButton::Left, false);
        assert_eq!(input.spin(true), -1);
        input.spin_button(SpinButton::Right, false);
        assert_eq!(input.spin(true), 0);
    }

    #[test]
    fn a_closed_gate_spins_nothing_but_the_buttons_keep_their_state() {
        let mut input = GrabInput::default();
        input.spin_button(SpinButton::Left, true);
        assert_eq!(input.spin(false), 0);
        assert_eq!(input.spin(true), 1);
    }

    #[test]
    fn the_spin_gate_needs_live_controls_a_captured_mouse_and_a_carried_body() {
        assert!(spin_gate_open(true, true, true));
        for (live, captured, carrying) in [
            (false, true, true),
            (true, false, true),
            (true, true, false),
        ] {
            assert!(
                !spin_gate_open(live, captured, carrying),
                "{live} {captured} {carrying}"
            );
        }
    }

    #[test]
    fn releasing_the_keys_keeps_a_spin_button_held_through_a_throw_and_a_regrab() {
        let clock = Clock::new();
        let mut input = carrying_input(&clock);
        input.spin_button(SpinButton::Left, true);
        input.throw_pressed(true, clock.at(1000));
        input.throw_released(true, clock.at(1100));
        input.release_keys();
        input.activate_pressed(AimTarget::Grabbable(BODY), false, clock.at(2000));
        assert_eq!(input.advance(clock.at(2200)), GRAB_BODY);
        assert_eq!(input.spin(true), 1);
    }

    #[test]
    fn releasing_the_keys_forgets_the_grab_hold_the_drop_tap_and_the_throw_charge() {
        let clock = Clock::new();
        let mut holding = GrabInput::default();
        holding.activate_pressed(AimTarget::Grabbable(BODY), false, clock.at(0));
        holding.release_keys();
        assert_eq!(holding.advance(clock.at(1000)), None);
        assert_eq!(holding.activate_released(false, clock.at(1000)), None);

        let mut carrying = carrying_input(&clock);
        carrying.throw_pressed(true, clock.at(1000));
        carrying.activate_pressed(AimTarget::Nothing, true, clock.at(1000));
        carrying.release_keys();
        assert_eq!(carrying.throw_released(true, clock.at(1500)), None);
        assert_eq!(carrying.activate_released(true, clock.at(1500)), None);
    }

    #[test]
    fn a_reset_also_forgets_the_spin_buttons() {
        let clock = Clock::new();
        let mut input = carrying_input(&clock);
        input.spin_button(SpinButton::Right, true);
        input.throw_pressed(true, clock.at(1000));
        input.reset();
        assert_eq!(input.spin(true), 0);
        assert_eq!(input.throw_released(true, clock.at(1500)), None);
    }

    #[test]
    fn the_controls_are_live_only_in_walk_mode_gameplay_without_the_pipboy_or_the_preview() {
        let live = CarryContext {
            walk_mode: true,
            gameplay_input: true,
            preview_open: false,
            pipboy_open: false,
        };
        assert!(live.controls_live());
        for context in [
            CarryContext {
                walk_mode: false,
                ..live
            },
            CarryContext {
                gameplay_input: false,
                ..live
            },
            CarryContext {
                preview_open: true,
                ..live
            },
            CarryContext {
                pipboy_open: true,
                ..live
            },
        ] {
            assert!(!context.controls_live(), "{context:?}");
        }
    }

    #[test]
    fn only_a_real_first_key_event_with_live_controls_counts() {
        assert!(carry_key_counts(true, false, false));
        assert!(!carry_key_counts(false, false, false), "pipboy or menu");
        assert!(!carry_key_counts(true, true, false), "repeat");
        assert!(!carry_key_counts(true, false, true), "synthetic");
    }

    #[test]
    fn each_carry_command_becomes_its_sim_command() {
        assert!(matches!(
            CarryCommand::Grab(BODY).into_sim_command(),
            SimCommand::GrabBody { entity } if entity == BODY
        ));
        assert!(matches!(
            CarryCommand::Drop.into_sim_command(),
            SimCommand::DropCarried
        ));
        assert!(matches!(
            CarryCommand::Throw { charge_secs: 1.25 }.into_sim_command(),
            SimCommand::ThrowCarried { charge_secs } if charge_secs.to_bits() == 1.25_f32.to_bits()
        ));
    }
}
