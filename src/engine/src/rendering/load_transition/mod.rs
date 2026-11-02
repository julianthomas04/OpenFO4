//! The loading screen of every switch of cell or worldspace, not only the first load: which
//! screen a transition shows, what starts when it begins, and the large screen's model, tip and
//! drag input.

mod art_loader;
mod drag;
mod history;
mod host;

pub(super) use drag::DragTracker;
pub(super) use history::{LoadHistory, spawn_report_title};
pub(in crate::rendering) use host::LoadScreenHost;

use history::{LoadOrigin, take_if_accepted};
use host::ArtStatus;

use super::State;
use crate::game_flow::{FlowAction, Phase, ScreenKind};
use crate::loading_screen::{LoadKind, Place, classify};

/// Everything the loading screens keep between frames.
pub(super) struct LoadScreens {
    pub host: LoadScreenHost,
    pub drag: DragTracker,
    pub history: LoadHistory,
}

/// Where a switch of place arrives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Destination {
    pub place: Place,
    /// The form key of the destination cell, empty for an exterior one.
    pub cell: String,
    /// The grid cell the player arrives in.
    pub grid: (i32, i32),
    /// The door skips the loading screen.
    pub no_load_screen: bool,
}

/// The arrival of a debug picker's choice, which no door can skip.
impl Destination {
    pub(super) fn picked(place: Place, cell: &str, grid: (i32, i32)) -> Self {
        Self {
            place,
            cell: cell.to_string(),
            grid,
            no_load_screen: false,
        }
    }
}

/// A transition decided before the streamer changes place, while the origin can still be read
/// from it; carried out once the streamer has succeeded.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct TransitionPlan {
    kind: LoadKind,
    destination: Place,
    cell: String,
    target_grid: (i32, i32),
}

/// Classifies the switch from `origin` (`None` for the session's first load) to `destination`.
fn plan_for(origin: Option<&Place>, destination: Destination) -> TransitionPlan {
    TransitionPlan {
        kind: classify(origin, &destination.place, destination.no_load_screen),
        destination: destination.place,
        cell: destination.cell,
        target_grid: destination.grid,
    }
}

/// What beginning a transition does to the game, as a pure decision.
#[derive(Debug, PartialEq, Eq)]
struct TransitionEffects {
    screen: ScreenKind,
    /// The load's own screen replaces the large one that has nothing to show.
    fall_back_to_small: bool,
    unlock_cursor: bool,
    hold_physics_fold: bool,
    release_movement_keys: bool,
    set_spawn_cell: bool,
}

/// Whether the plan's screen has a model to choose for it.
fn wants_art(kind: LoadKind) -> bool {
    kind == LoadKind::Large
}

/// The effects of a transition of `kind`; none at all when the door shows no screen. A large
/// screen with no `art_available` is the small one.
fn transition_effects(
    kind: LoadKind,
    origin: LoadOrigin,
    art_available: bool,
) -> Option<TransitionEffects> {
    let screen = match kind {
        LoadKind::None => return None,
        LoadKind::Large if art_available => ScreenKind::Large,
        LoadKind::Small | LoadKind::Large => ScreenKind::Small,
    };
    Some(TransitionEffects {
        screen,
        fall_back_to_small: origin == LoadOrigin::Initial && screen == ScreenKind::Small,
        unlock_cursor: screen == ScreenKind::Large,
        hold_physics_fold: true,
        release_movement_keys: true,
        set_spawn_cell: true,
    })
}

impl State {
    /// Whether a switch of place may start now: from play, or as the session's first load.
    pub(super) fn accepts_transition(&self) -> bool {
        match self.flow.phase() {
            Phase::InGame => true,
            Phase::Loading => self.load_screens.history.next_is_initial(),
            Phase::MainMenu | Phase::SpawnSelect | Phase::Revealing => false,
        }
    }

    /// Takes the pending request in the slot `select` names if a switch of place may start now;
    /// a request meanwhile is dropped with a warning.
    pub(super) fn take_pending_transition<T>(
        &mut self,
        select: fn(&mut State) -> &mut Option<T>,
        request: &str,
    ) -> Option<T> {
        let accepting = self.accepts_transition();
        take_if_accepted(select(self), accepting, request)
    }

    fn current_place(&self) -> Place {
        if self.streamer.in_interior() {
            Place::Interior
        } else {
            Place::Exterior {
                worldspace: self.streamer.worldspace.clone(),
            }
        }
    }

    /// Decides which screen the switch to `destination` shows.
    pub(super) fn plan_transition(&self, destination: Destination) -> TransitionPlan {
        let origin = (!self.load_screens.history.next_is_initial()).then(|| self.current_place());
        plan_for(origin.as_ref(), destination)
    }

    /// Starts the load the plan describes, once the streamer has changed place. A switch that
    /// shows no screen changes nothing but the load's report.
    pub(super) fn begin_transition(&mut self, plan: &TransitionPlan) {
        let origin = self.load_screens.history.begin();
        let chosen = wants_art(plan.kind)
            .then(|| self.load_screens.host.choose(&plan.destination, &plan.cell))
            .flatten();
        let Some(effects) = transition_effects(plan.kind, origin, chosen.is_some()) else {
            return;
        };

        self.flow.begin_transition(effects.screen);
        if effects.fall_back_to_small {
            self.flow.fall_back_to_small();
        }
        if effects.unlock_cursor {
            self.dispatch(FlowAction::UnlockCursor);
        }
        if effects.hold_physics_fold {
            self.sim.set_fold_hold(true);
        }
        self.load_screens.drag.release();
        if effects.set_spawn_cell {
            self.load_spawn_cell = plan.target_grid;
        }
        if effects.release_movement_keys {
            self.release_held_movement();
        }
        self.show_chosen_screen(chosen);
    }

