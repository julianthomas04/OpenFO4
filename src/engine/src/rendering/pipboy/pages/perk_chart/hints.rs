//! The hint bar of the chart: which hints it shows, and the shared hint bar drawing them at the
//! place and over the backdrop the menu gives its own bar.

use super::layout::{ChartLayout, HintLabels};
use super::state::ChartState;
use crate::rendering::pipboy::tab::{ButtonHint, TabContext};
use crate::sim::pipboy::PipBoyModel;

/// Stand-in: the character the translation table has the unspent points stand in for. The game
/// substitutes it in native code, so no movie defines it.
const COUNT_PLACEHOLDER: char = '#';

/// The hints for the current selection: accept (or choose with the points left, hidden without
/// points), close and the rank keys.
pub fn hints(labels: &HintLabels, state: &ChartState, model: &PipBoyModel) -> Vec<ButtonHint> {
    let [accept, close, previous, next] = labels.keys;
    let selection = state.selection(model);
    let points = model.character.progression.unspent_perk_points;
    let confirming = state.is_confirming();

    let mut hints = Vec::with_capacity(4);
    if confirming {
        hints.push(ButtonHint::new(accept, labels.accept.clone()));
    } else if points > 0 {
        let label = labels
            .choose_template
            .replace(COUNT_PLACEHOLDER, &points.to_string());
        let can_choose = selection
            .as_ref()
            .is_some_and(|selection| state.accept_hint_enabled(selection));
        hints.push(ButtonHint::new(accept, label).enabled(can_choose));
    }
    hints.push(ButtonHint::new(close, labels.close.clone()));
    hints.push(
        ButtonHint::new(previous, labels.previous.clone()).enabled(
            selection
                .as_ref()
                .is_some_and(|selection| state.can_view_previous(selection)),
        ),
    );
    hints.push(
        ButtonHint::new(next, labels.next.clone()).enabled(
            selection
                .as_ref()
                .is_some_and(|selection| state.can_view_next(selection)),
        ),
    );
    hints
}

