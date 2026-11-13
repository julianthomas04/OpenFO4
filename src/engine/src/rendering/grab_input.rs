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
    /// A placed item and what the sim says of its body.
    Item {
        entity: EntityId,
        body: ItemBody,
    },
}

/// What the sim says of a placed item's body.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(super) enum ItemBody {
    /// A movable body within the mass a grab lifts.
    Carriable,
    /// A movable body above that mass: a hold reports it, the release takes the item.
    TooHeavy,
    /// No movable body (pinned, static, or outside the dynamic radius): E acts on the release
    /// only, whatever the hold.
    Fixed,
}

impl AimTarget {
    /// A door or animated object wins over an item, an item over a loose body, so a door's
    /// activation never waits for the grab hold. Only the closures of the branches taken run.
    pub(super) fn resolve(
        activatable: bool,
        entity: EntityId,
        is_item: impl FnOnce(EntityId) -> bool,
        is_grabbable: impl FnOnce(EntityId) -> bool,
        can_carry: impl FnOnce(EntityId) -> bool,
    ) -> Self {
        if activatable {
            Self::Activatable
        } else if is_item(entity) {
            let body = if can_carry(entity) {
                ItemBody::Carriable
            } else if is_grabbable(entity) {
                ItemBody::TooHeavy
            } else {
                ItemBody::Fixed
            };
            Self::Item { entity, body }
        } else if is_grabbable(entity) {
            Self::Grabbable(entity)
        } else {
            Self::Nothing
        }
    }

    /// While a body is carried the crosshair ray hits that body, so an item or loose body under
    /// it is no target; a door still is.
    pub(super) fn while_carrying(self, carrying: bool) -> Self {
        match self {
            Self::Item { .. } | Self::Grabbable(_) if carrying => Self::Nothing,
            other => other,
        }
    }

    /// Whether E does something at once or on release here: a door or animated object, or an
    /// item. A loose body only grabs on a hold.
    pub(super) fn is_interactible(self) -> bool {
        matches!(self, Self::Activatable | Self::Item { .. })
    }

    /// Whether this is the placed item `entity`.
    fn is_item(self, entity: EntityId) -> bool {
        matches!(self, Self::Item { entity: aimed, .. } if aimed == entity)
    }
}

/// What the carry controls ask the engine to do.
#[derive(Copy, Clone, PartialEq, Debug)]
pub(super) enum GrabAction {
    /// Activate the aimed door or animated object.
    Activate,
    Carry(CarryCommand),
    /// Take the placed item into the inventory.
    PickUp(EntityId),
    /// Tell the player the movable item E is held on is too heavy to carry.
    ReportTooHeavy,
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
    /// Down on a loose body or an item since `since`, counting towards the grab of that body or
    /// the pickup of that item.
    Holding {
        entity: EntityId,
        since: Instant,
        target: HeldTarget,
    },
    /// Pressed while carrying: its release drops the body.
    Dropping,
    /// Down with its press already spent: its release does nothing.
    Spent,
}

