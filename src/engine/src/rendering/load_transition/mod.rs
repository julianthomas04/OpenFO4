//! The loading screen of every switch of cell or worldspace, not only the first load: which
//! screen a transition shows, what starts when it begins, and the large screen's model, tip and
//! drag input.

mod art_loader;
mod drag;
mod history;
mod host;

pub(super) use drag::DragTracker;
pub(super) use history::{LoadHistory, spawn_report_title, take_if_accepted};
pub(in crate::rendering) use host::LoadScreenHost;

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

/// A transition decided before the streamer changes place, while the origin can still be read
/// from it; carried out once the streamer has succeeded.
pub(super) struct TransitionPlan {
    kind: LoadKind,
    destination: Place,
    cell: String,
    target_grid: (i32, i32),
}

/// The screen a classified transition shows; `None` shows no screen at all.
fn requested_screen(kind: LoadKind) -> Option<ScreenKind> {
    match kind {
        LoadKind::None => None,
        LoadKind::Small => Some(ScreenKind::Small),
        LoadKind::Large => Some(ScreenKind::Large),
    }
}

/// A large screen with nothing to show is the small one.
fn shown_screen(requested: ScreenKind, has_art: bool) -> ScreenKind {
    match (requested, has_art) {
        (ScreenKind::Large, true) => ScreenKind::Large,
        _ => ScreenKind::Small,
    }
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

    fn current_place(&self) -> Place {
        if self.streamer.in_interior() {
            Place::Interior
        } else {
            Place::Exterior {
                worldspace: self.streamer.worldspace.clone(),
            }
        }
    }

    /// Decides which screen the switch to `destination` shows. `cell` is the form key of the
    /// destination cell, empty for an exterior one; `target_grid` is where the player arrives.
    pub(super) fn plan_transition(
        &self,
        destination: Place,
        cell: &str,
        target_grid: (i32, i32),
        no_load_screen: bool,
    ) -> TransitionPlan {
        let origin = (!self.load_screens.history.next_is_initial()).then(|| self.current_place());
        TransitionPlan {
            kind: classify(origin.as_ref(), &destination, no_load_screen),
            destination,
            cell: cell.to_string(),
            target_grid,
        }
    }

    /// Starts the load the plan describes, once the streamer has changed place. A switch that
    /// shows no screen changes nothing but the load's report.
    pub(super) fn begin_transition(&mut self, plan: &TransitionPlan) {
        let origin = self.load_screens.history.begin();
        let Some(requested) = requested_screen(plan.kind) else {
            return;
        };
        let chosen = (requested == ScreenKind::Large)
            .then(|| self.load_screens.host.choose(&plan.destination, &plan.cell))
            .flatten();
        let screen = shown_screen(requested, chosen.is_some());

        let action = self.flow.begin_transition(screen);
        self.dispatch(action);
        if origin == history::LoadOrigin::Initial {
            match screen {
                ScreenKind::Small => self.flow.fall_back_to_small(),
                ScreenKind::Large => self.dispatch(FlowAction::UnlockCursor),
            }
        }
        self.sim.set_fold_hold(true);
        self.load_screens.drag.release();
        self.load_spawn_cell = plan.target_grid;
        self.release_held_movement();
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
        let plan = self.plan_transition(destination, "", self.load_spawn_cell, false);
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

    #[test]
    fn a_classified_kind_maps_to_the_screen_it_asks_for() {
        assert_eq!(requested_screen(LoadKind::None), None);
        assert_eq!(requested_screen(LoadKind::Small), Some(ScreenKind::Small));
        assert_eq!(requested_screen(LoadKind::Large), Some(ScreenKind::Large));
    }

    #[test]
    fn a_large_screen_without_art_is_small_and_a_small_one_stays_small() {
        assert_eq!(shown_screen(ScreenKind::Large, true), ScreenKind::Large);
        assert_eq!(shown_screen(ScreenKind::Large, false), ScreenKind::Small);
        assert_eq!(shown_screen(ScreenKind::Small, true), ScreenKind::Small);
        assert_eq!(shown_screen(ScreenKind::Small, false), ScreenKind::Small);
    }
}