pub fn draw_bar(cx: &TabContext<'_>, layout: &ChartLayout, hints: &[ButtonHint]) {
    layout.hint_bar.paint_in_movie(
        cx.painter,
        cx.layout,
        &layout.hint_placement,
        cx.tint,
        hints,
        cx.seconds_open,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::pipboy::pages::perk_chart::state::tests::{cells, constants, model};
    use crate::rendering::pipboy::pages::perk_chart_layout::Cell;
    use crate::rendering::pipboy::tab::{HintKey, NavInput};
    use crate::sim::pipboy::PipBoyAction;

    fn labels() -> HintLabels {
        HintLabels {
            accept: "accept".to_owned(),
            close: "close".to_owned(),
            previous: "previous".to_owned(),
            next: "next".to_owned(),
            keys: [HintKey::Enter, HintKey::Tab, HintKey::Ctrl, HintKey::Alt],
            choose_template: "choose (#)".to_owned(),
        }
    }

    fn state_on(model: &PipBoyModel, cell: Cell) -> ChartState {
        let constants = constants();
        let cells = cells();
        let mut state = ChartState::new(&constants);
        let centre = state.view.to_stage(cells[cell.index()].border.center());
        let mut actions: Vec<PipBoyAction> = Vec::new();
        let mut requests = Vec::new();
        let mut env = crate::rendering::pipboy::pages::perk_chart::state::InputEnv {
            constants: &constants,
            cells: &cells,
            error_seconds: 2.0,
            model,
            now: 0.0,
            actions: &mut actions,
            requests: &mut requests,
        };
        state.handle_input(NavInput::PointerMoved(centre), &mut env);
        state
    }

    fn find(hints: &[ButtonHint], key: HintKey) -> Option<&ButtonHint> {
        hints.iter().find(|hint| hint.key == key)
    }

    #[test]
    fn the_bar_shows_choose_with_the_points_close_and_the_rank_keys() {
        let model = model();
        let state = state_on(&model, Cell { column: 0, row: 1 });

        let hints = hints(&labels(), &state, &model);

        let keys: Vec<_> = hints.iter().map(|hint| hint.key).collect();
        assert_eq!(
            keys,
            [HintKey::Enter, HintKey::Tab, HintKey::Ctrl, HintKey::Alt]
        );
        assert_eq!(find(&hints, HintKey::Enter).unwrap().label, "choose (1)");
        assert!(find(&hints, HintKey::Enter).unwrap().enabled);
        assert_eq!(find(&hints, HintKey::Tab).unwrap().label, "close");
    }

    #[test]
    fn without_points_the_choose_hint_is_hidden() {
        let mut model = model();
        model.character.progression.unspent_perk_points = 0;
        let state = state_on(&model, Cell { column: 0, row: 1 });

        let hints = hints(&labels(), &state, &model);

        assert!(find(&hints, HintKey::Enter).is_none());
        assert!(find(&hints, HintKey::Tab).is_some());
    }

    #[test]
    fn choose_is_disabled_with_nothing_selected_or_a_perk_that_cannot_be_taken() {
        let model = model();
        let nothing = ChartState::new(&constants());
        let locked = state_on(&model, Cell { column: 0, row: 2 });

        assert!(
            !find(&hints(&labels(), &nothing, &model), HintKey::Enter)
                .unwrap()
                .enabled
        );
        assert!(
            !find(&hints(&labels(), &locked, &model), HintKey::Enter)
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn the_rank_keys_follow_the_shown_rank() {
        let model = model();
        let mut state = state_on(&model, Cell { column: 0, row: 1 });
        let first = hints(&labels(), &state, &model);
        assert!(
            !find(&first, HintKey::Ctrl).unwrap().enabled,
            "rank one is the first rank"
        );
        assert!(find(&first, HintKey::Alt).unwrap().enabled);

        let constants = constants();
        let cells = cells();
        let mut actions = Vec::new();
        let mut requests = Vec::new();
        let mut env = crate::rendering::pipboy::pages::perk_chart::state::InputEnv {
            constants: &constants,
            cells: &cells,
            error_seconds: 2.0,
            model: &model,
            now: 0.0,
            actions: &mut actions,
            requests: &mut requests,
        };
        state.handle_input(NavInput::Hint(HintKey::Alt), &mut env);
        state.handle_input(NavInput::Hint(HintKey::Alt), &mut env);

        let last = hints(&labels(), &state, &model);
        assert!(find(&last, HintKey::Ctrl).unwrap().enabled);
        assert!(
            !find(&last, HintKey::Alt).unwrap().enabled,
            "rank three is the last rank"
        );
    }

    #[test]
    fn while_confirming_the_bar_offers_accept_and_close_only() {
        let model = model();
        let mut state = state_on(&model, Cell { column: 0, row: 1 });
        let constants = constants();
        let cells = cells();
        let mut actions = Vec::new();
        let mut requests = Vec::new();
        let mut env = crate::rendering::pipboy::pages::perk_chart::state::InputEnv {
            constants: &constants,
            cells: &cells,
            error_seconds: 2.0,
            model: &model,
            now: 0.0,
            actions: &mut actions,
            requests: &mut requests,
        };
        state.handle_input(NavInput::Hint(HintKey::Enter), &mut env);

        let hints = hints(&labels(), &state, &model);

        assert_eq!(find(&hints, HintKey::Enter).unwrap().label, "accept");
        assert!(!find(&hints, HintKey::Ctrl).unwrap().enabled);
        assert!(
            find(&hints, HintKey::Alt).unwrap().enabled,
            "the rank keys keep working while confirming"
        );
    }

    #[test]
    fn the_choose_hint_dims_while_another_rank_than_the_next_is_browsed() {
        let model = model();
        let mut state = state_on(&model, Cell { column: 0, row: 1 });
        let (constants, cells) = (constants(), cells());
        let (mut actions, mut requests) = (Vec::new(), Vec::new());
        let mut env = crate::rendering::pipboy::pages::perk_chart::state::InputEnv {
            constants: &constants,
            cells: &cells,
            error_seconds: 2.0,
            model: &model,
            now: 0.0,
            actions: &mut actions,
            requests: &mut requests,
        };
        state.handle_input(NavInput::Hint(HintKey::Alt), &mut env);

        let hints = hints(&labels(), &state, &model);

        assert!(!find(&hints, HintKey::Enter).unwrap().enabled);
    }
}
