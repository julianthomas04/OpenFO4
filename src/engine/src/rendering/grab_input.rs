//! The carry controls, matched to measurements of vanilla Fallout 4: holding E on a loose body
//! grabs it and the grab latches; a later E tap drops it when that tap is let go; R charges a
//! throw while held and throws on release; the left and right mouse buttons spin the carried
//! body while held. Pure: events, frame starts and R's times in, [`GrabAction`]s and the spin out.

use std::time::{Duration, Instant};

use crate::sim::{EntityId, PickClass, SimCommand};

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

/// What a ref under the crosshair is to the pick: a door or animated object wins over a placed
/// item, so a door's activation never waits for the grab hold. `is_item` only runs for a ref that
/// is no activator.
pub(super) fn pick_class(is_activator: bool, is_item: impl FnOnce() -> bool) -> PickClass {
    if is_activator {
        PickClass::Activator
    } else if is_item() {
        PickClass::Item
    } else {
        PickClass::Solid
    }
}

impl AimTarget {
    /// What the crosshair rests on for a ref of `class`: a door or animated object, an item (and
    /// what its body allows), a loose body, or nothing. Only the closures of the branches taken
    /// run.
    pub(super) fn resolve(
        class: PickClass,
        entity: EntityId,
        is_grabbable: impl FnOnce(EntityId) -> bool,
        can_carry: impl FnOnce(EntityId) -> bool,
    ) -> Self {
        match class {
            PickClass::Activator => Self::Activatable,
            PickClass::Item => {
                let body = if can_carry(entity) {
                    ItemBody::Carriable
                } else if is_grabbable(entity) {
                    ItemBody::TooHeavy
                } else {
                    ItemBody::Fixed
                };
                Self::Item { entity, body }
            }
            PickClass::Solid if is_grabbable(entity) => Self::Grabbable(entity),
            PickClass::Solid => Self::Nothing,
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
    /// Down on a loose body or an item, counting towards the grab of that body or the pickup of
    /// that item.
    Holding {
        entity: EntityId,
        hold: FrameHold,
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

/// E's hold as the frames count it, measured on the original (2026-11-14): at 60 fps no carry
/// after 212 ms, some from 216 ms, every one from 228 ms. The hold counts from the first frame
/// after the press, and each later frame adds the time since the one before.
#[derive(Copy, Clone, PartialEq, Debug, Default)]
struct FrameHold {
    counting_since: Option<Instant>,
}

impl FrameHold {
    /// Counts the frame starting at `frame_start`: whether E, still down then, has been held for
    /// `grab_hold`.
    fn reached_at_frame(&mut self, frame_start: Instant, grab_hold: Duration) -> bool {
        let first_frame = *self.counting_since.get_or_insert(frame_start);
        frame_start.saturating_duration_since(first_frame) >= grab_hold
    }
}

#[derive(Debug)]
pub(super) struct GrabInput {
    /// How long E must stay down on a loose body, as the frames count it, before it is grabbed.
    grab_hold: Duration,
    activate: ActivateKey,
    /// When R went down, while a throw is charging.
    throw_since: Option<Instant>,
    left_held: bool,
    right_held: bool,
}

#[cfg(test)]
impl Default for GrabInput {
    fn default() -> Self {
        Self::new(super::pick_settings::ORIGINAL_PICK_DEFAULTS.grab_hold())
    }
}

impl GrabInput {
    pub fn new(grab_hold: Duration) -> Self {
        Self {
            grab_hold,
            activate: ActivateKey::default(),
            throw_since: None,
            left_held: false,
            right_held: false,
        }
    }

    /// E went down or up with `aim` under the crosshair. Key events reach this before the frame
    /// they precede counts the hold, so a release seen here was before that frame.
    pub fn activate_key(
        &mut self,
        is_pressed: bool,
        carrying: bool,
        aim: AimTarget,
    ) -> Option<GrabAction> {
        if is_pressed {
            self.activate_pressed(aim, carrying)
        } else {
            self.activate_released(carrying, aim)
        }
    }

    /// E went down. Every press starts afresh, so a key-up the platform never delivered cannot
    /// leave E dead. `carrying`: whether a body is carried right now.
    fn activate_pressed(&mut self, aim: AimTarget, carrying: bool) -> Option<GrabAction> {
        if carrying {
            self.activate = ActivateKey::Dropping;
            return None;
        }
        self.activate = ActivateKey::Spent;
        match aim {
            AimTarget::Activatable => Some(GrabAction::Activate),
            AimTarget::Grabbable(entity) => self.hold(entity, HeldTarget::Body),
            AimTarget::Item { entity, body } => {
                let target = match body {
                    ItemBody::Carriable => HeldTarget::GrabbableItem,
                    ItemBody::TooHeavy => HeldTarget::HeavyItem,
                    ItemBody::Fixed => HeldTarget::FixedItem,
                };
                self.hold(entity, target)
            }
            AimTarget::Nothing => None,
        }
    }

    fn hold(&mut self, entity: EntityId, target: HeldTarget) -> Option<GrabAction> {
        self.activate = ActivateKey::Holding {
            entity,
            hold: FrameHold::default(),
            target,
        };
        None
    }

    /// E was let go with `aimed` under the crosshair, before any frame found its hold at the grab
    /// hold, however long the frames took: a press on a loose body does nothing, a press on an
    /// item picks it up, provided the crosshair still rests on that item.
    fn activate_released(&mut self, carrying: bool, aimed: AimTarget) -> Option<GrabAction> {
        match std::mem::take(&mut self.activate) {
            ActivateKey::Holding { entity, target, .. } => taken_on_release(entity, target, aimed),
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

    /// Counts the frame starting at `frame_start` towards E's hold: once the hold reaches the
    /// grab hold, grabs the body or carriable item E went down on, or reports once that the item
    /// cannot be carried. The target is the one aimed at on the press; the sim validates it again.
    /// Called once per frame, after that frame's key events.
    pub fn advance(&mut self, frame_start: Instant) -> Option<GrabAction> {
        let ActivateKey::Holding {
            entity,
            mut hold,
            target,
        } = self.activate
        else {
            return None;
        };
        let reached = hold.reached_at_frame(frame_start, self.grab_hold);
        let (state, action) = match target {
            HeldTarget::Body | HeldTarget::GrabbableItem if reached => (
                ActivateKey::Spent,
                Some(GrabAction::Carry(CarryCommand::Grab(entity))),
            ),
            HeldTarget::HeavyItem if reached => (
                ActivateKey::Holding {
                    entity,
                    hold,
                    target: HeldTarget::ReportedHeavyItem,
                },
                Some(GrabAction::ReportTooHeavy),
            ),
            _ => (
                ActivateKey::Holding {
                    entity,
                    hold,
                    target,
                },
                None,
            ),
        };
        self.activate = state;
        action
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
        *self = Self::new(self.grab_hold);
    }
}

fn taken_on_release(entity: EntityId, target: HeldTarget, aimed: AimTarget) -> Option<GrabAction> {
    match target {
        HeldTarget::Body => None,
        HeldTarget::GrabbableItem
        | HeldTarget::HeavyItem
        | HeldTarget::ReportedHeavyItem
        | HeldTarget::FixedItem => aimed.is_item(entity).then_some(GrabAction::PickUp(entity)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::ini::GameIni;
    use crate::rendering::pick_settings::{ORIGINAL_PICK_DEFAULTS, PickSettings};

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

    /// The hold the rule tests run against: the rule does not depend on the measured number.
    const TEST_HOLD: Duration = Duration::from_millis(160);

    fn test_input() -> GrabInput {
        GrabInput::new(TEST_HOLD)
    }

    const GRAB_BODY: Option<GrabAction> = Some(GrabAction::Carry(CarryCommand::Grab(BODY)));
    const GRAB_OTHER_BODY: Option<GrabAction> =
        Some(GrabAction::Carry(CarryCommand::Grab(OTHER_BODY)));

    const DROP: Option<GrabAction> = Some(GrabAction::Carry(CarryCommand::Drop));
    const REPORT: Option<GrabAction> = Some(GrabAction::ReportTooHeavy);

    /// An input whose E went down on `aim` just before a frame that started at 0 ms.
    fn pressed_on(aim: AimTarget, clock: &Clock) -> GrabInput {
        let mut input = test_input();
        assert_eq!(input.activate_key(true, false, aim), None);
        assert_eq!(input.advance(clock.at(0)), None);
        input
    }

    /// An input that has grabbed `BODY` and had E let go, at 300 ms.
    fn carrying_input(clock: &Clock) -> GrabInput {
        let mut input = pressed_on(AimTarget::Grabbable(BODY), clock);
        assert_eq!(input.advance(clock.at(200)), GRAB_BODY);
        assert_eq!(input.activate_key(false, true, AimTarget::Nothing), None);
        input
    }

    #[test]
    fn a_press_on_a_loose_body_let_go_before_the_grab_hold_does_nothing() {
        let clock = Clock::new();
        let mut input = pressed_on(AimTarget::Grabbable(BODY), &clock);
        assert_eq!(input.advance(clock.at(100)), None);
        assert_eq!(input.advance(clock.at(150)), None);
        assert_eq!(input.activate_key(false, false, AimTarget::Nothing), None);
    }

    #[test]
    fn holding_e_on_a_loose_body_grabs_it_once_on_the_first_frame_at_the_grab_hold() {
        let clock = Clock::new();
        let mut input = pressed_on(AimTarget::Grabbable(BODY), &clock);
        assert_eq!(input.advance(clock.at(159)), None);
        assert_eq!(input.advance(clock.at(160)), GRAB_BODY);
        assert_eq!(input.advance(clock.at(500)), None);
        assert_eq!(input.activate_key(false, false, AimTarget::Nothing), None);
    }

    #[test]
    fn the_time_before_the_first_frame_after_the_press_does_not_count_towards_the_grab_hold() {
        let clock = Clock::new();
        let mut input = test_input();
        input.activate_key(true, false, AimTarget::Grabbable(BODY));
        assert_eq!(input.advance(clock.at(250)), None);
        assert_eq!(input.advance(clock.at(409)), None);
        assert_eq!(input.advance(clock.at(410)), GRAB_BODY);
    }

    #[test]
    fn a_release_before_any_frame_counted_the_hold_is_a_tap_however_late_it_comes() {
        for (aim, taken) in [(ITEM, PICK_UP), (AimTarget::Grabbable(BODY), None)] {
            let mut input = test_input();
            input.activate_key(true, false, aim);
            assert_eq!(input.activate_key(false, false, aim), taken, "{aim:?}");
        }
    }

    #[test]
    fn releasing_e_after_the_grab_keeps_the_body_held() {
        let clock = Clock::new();
        let mut input = pressed_on(AimTarget::Grabbable(BODY), &clock);
        input.advance(clock.at(200));
        assert_eq!(input.activate_key(false, true, AimTarget::Nothing), None);
    }

    #[test]
    fn an_e_tap_while_carrying_drops_the_body_on_its_release_and_not_on_its_press() {
        let clock = Clock::new();
        let mut input = carrying_input(&clock);
        assert_eq!(
            input.activate_key(true, true, AimTarget::Grabbable(OTHER_BODY)),
            None
        );
        assert_eq!(input.advance(clock.at(1400)), None);
        assert_eq!(input.activate_key(false, true, AimTarget::Nothing), DROP);
    }

    #[test]
    fn an_e_press_while_carrying_ignores_a_door_under_the_crosshair() {
        let clock = Clock::new();
        let mut input = carrying_input(&clock);
        assert_eq!(input.activate_key(true, true, AimTarget::Activatable), None);
        assert_eq!(input.activate_key(false, true, AimTarget::Nothing), DROP);
    }

    #[test]
    fn a_drop_tap_whose_body_was_let_go_meanwhile_does_nothing_on_release() {
        let clock = Clock::new();
        let mut input = carrying_input(&clock);
        input.activate_key(true, true, AimTarget::Nothing);
        assert_eq!(input.activate_key(false, false, AimTarget::Nothing), None);
    }

    #[test]
    fn a_door_press_activates_at_once_and_its_release_does_nothing() {
        let clock = Clock::new();
        let mut input = test_input();
        assert_eq!(
            input.activate_key(true, false, AimTarget::Activatable),
            Some(GrabAction::Activate)
        );
        assert_eq!(input.advance(clock.at(0)), None);
        assert_eq!(input.advance(clock.at(1000)), None);
        assert_eq!(input.activate_key(false, false, AimTarget::Nothing), None);
    }

    #[test]
    fn a_press_on_nothing_does_nothing() {
        let clock = Clock::new();
        let mut input = pressed_on(AimTarget::Nothing, &clock);
        assert_eq!(input.advance(clock.at(1000)), None);
        assert_eq!(input.activate_key(false, false, AimTarget::Nothing), None);
    }

    #[test]
    fn a_press_after_a_lost_key_up_is_a_fresh_press() {
        let clock = Clock::new();
        let mut door = test_input();
        door.activate_key(true, false, AimTarget::Grabbable(BODY));
        assert_eq!(
            door.activate_key(true, false, AimTarget::Activatable),
            Some(GrabAction::Activate)
        );

        let mut body = test_input();
        body.activate_key(true, false, AimTarget::Activatable);
        body.activate_key(true, false, AimTarget::Grabbable(OTHER_BODY));
        assert_eq!(body.advance(clock.at(1000)), None);
        assert_eq!(body.advance(clock.at(1159)), None);
        assert_eq!(body.advance(clock.at(1160)), GRAB_OTHER_BODY);
    }

    const NEVER: fn(EntityId) -> bool = |_| panic!("a closure of a branch not taken ran");

    #[test]
    fn a_door_wins_over_an_item_and_an_item_over_a_grabbable_body() {
        assert_eq!(
            AimTarget::resolve(PickClass::Activator, BODY, NEVER, NEVER),
            AimTarget::Activatable
        );
        assert_eq!(
            AimTarget::resolve(PickClass::Item, BODY, NEVER, |_| true),
            ITEM
        );
        assert_eq!(
            AimTarget::resolve(PickClass::Solid, BODY, |entity| entity == BODY, NEVER),
            AimTarget::Grabbable(BODY)
        );
        assert_eq!(
            AimTarget::resolve(PickClass::Solid, BODY, |_| false, NEVER),
            AimTarget::Nothing
        );
    }

    #[test]
    fn a_ref_that_is_both_a_door_and_an_item_is_a_door_and_neither_is_solid() {
        assert_eq!(pick_class(true, || true), PickClass::Activator);
        assert_eq!(
            pick_class(true, || panic!("the item test ran for an activator")),
            PickClass::Activator
        );
        assert_eq!(pick_class(false, || true), PickClass::Item);
        assert_eq!(pick_class(false, || false), PickClass::Solid);
    }

    #[test]
    fn an_item_is_too_heavy_only_with_a_movable_body_and_fixed_without_one() {
        assert_eq!(
            AimTarget::resolve(PickClass::Item, BODY, |_| true, |_| false),
            HEAVY_ITEM
        );
        assert_eq!(
            AimTarget::resolve(PickClass::Item, BODY, |_| false, |_| false),
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

    #[test]
    fn a_tap_on_a_carriable_item_picks_it_up_once_on_release() {
        let clock = Clock::new();
        let mut input = pressed_on(ITEM, &clock);
        assert_eq!(input.advance(clock.at(159)), None);
        assert_eq!(input.activate_key(false, false, ITEM), PICK_UP);
        assert_eq!(input.activate_key(false, false, ITEM), None);
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
            assert_eq!(input.activate_key(false, false, aimed), None, "{aimed:?}");
        }
    }

    #[test]
    fn a_tap_on_an_item_that_became_too_heavy_to_carry_still_picks_it_up() {
        let clock = Clock::new();
        let mut input = pressed_on(ITEM, &clock);
        assert_eq!(input.activate_key(false, false, HEAVY_ITEM), PICK_UP);
    }

    #[test]
    fn holding_a_carriable_item_grabs_it_once_and_a_later_release_does_nothing() {
        let clock = Clock::new();
        let mut input = pressed_on(ITEM, &clock);
        assert_eq!(input.advance(clock.at(159)), None);
        assert_eq!(input.advance(clock.at(160)), GRAB_BODY);
        assert_eq!(input.advance(clock.at(500)), None);
        assert_eq!(input.activate_key(false, true, ITEM), None);
    }

    #[test]
    fn an_item_that_cannot_be_carried_is_picked_up_on_release_whatever_the_duration() {
        let clock = Clock::new();
        for released_after in [10, 159, 160, 161, 5000] {
            let mut input = pressed_on(HEAVY_ITEM, &clock);
            input.advance(clock.at(released_after));
            assert_eq!(
                input.activate_key(false, false, HEAVY_ITEM),
                PICK_UP,
                "{released_after}"
            );
        }
    }

    #[test]
    fn holding_e_on_an_item_that_cannot_be_carried_reports_it_once_at_the_grab_hold() {
        let clock = Clock::new();
        let mut input = pressed_on(HEAVY_ITEM, &clock);
        assert_eq!(input.advance(clock.at(159)), None);
        assert_eq!(input.advance(clock.at(160)), REPORT);
        assert_eq!(input.advance(clock.at(161)), None);
        assert_eq!(input.advance(clock.at(5000)), None);
    }

    #[test]
    fn an_item_that_cannot_be_carried_let_go_before_the_frame_at_the_grab_hold_is_taken_unreported()
    {
        let clock = Clock::new();
        let mut input = pressed_on(HEAVY_ITEM, &clock);
        assert_eq!(input.advance(clock.at(159)), None);
        assert_eq!(input.activate_key(false, false, HEAVY_ITEM), PICK_UP);
        assert_eq!(input.advance(clock.at(5000)), None);
    }

    #[test]
    fn an_item_without_a_movable_body_is_never_reported_or_grabbed_and_taken_on_release() {
        let clock = Clock::new();
        for released_after in [10, 160, 700, 5000] {
            let mut input = pressed_on(FIXED_ITEM, &clock);
            assert_eq!(
                input.advance(clock.at(released_after)),
                None,
                "{released_after}"
            );
            assert_eq!(
                input.activate_key(false, false, FIXED_ITEM),
                PICK_UP,
                "{released_after}"
            );
        }
    }

    #[test]
    fn an_item_without_a_movable_body_is_not_taken_when_the_aim_moved_off_it() {
        let clock = Clock::new();
        let mut input = pressed_on(FIXED_ITEM, &clock);
        assert_eq!(input.advance(clock.at(700)), None);
        assert_eq!(input.activate_key(false, false, AimTarget::Nothing), None);
    }

    #[test]
    fn the_item_is_still_picked_up_on_release_after_it_was_reported() {
        let clock = Clock::new();
        let mut input = pressed_on(HEAVY_ITEM, &clock);
        assert_eq!(input.advance(clock.at(200)), REPORT);
        assert_eq!(input.activate_key(false, false, HEAVY_ITEM), PICK_UP);
    }

    #[test]
    fn each_hold_on_an_item_that_cannot_be_carried_reports_again() {
        let clock = Clock::new();
        let mut input = pressed_on(HEAVY_ITEM, &clock);
        input.advance(clock.at(200));
        input.activate_key(false, false, HEAVY_ITEM);
        input.activate_key(true, false, HEAVY_ITEM);
        assert_eq!(input.advance(clock.at(1000)), None);
        assert_eq!(input.advance(clock.at(1200)), REPORT);
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
            input.advance(clock.at(2000));
            assert_eq!(input.activate_key(false, false, aimed), None, "{aimed:?}");
        }
    }

    #[test]
    fn a_menu_opening_while_e_is_held_on_an_item_leaves_the_release_without_effect() {
        let clock = Clock::new();
        for aim in [ITEM, HEAVY_ITEM, FIXED_ITEM] {
            let mut input = pressed_on(aim, &clock);
            input.release_keys();
            assert_eq!(input.advance(clock.at(1000)), None);
            assert_eq!(input.activate_key(false, false, aim), None);
        }
    }

    #[test]
    fn a_tap_on_a_loose_body_never_picks_it_up_even_when_an_item_is_aimed_at() {
        let clock = Clock::new();
        let mut input = pressed_on(AimTarget::Grabbable(BODY), &clock);
        assert_eq!(input.activate_key(false, false, ITEM), None);
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
        let mut input = test_input();
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
        let mut input = test_input();
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
        let mut input = test_input();
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
        input.activate_key(true, false, AimTarget::Grabbable(BODY));
        assert_eq!(input.advance(clock.at(2000)), None);
        assert_eq!(input.advance(clock.at(2200)), GRAB_BODY);
        assert_eq!(input.spin(true), 1);
    }

    #[test]
    fn releasing_the_keys_forgets_the_grab_hold_the_drop_tap_and_the_throw_charge() {
        let clock = Clock::new();
        let mut holding = pressed_on(AimTarget::Grabbable(BODY), &clock);
        holding.release_keys();
        assert_eq!(holding.advance(clock.at(1000)), None);
        assert_eq!(holding.activate_key(false, false, AimTarget::Nothing), None);

        let mut carrying = carrying_input(&clock);
        carrying.throw_pressed(true, clock.at(1000));
        carrying.activate_key(true, true, AimTarget::Nothing);
        carrying.release_keys();
        assert_eq!(carrying.throw_released(true, clock.at(1500)), None);
        assert_eq!(carrying.activate_key(false, true, AimTarget::Nothing), None);
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
    fn a_reset_keeps_the_grab_hold() {
        let clock = Clock::new();
        let mut input = GrabInput::new(Duration::from_millis(500));
        input.reset();
        input.activate_key(true, false, ITEM);
        assert_eq!(input.advance(clock.at(0)), None);
        assert_eq!(input.advance(clock.at(499)), None);
        assert_eq!(input.advance(clock.at(500)), GRAB_BODY);
    }

    /// Frames run back to back, the way the engine loop sees them: E's press and release reach
    /// the input, in order, before the first frame that starts at or after them, and that frame
    /// then counts towards the hold.
    mod frames {
        use super::*;

        const SEVERAL_FRAME_RATES_MS: [u64; 5] = [7, 16, 17, 33, 50];
        const HITCH_MS: u64 = 300;

        #[derive(Copy, Clone, PartialEq, Eq, Debug)]
        enum Outcome {
            /// Grabbed on the frame that started at this many ms.
            Grabbed(u64),
            Taken,
        }

        /// Frame starts every `period` ms from 0; the frame starting at `hitch_at`, if any, lasts
        /// `HITCH_MS` instead.
        fn frame_starts(period: u64, hitch_at: Option<u64>) -> impl Iterator<Item = u64> {
            std::iter::successors(Some(0), move |&start| {
                let duration = if Some(start) == hitch_at {
                    HITCH_MS
                } else {
                    period
                };
                Some(start + duration)
            })
            .take_while(|&start| start < 5000)
        }

        fn outcome(
            mut input: GrabInput,
            starts: impl Iterator<Item = u64>,
            pressed_at: u64,
            released_at: u64,
        ) -> Outcome {
            let clock = Clock::new();
            let mut pressed = false;
            for start in starts {
                if !pressed && pressed_at <= start {
                    pressed = true;
                    assert_eq!(input.activate_key(true, false, ITEM), None);
                }
                if pressed && released_at <= start {
                    let released = input.activate_key(false, false, ITEM);
                    assert_eq!(
                        released, PICK_UP,
                        "a release before the grab takes the item"
                    );
                    return Outcome::Taken;
                }
                let action = input.advance(clock.at(start));
                if action == GRAB_BODY {
                    return Outcome::Grabbed(start);
                }
                assert_eq!(action, None, "at {start} ms");
            }
            panic!("neither grabbed nor released within the frames");
        }

        /// The first frame start at least `hold` after the first frame start after `pressed_at`.
        fn deciding_frame(starts: impl Iterator<Item = u64>, pressed_at: u64, hold: u64) -> u64 {
            let mut after_the_press = starts.skip_while(|&start| start < pressed_at);
            let first = after_the_press.next().expect("a frame after the press");
            std::iter::once(first)
                .chain(after_the_press)
                .find(|&start| start - first >= hold)
                .expect("a frame at the hold")
        }

        fn ms(duration: Duration) -> u64 {
            u64::try_from(duration.as_millis()).expect("a hold of a few seconds")
        }

        #[test]
        fn held_through_grabs_on_the_first_frame_at_or_past_the_delay_at_several_frame_rates() {
            let hold = ms(ORIGINAL_PICK_DEFAULTS.grab_hold());
            for period in SEVERAL_FRAME_RATES_MS {
                for pressed_at in 1..=period {
                    let expected = deciding_frame(frame_starts(period, None), pressed_at, hold);
                    assert_eq!(
                        outcome(
                            GrabInput::default(),
                            frame_starts(period, None),
                            pressed_at,
                            expected + 1
                        ),
                        Outcome::Grabbed(expected),
                        "{period} ms frames, pressed at {pressed_at}"
                    );
                }
            }
        }

        #[test]
        fn a_release_before_the_deciding_frame_takes_the_item_however_long_the_hold_was() {
            let hold = ms(ORIGINAL_PICK_DEFAULTS.grab_hold());
            for period in SEVERAL_FRAME_RATES_MS {
                for pressed_at in 1..=period {
                    let deciding = deciding_frame(frame_starts(period, None), pressed_at, hold);
                    assert!(deciding - pressed_at >= hold);
                    assert_eq!(
                        outcome(
                            GrabInput::default(),
                            frame_starts(period, None),
                            pressed_at,
                            deciding
                        ),
                        Outcome::Taken,
                        "{period} ms frames, pressed at {pressed_at}"
                    );
                }
            }
        }

        /// A 300 ms frame right after the press: a release during it, long past the delay by its
        /// event time, is still a tap; a key held through it grabs on the frame after it.
        #[test]
        fn a_release_during_a_300_ms_hitch_frame_is_a_tap_and_a_hold_through_it_grabs_after_it() {
            let period = 17;
            let pressed_at = 5;
            let first_frame = period;
            let starts = || frame_starts(period, Some(first_frame));
            let after_the_hitch = first_frame + HITCH_MS;
            assert_eq!(
                outcome(
                    GrabInput::default(),
                    starts(),
                    pressed_at,
                    after_the_hitch - 1
                ),
                Outcome::Taken
            );
            assert_eq!(
                outcome(
                    GrabInput::default(),
                    starts(),
                    pressed_at,
                    after_the_hitch + 1
                ),
                Outcome::Grabbed(after_the_hitch)
            );
        }

        #[test]
        fn the_delay_is_the_one_the_pick_settings_carry() {
            let settings =
                PickSettings::from_ini(&GameIni::from_texts(["[Controls]\nfZKeyDelay=0.5\n"]))
                    .expect("a valid delay");
            assert_eq!(
                GrabInput::default().grab_hold,
                ORIGINAL_PICK_DEFAULTS.grab_hold()
            );
            let period = 16;
            let pressed_at = 3;
            for (hold, input) in [
                (ORIGINAL_PICK_DEFAULTS.grab_hold(), GrabInput::default()),
                (settings.grab_hold(), GrabInput::new(settings.grab_hold())),
            ] {
                let expected = deciding_frame(frame_starts(period, None), pressed_at, ms(hold));
                assert_eq!(
                    outcome(input, frame_starts(period, None), pressed_at, 5000),
                    Outcome::Grabbed(expected),
                    "{hold:?}"
                );
            }
        }
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