    fn show_chosen_screen(&mut self, chosen: Option<host::ChosenScreen>) {
        self.load_screens.host.clear();
        let Some(chosen) = chosen else {
            return;
        };
        let label = chosen.label.clone();
        if let Err(error) = self.load_screens.host.show(chosen) {
            tracing::error!("loading screen {label} falls back to the small screen: {error}");
            self.flow.fall_back_to_small();
        }
    }

    /// The first load of a session that no switch of place began, such as a direct start into
    /// the default worldspace, still gets its screen.
    pub(super) fn begin_unstarted_initial_load(&mut self) {
        if self.flow.phase() != Phase::Loading || !self.load_screens.history.next_is_initial() {
            return;
        }
        let destination = self.current_place();
        let plan = self.plan_transition(Destination {
            place: destination,
            cell: String::new(),
            grid: self.load_spawn_cell,
            no_load_screen: false,
        });
        self.begin_transition(&plan);
    }

    /// Follows the model's load, and takes the model down once the load screen is. A model that
    /// fails to load turns the large screen into the small one.
    pub(super) fn sync_load_screen(&mut self) {
        if self.flow.phase() != Phase::Loading {
            if self.load_screens.host.is_active() {
                self.load_screens.host.clear();
            }
            self.load_screens.drag.release();
            return;
        }
        if let ArtStatus::Failed { label, error } = self.load_screens.host.poll() {
            tracing::error!("loading screen {label} falls back to the small screen: {error}");
            self.flow.fall_back_to_small();
            self.load_screens.drag.release();
        }
    }

    /// Whether the pointer and the wheel turn and zoom the model.
    pub(super) fn art_interactive(&self) -> bool {
        self.flow.overlay_layers().art && self.load_screens.host.has_art()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exterior(worldspace: &str) -> Place {
        Place::Exterior {
            worldspace: worldspace.to_string(),
        }
    }

    fn destination(place: Place, no_load_screen: bool) -> Destination {
        Destination {
            place,
            cell: "cell".to_string(),
            grid: (3, -4),
            no_load_screen,
        }
    }

    #[test]
    fn a_door_that_skips_its_screen_plans_no_screen_whatever_the_places() {
        let plan = plan_for(Some(&Place::Interior), destination(exterior("ws"), true));
        assert_eq!(plan.kind, LoadKind::None);
        let plan = plan_for(Some(&exterior("a")), destination(exterior("b"), true));
        assert_eq!(plan.kind, LoadKind::None);
    }

    #[test]
    fn an_ordinary_door_plans_the_screen_of_its_places() {
        let plan = plan_for(Some(&exterior("a")), destination(exterior("b"), false));
        assert_eq!(plan.kind, LoadKind::Large);
        let plan = plan_for(Some(&Place::Interior), destination(exterior("b"), false));
        assert_eq!(plan.kind, LoadKind::Small);
    }

    #[test]
    fn the_first_load_plans_a_large_screen_even_through_a_door_that_skips_its_screen() {
        let plan = plan_for(None, destination(Place::Interior, true));
        assert_eq!(plan.kind, LoadKind::Large);
    }

    #[test]
    fn the_plan_keeps_where_the_player_arrives() {
        let plan = plan_for(None, destination(Place::Interior, false));
        assert_eq!(plan.destination, Place::Interior);
        assert_eq!(plan.cell, "cell");
        assert_eq!(plan.target_grid, (3, -4));
    }

    #[test]
    fn a_transition_that_shows_no_screen_has_no_effects_at_all() {
        for origin in [LoadOrigin::Initial, LoadOrigin::Transition] {
            for art in [false, true] {
                assert_eq!(transition_effects(LoadKind::None, origin, art), None);
            }
        }
    }

    #[test]
    fn a_small_transition_holds_the_fold_and_resets_input_but_keeps_the_cursor() {
        let effects = transition_effects(LoadKind::Small, LoadOrigin::Transition, true).unwrap();
        assert_eq!(
            effects,
            TransitionEffects {
                screen: ScreenKind::Small,
                fall_back_to_small: false,
                unlock_cursor: false,
                hold_physics_fold: true,
                release_movement_keys: true,
                set_spawn_cell: true,
            }
        );
    }

    #[test]
    fn a_large_transition_with_art_also_frees_the_cursor() {
        let effects = transition_effects(LoadKind::Large, LoadOrigin::Transition, true).unwrap();
        assert_eq!(effects.screen, ScreenKind::Large);
        assert!(effects.unlock_cursor && effects.hold_physics_fold);
        assert!(!effects.fall_back_to_small);
    }

    #[test]
    fn a_large_transition_without_art_is_a_small_one() {
        let effects = transition_effects(LoadKind::Large, LoadOrigin::Transition, false).unwrap();
        assert_eq!(effects.screen, ScreenKind::Small);
        assert!(!effects.unlock_cursor && effects.hold_physics_fold);
    }

    #[test]
    fn the_first_load_without_art_tells_the_flow_to_fall_back() {
        let effects = transition_effects(LoadKind::Large, LoadOrigin::Initial, false).unwrap();
        assert!(effects.fall_back_to_small);
        let with_art = transition_effects(LoadKind::Large, LoadOrigin::Initial, true).unwrap();
        assert!(!with_art.fall_back_to_small && with_art.unlock_cursor);
    }

    #[test]
    fn only_a_large_transition_wants_art() {
        assert!(wants_art(LoadKind::Large));
        assert!(!wants_art(LoadKind::Small));
        assert!(!wants_art(LoadKind::None));
    }
}