/// What E went down on.
#[derive(Copy, Clone, PartialEq, Debug)]
enum HeldTarget {
    /// A loose body: a hold grabs it, a tap does nothing.
    Body,
    /// An item that can be carried: a tap picks it up, a hold grabs it.
    GrabbableItem,
    /// A movable item above the grab mass: a hold reports it, its release picks it up, however
    /// long E was down.
    HeavyItem,
    /// A `HeavyItem` whose hold already reached the grab hold and was reported: its release
    /// picks it up all the same.
    ReportedHeavyItem,
    /// An item without a movable body: no grab, no report, its release picks it up, however long
    /// E was down.
    FixedItem,
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
            AimTarget::Grabbable(entity) => self.hold(entity, HeldTarget::Body, now),
            AimTarget::Item { entity, body } => {
                let target = match body {
                    ItemBody::Carriable => HeldTarget::GrabbableItem,
                    ItemBody::TooHeavy => HeldTarget::HeavyItem,
                    ItemBody::Fixed => HeldTarget::FixedItem,
                };
                self.hold(entity, target, now)
            }
            AimTarget::Nothing => None,
        }
    }

    fn hold(&mut self, entity: EntityId, target: HeldTarget, now: Instant) -> Option<GrabAction> {
        self.activate = ActivateKey::Holding {
            entity,
            since: now,
            target,
        };
        None
    }

    /// E was let go at `now`, with `aimed` under the crosshair. A press on a loose body let go
    /// before the grab hold does nothing. A press on an item picks it up when let go before the
    /// grab hold, or at any time when the item cannot be carried, provided the crosshair still
    /// rests on that item. A hold that reached the grab hold grabs and never picks up.
    pub fn activate_released(
        &mut self,
        carrying: bool,
        now: Instant,
        aimed: AimTarget,
    ) -> Option<GrabAction> {
        match std::mem::take(&mut self.activate) {
            ActivateKey::Holding {
                entity,
                since,
                target,
            } => release_after_hold(entity, since, target, now, aimed),
            ActivateKey::Dropping => carrying.then_some(GrabAction::Carry(CarryCommand::Drop)),
            ActivateKey::Up | ActivateKey::Spent => None,
        }
    }

    /// E was let go at `now`: first what its hold had reached by `now` without a frame having
    /// seen it (the grab, or the too-heavy report), then the release itself. A heavy item held
    /// past the grab hold is reported and taken however the frames fell.
    pub fn release_with_due_actions(
        &mut self,
        carrying: bool,
        now: Instant,
        aimed: AimTarget,
    ) -> [Option<GrabAction>; 2] {
        let due = self.advance(now);
        [due, self.activate_released(carrying, now, aimed)]
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

    /// Once E has been down for the grab hold at `now`: grabs the body or carriable item E went
    /// down on, or reports once that the item cannot be carried. The target is the one aimed at
    /// on the press; the sim validates it again.
    pub fn advance(&mut self, now: Instant) -> Option<GrabAction> {
        let ActivateKey::Holding {
            entity,
            since,
            target,
        } = self.activate
        else {
            return None;
        };
        let held_long_enough = now.saturating_duration_since(since) >= GRAB_HOLD;
        match target {
            HeldTarget::Body | HeldTarget::GrabbableItem => {
                let grab = grab_if_held(entity, since, now);
                if grab.is_some() {
                    self.activate = ActivateKey::Spent;
                }
                grab
            }
            HeldTarget::HeavyItem if held_long_enough => {
                self.activate = ActivateKey::Holding {
                    entity,
                    since,
                    target: HeldTarget::ReportedHeavyItem,
                };
                Some(GrabAction::ReportTooHeavy)
            }
            HeldTarget::HeavyItem | HeldTarget::ReportedHeavyItem | HeldTarget::FixedItem => None,
        }
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

fn release_after_hold(
    entity: EntityId,
    since: Instant,
    target: HeldTarget,
    now: Instant,
    aimed: AimTarget,
) -> Option<GrabAction> {
    let pick_up = || aimed.is_item(entity).then_some(GrabAction::PickUp(entity));
    match target {
        HeldTarget::Body => grab_if_held(entity, since, now),
        HeldTarget::GrabbableItem => grab_if_held(entity, since, now).or_else(pick_up),
        HeldTarget::HeavyItem | HeldTarget::ReportedHeavyItem | HeldTarget::FixedItem => pick_up(),
    }
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
        assert_eq!(
            input.activate_released(true, clock.at(300), AimTarget::Nothing),
            None
        );
        input
    }

    #[test]
    fn a_150_ms_press_on_a_loose_body_does_nothing() {
        let clock = Clock::new();
        let mut input = GrabInput::default();
        input.activate_pressed(AimTarget::Grabbable(BODY), false, clock.at(0));
        assert_eq!(input.advance(clock.at(100)), None);
        assert_eq!(input.advance(clock.at(150)), None);
        assert_eq!(
            input.activate_released(false, clock.at(150), AimTarget::Nothing),
            None
        );
    }

    #[test]
    fn a_170_ms_hold_grabs_the_body_once() {
        let clock = Clock::new();
        let mut input = GrabInput::default();
        input.activate_pressed(AimTarget::Grabbable(BODY), false, clock.at(0));
        assert_eq!(input.advance(clock.at(170)), GRAB_BODY);
        assert_eq!(input.advance(clock.at(500)), None);
        assert_eq!(
            input.activate_released(false, clock.at(600), AimTarget::Nothing),
            None
        );
    }

    #[test]
    fn a_long_frame_before_the_press_does_not_count_towards_the_grab_hold() {
        let clock = Clock::new();
        let mut input = GrabInput::default();
        input.activate_pressed(AimTarget::Grabbable(BODY), false, clock.at(250));
        assert_eq!(input.advance(clock.at(260)), None);
        assert_eq!(
            input.activate_released(false, clock.at(390), AimTarget::Nothing),
            None
        );
    }

    #[test]
    fn a_hold_let_go_before_any_frame_saw_it_still_grabs_on_release() {
        let clock = Clock::new();
        let mut input = GrabInput::default();
        input.activate_pressed(AimTarget::Grabbable(BODY), false, clock.at(0));
        assert_eq!(
            input.activate_released(false, clock.at(400), AimTarget::Nothing),
            GRAB_BODY
        );
    }

    #[test]
    fn releasing_e_after_the_grab_keeps_the_body_held() {
        let clock = Clock::new();
        let mut input = GrabInput::default();
        input.activate_pressed(AimTarget::Grabbable(BODY), false, clock.at(0));
        input.advance(clock.at(200));
        assert_eq!(
            input.activate_released(true, clock.at(1000), AimTarget::Nothing),
            None
        );
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
        assert_eq!(
            input.activate_released(true, clock.at(1500), AimTarget::Nothing),
            DROP
        );
    }

    #[test]
    fn an_e_press_while_carrying_ignores_a_door_under_the_crosshair() {
        let clock = Clock::new();
        let mut input = carrying_input(&clock);
        assert_eq!(
            input.activate_pressed(AimTarget::Activatable, true, clock.at(400)),
            None
        );
        assert_eq!(
            input.activate_released(true, clock.at(500), AimTarget::Nothing),
            DROP
        );
    }

    #[test]
    fn a_drop_tap_whose_body_was_let_go_meanwhile_does_nothing_on_release() {
        let clock = Clock::new();
        let mut input = carrying_input(&clock);
        input.activate_pressed(AimTarget::Nothing, true, clock.at(400));
        assert_eq!(
            input.activate_released(false, clock.at(500), AimTarget::Nothing),
            None
        );
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
        assert_eq!(
            input.activate_released(false, clock.at(1000), AimTarget::Nothing),
            None
        );
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
        assert_eq!(
            input.activate_released(false, clock.at(1000), AimTarget::Nothing),
            None
        );
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

    const NEVER: fn(EntityId) -> bool = |_| panic!("a closure of a branch not taken ran");

    #[test]
    fn a_door_wins_over_an_item_and_an_item_over_a_grabbable_body() {
        assert_eq!(
            AimTarget::resolve(true, BODY, NEVER, NEVER, NEVER),
            AimTarget::Activatable
        );
        assert_eq!(
            AimTarget::resolve(false, BODY, |_| true, NEVER, |_| true),
            ITEM
        );
        assert_eq!(
            AimTarget::resolve(false, BODY, |_| false, |entity| entity == BODY, NEVER),
            AimTarget::Grabbable(BODY)
        );
        assert_eq!(
            AimTarget::resolve(false, BODY, |_| false, |_| false, NEVER),
            AimTarget::Nothing
        );
    }

    #[test]
    fn an_item_is_too_heavy_only_with_a_movable_body_and_fixed_without_one() {
        assert_eq!(
            AimTarget::resolve(false, BODY, |_| true, |_| true, |_| false),
            HEAVY_ITEM
        );
        assert_eq!(
            AimTarget::resolve(false, BODY, |_| true, |_| false, |_| false),
            FIXED_ITEM
        );
    }

    #[test]
    fn doors_and_items_are_interactible_and_loose_bodies_and_nothing_are_not() {
        assert!(AimTarget::Activatable.is_interactible());
        assert!(ITEM.is_interactible());
        assert!(HEAVY_ITEM.is_interactible());
        assert!(FIXED_ITEM.is_interactible());
        assert!(!AimTarget::Grabbable(BODY).is_interactible());
        assert!(!AimTarget::Nothing.is_interactible());
    }

    #[test]
    fn while_a_body_is_carried_no_item_or_body_is_aimed_at_but_a_door_still_is() {
        for aim in [ITEM, HEAVY_ITEM, FIXED_ITEM, AimTarget::Grabbable(BODY)] {
            assert_eq!(aim.while_carrying(true), AimTarget::Nothing, "{aim:?}");
            assert_eq!(aim.while_carrying(false), aim, "{aim:?}");
        }
        for aim in [AimTarget::Activatable, AimTarget::Nothing] {
            assert_eq!(aim.while_carrying(true), aim, "{aim:?}");
        }
    }

    const ITEM: AimTarget = AimTarget::Item {
        entity: BODY,
        body: ItemBody::Carriable,
    };
    const HEAVY_ITEM: AimTarget = AimTarget::Item {
        entity: BODY,
        body: ItemBody::TooHeavy,
    };
    const FIXED_ITEM: AimTarget = AimTarget::Item {
        entity: BODY,
        body: ItemBody::Fixed,
    };
    const OTHER_ITEM: AimTarget = AimTarget::Item {
        entity: OTHER_BODY,
        body: ItemBody::Carriable,
    };
    const PICK_UP: Option<GrabAction> = Some(GrabAction::PickUp(BODY));

    fn pressed_on(aim: AimTarget, clock: &Clock) -> GrabInput {
        let mut input = GrabInput::default();
        assert_eq!(input.activate_pressed(aim, false, clock.at(0)), None);
        input
    }

    #[test]
    fn a_tap_on_a_carriable_item_picks_it_up_on_release() {
        let clock = Clock::new();
        let mut input = pressed_on(ITEM, &clock);
        assert_eq!(input.advance(clock.at(100)), None);
        assert_eq!(input.activate_released(false, clock.at(150), ITEM), PICK_UP);
        assert_eq!(input.activate_released(false, clock.at(160), ITEM), None);
    }

    #[test]
    fn a_tap_picks_up_only_when_the_pressed_item_is_still_aimed_at() {
        let clock = Clock::new();
        for aimed in [
            AimTarget::Nothing,
            AimTarget::Activatable,
            AimTarget::Grabbable(BODY),
            OTHER_ITEM,
        ] {
            let mut input = pressed_on(ITEM, &clock);
            assert_eq!(
                input.activate_released(false, clock.at(100), aimed),
                None,
                "{aimed:?}"
            );
        }
    }

    #[test]
    fn a_tap_on_an_item_that_became_too_heavy_to_carry_still_picks_it_up() {
        let clock = Clock::new();
        let mut input = pressed_on(ITEM, &clock);
        assert_eq!(
            input.activate_released(false, clock.at(100), HEAVY_ITEM),
            PICK_UP
        );
    }

    #[test]
    fn holding_a_carriable_item_grabs_it_once_and_a_later_release_does_nothing() {
        let clock = Clock::new();
        let mut input = pressed_on(ITEM, &clock);
        assert_eq!(input.advance(clock.at(159)), None);
        assert_eq!(input.advance(clock.at(160)), GRAB_BODY);
        assert_eq!(input.advance(clock.at(500)), None);
        assert_eq!(input.activate_released(true, clock.at(600), ITEM), None);
    }

    #[test]
    fn a_hold_that_reached_the_grab_hold_unseen_grabs_on_release_and_never_picks_up() {
        let clock = Clock::new();
        let mut input = pressed_on(ITEM, &clock);
        assert_eq!(
            input.activate_released(false, clock.at(160), ITEM),
            GRAB_BODY
        );
    }

    #[test]
    fn an_item_that_cannot_be_carried_is_picked_up_on_release_whatever_the_duration() {
        let clock = Clock::new();
        for released_at in [10, 159, 160, 161, 5000] {
            let mut input = pressed_on(HEAVY_ITEM, &clock);
            input.advance(clock.at(released_at));
            assert_eq!(
                input.activate_released(false, clock.at(released_at), HEAVY_ITEM),
                PICK_UP,
                "{released_at}"
            );
        }
    }

    #[test]
    fn holding_e_on_an_item_that_cannot_be_carried_reports_it_once_at_the_grab_hold() {
        let clock = Clock::new();
        let mut input = pressed_on(HEAVY_ITEM, &clock);
        assert_eq!(input.advance(clock.at(159)), None);
        assert_eq!(
            input.advance(clock.at(160)),
            Some(GrabAction::ReportTooHeavy)
        );
        assert_eq!(input.advance(clock.at(161)), None);
        assert_eq!(input.advance(clock.at(5000)), None);
    }

    #[test]
    fn an_unseen_hold_on_an_item_that_cannot_be_carried_is_reported_and_taken_on_release() {
        let clock = Clock::new();
        for released_at in [160, 161, 170, 5000] {
            let mut input = pressed_on(HEAVY_ITEM, &clock);
            assert_eq!(
                input.release_with_due_actions(false, clock.at(released_at), HEAVY_ITEM),
                [Some(GrabAction::ReportTooHeavy), PICK_UP],
                "{released_at}"
            );
        }
    }

    #[test]
    fn a_tap_on_an_item_that_cannot_be_carried_released_unseen_is_only_taken() {
        let clock = Clock::new();
        for released_at in [10, 159] {
            let mut input = pressed_on(HEAVY_ITEM, &clock);
            assert_eq!(
                input.release_with_due_actions(false, clock.at(released_at), HEAVY_ITEM),
                [None, PICK_UP],
                "{released_at}"
            );
        }
    }

    #[test]
    fn a_hold_already_reported_is_not_reported_again_on_release() {
        let clock = Clock::new();
        let mut input = pressed_on(HEAVY_ITEM, &clock);
        assert_eq!(
            input.advance(clock.at(200)),
            Some(GrabAction::ReportTooHeavy)
        );
        assert_eq!(
            input.release_with_due_actions(false, clock.at(900), HEAVY_ITEM),
            [None, PICK_UP]
        );
    }

    #[test]
    fn an_unseen_hold_on_a_body_or_carriable_item_grabs_once_on_release() {
        let clock = Clock::new();
        for aim in [ITEM, AimTarget::Grabbable(BODY)] {
            let mut input = pressed_on(aim, &clock);
            assert_eq!(
                input.release_with_due_actions(false, clock.at(300), aim),
                [GRAB_BODY, None],
                "{aim:?}"
            );
        }
    }

    #[test]
    fn a_short_tap_on_a_carriable_item_picks_it_up_on_release() {
        let clock = Clock::new();
        let mut input = pressed_on(ITEM, &clock);
        assert_eq!(
            input.release_with_due_actions(false, clock.at(100), ITEM),
            [None, PICK_UP]
        );
    }

    #[test]
    fn an_item_without_a_movable_body_is_never_reported_or_grabbed_and_taken_on_release() {
        let clock = Clock::new();
        for released_at in [10, 160, 700, 5000] {
            let mut input = pressed_on(FIXED_ITEM, &clock);
            assert_eq!(input.advance(clock.at(released_at)), None, "{released_at}");
            assert_eq!(
                input.release_with_due_actions(false, clock.at(released_at), FIXED_ITEM),
                [None, PICK_UP],
                "{released_at}"
            );
        }
    }

    #[test]
    fn an_item_without_a_movable_body_is_not_taken_when_the_aim_moved_off_it() {
        let clock = Clock::new();
        let mut input = pressed_on(FIXED_ITEM, &clock);
        assert_eq!(
            input.release_with_due_actions(false, clock.at(700), AimTarget::Nothing),
            [None, None]
        );
    }

    #[test]
    fn a_tap_on_an_item_that_cannot_be_carried_reports_nothing_and_still_picks_it_up() {
        let clock = Clock::new();
        let mut input = pressed_on(HEAVY_ITEM, &clock);
        assert_eq!(input.advance(clock.at(100)), None);
        assert_eq!(
            input.activate_released(false, clock.at(100), HEAVY_ITEM),
            PICK_UP
        );
    }

    #[test]
    fn the_item_is_still_picked_up_on_release_after_it_was_reported() {
        let clock = Clock::new();
        let mut input = pressed_on(HEAVY_ITEM, &clock);
        assert_eq!(
            input.advance(clock.at(200)),
            Some(GrabAction::ReportTooHeavy)
        );
        assert_eq!(
            input.activate_released(false, clock.at(1000), HEAVY_ITEM),
            PICK_UP
        );
    }

    #[test]
    fn each_hold_on_an_item_that_cannot_be_carried_reports_again() {
        let clock = Clock::new();
        let mut input = pressed_on(HEAVY_ITEM, &clock);
        input.advance(clock.at(200));
        input.activate_released(false, clock.at(300), HEAVY_ITEM);
        input.activate_pressed(HEAVY_ITEM, false, clock.at(1000));
        assert_eq!(
            input.advance(clock.at(1200)),
            Some(GrabAction::ReportTooHeavy)
        );
    }

    #[test]
    fn a_hold_on_a_carriable_item_or_a_loose_body_reports_no_weight() {
        let clock = Clock::new();
        for aim in [ITEM, AimTarget::Grabbable(BODY)] {
            let mut input = pressed_on(aim, &clock);
            assert_eq!(input.advance(clock.at(200)), GRAB_BODY, "{aim:?}");
            assert_eq!(input.advance(clock.at(2000)), None, "{aim:?}");
        }
    }

    #[test]
    fn an_item_that_cannot_be_carried_is_not_picked_up_when_the_aim_moved_off_it() {
        let clock = Clock::new();
        for aimed in [AimTarget::Nothing, OTHER_ITEM] {
            let mut input = pressed_on(HEAVY_ITEM, &clock);
            assert_eq!(
                input.activate_released(false, clock.at(2000), aimed),
                None,
                "{aimed:?}"
            );
        }
    }

    #[test]
    fn a_menu_opening_while_e_is_held_on_an_item_leaves_the_release_without_effect() {
        let clock = Clock::new();
        for aim in [ITEM, HEAVY_ITEM, FIXED_ITEM] {
            let mut input = pressed_on(aim, &clock);
            input.release_keys();
            assert_eq!(input.advance(clock.at(1000)), None);
            assert_eq!(input.activate_released(false, clock.at(1000), aim), None);
        }
    }

    #[test]
    fn a_tap_on_a_loose_body_never_picks_it_up_even_when_an_item_is_aimed_at() {
        let clock = Clock::new();
        let mut input = pressed_on(AimTarget::Grabbable(BODY), &clock);
        assert_eq!(input.activate_released(false, clock.at(100), ITEM), None);
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
        assert_eq!(
            holding.activate_released(false, clock.at(1000), AimTarget::Nothing),
            None
        );

        let mut carrying = carrying_input(&clock);
        carrying.throw_pressed(true, clock.at(1000));
        carrying.activate_pressed(AimTarget::Nothing, true, clock.at(1000));
        carrying.release_keys();
        assert_eq!(carrying.throw_released(true, clock.at(1500)), None);
        assert_eq!(
            carrying.activate_released(true, clock.at(1500), AimTarget::Nothing),
            None
        );
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
