//! Picks the large loading screen for a destination.

use crate::assets::db::conditions::Condition;
use crate::assets::db::load_screens::{LoadScreen, LoadScreenCatalog};
use crate::conditions::{ConditionContext, ConditionError, evaluate};
use crate::random::RandomSource;

#[derive(Debug, thiserror::Error)]
enum Ineligible {
    #[error("a condition names quest aliases, which need a quest system")]
    UsesAliases,
    #[error(transparent)]
    Condition(#[from] ConditionError),
}

/// The vanilla selection rule is not publicly documented; this policy is inferred. Screens that only
/// the main menu displays are dropped and every other screen is evaluated once against `ctx`. A
/// screen passing with conditions is preferred over one without, because the unconditional ones are
/// the generic fallback; `rng` picks uniformly within the preferred group. Weights come from the
/// conditions themselves (`GetRandomPercent`). A screen whose conditions cannot be evaluated is
/// never eligible and is reported once.
pub fn select<'a>(
    catalog: &'a LoadScreenCatalog,
    ctx: &dyn ConditionContext,
    rng: &mut dyn RandomSource,
) -> Option<&'a LoadScreen> {
    let mut conditioned = Vec::new();
    let mut unconditional = Vec::new();
    for screen in &catalog.screens {
        if screen.displays_in_main_menu {
            continue;
        }
        match passes(screen, ctx) {
            Ok(false) => {}
            Ok(true) if screen.conditions.is_empty() => unconditional.push(screen),
            Ok(true) => conditioned.push(screen),
            Err(reason) => report_ineligible(screen, &reason),
        }
    }
    let pool = if conditioned.is_empty() {
        unconditional
    } else {
        conditioned
    };
    if pool.is_empty() {
        return None;
    }
    Some(pool[rng.below(pool.len())])
}

fn passes(screen: &LoadScreen, ctx: &dyn ConditionContext) -> Result<bool, Ineligible> {
    if screen.conditions.iter().any(Condition::uses_aliases) {
        return Err(Ineligible::UsesAliases);
    }
    Ok(evaluate(&screen.conditions, ctx)?)
}

fn report_ineligible(screen: &LoadScreen, reason: &Ineligible) {
    crate::utils::warn_once(
        &format!("load-screen-ineligible:{}", screen.form_key),
        || {
            format!(
                "load screen {} ({}) is never shown: {reason}",
                screen.form_key,
                screen.label()
            )
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::db::conditions::{Comparand, CompareOp, Condition, ConditionParam};
    use crate::assets::db::load_screens::ModelTransform;
    use crate::conditions::test_support::{condition, record_param, with_params};
    use crate::conditions::{GET_GLOBAL_VALUE, GET_IN_WORLDSPACE, GET_RANDOM_PERCENT};
    use crate::loading_screen::classify::Place;
    use crate::loading_screen::context::LoadScreenContext;
    use crate::random::test_support::ScriptedRandom;

    const COMMONWEALTH: &str = "003C:Fallout4.esm";
    const CELL: &str = "0A01:Fallout4.esm";

    fn screen(key: &str, conditions: Vec<Condition>) -> LoadScreen {
        LoadScreen {
            form_key: key.to_string(),
            editor_id: Some(format!("LS_{key}")),
            tip: None,
            model_path: "a.nif".to_string(),
            displays_in_main_menu: false,
            no_rotation: false,
            transform: ModelTransform {
                position: [0.0; 3],
                rotation_radians: [0.0; 3],
                scale: 1.0,
                around_origin: false,
                zoom_min: 0.0,
                zoom_max: 1.0,
            },
            rotation_limits: None,
            zoom_limits: None,
            conditions,
        }
    }

    fn percent_at_most(limit: f32) -> Condition {
        condition(
            GET_RANDOM_PERCENT,
            CompareOp::LessOrEqual,
            Comparand::Value(limit),
        )
    }

    fn in_commonwealth(expected: f32) -> Condition {
        with_params(
            condition(
                GET_IN_WORLDSPACE,
                CompareOp::Equal,
                Comparand::Value(expected),
            ),
            record_param(COMMONWEALTH),
            ConditionParam::default(),
        )
    }

    fn catalog(screens: Vec<LoadScreen>) -> LoadScreenCatalog {
        LoadScreenCatalog {
            screens,
            ..LoadScreenCatalog::default()
        }
    }

    struct Outcome {
        chosen: Option<String>,
        pick_sizes: Vec<usize>,
    }

    /// Selects for the Commonwealth with scripted percent rolls and scripted picks.
    fn run(catalog: &LoadScreenCatalog, rolls: &[f32], picks: &[usize]) -> Outcome {
        let ctx = LoadScreenContext::new(
            catalog,
            &Place::Exterior {
                worldspace: COMMONWEALTH.to_string(),
            },
            CELL,
            ScriptedRandom::new(&[], rolls),
        );
        let mut picker = ScriptedRandom::new(picks, &[]);
        let chosen = select(catalog, &ctx, &mut picker).map(|s| s.form_key.clone());
        Outcome {
            chosen,
            pick_sizes: picker.asked_below().to_vec(),
        }
    }

    fn chosen(catalog: &LoadScreenCatalog, rolls: &[f32], picks: &[usize]) -> Option<String> {
        run(catalog, rolls, picks).chosen
    }

    #[test]
    fn a_catalog_without_screens_selects_nothing_and_asks_for_no_pick() {
        let outcome = run(&catalog(vec![]), &[], &[]);

        assert_eq!(outcome.chosen, None);
        assert_eq!(outcome.pick_sizes, Vec::<usize>::new());
    }

    #[test]
    fn the_percent_boundary_is_inclusive_for_at_most() {
        let data = catalog(vec![screen("A", vec![percent_at_most(25.0)])]);

        assert_eq!(chosen(&data, &[0.25], &[0]).as_deref(), Some("A"));
        assert_eq!(chosen(&data, &[0.26], &[0]), None);
    }

    #[test]
    fn a_fraction_of_a_percent_is_the_integer_zero_so_it_passes_only_at_zero() {
        let data = catalog(vec![screen("A", vec![percent_at_most(0.25)])]);

        assert_eq!(chosen(&data, &[0.0099], &[0]).as_deref(), Some("A"));
        assert_eq!(chosen(&data, &[0.0199], &[0]), None);
    }

    #[test]
    fn a_percent_of_zero_passes_an_at_most_zero_condition() {
        let data = catalog(vec![screen("A", vec![percent_at_most(0.0)])]);

        assert_eq!(chosen(&data, &[0.0], &[0]).as_deref(), Some("A"));
    }

    #[test]
    fn each_screen_rolls_its_own_percent_so_the_weights_apply_independently() {
        let data = catalog(vec![
            screen("RARE", vec![percent_at_most(25.0)]),
            screen("COMMON", vec![percent_at_most(50.0)]),
        ]);

        assert_eq!(chosen(&data, &[0.4, 0.4], &[0]).as_deref(), Some("COMMON"));
        assert_eq!(chosen(&data, &[0.1, 0.9], &[0]).as_deref(), Some("RARE"));
        assert_eq!(chosen(&data, &[0.9, 0.9], &[0]), None);
    }

    #[test]
    fn several_passing_screens_are_picked_uniformly_among_exactly_those() {
        let data = catalog(vec![
            screen("A", vec![in_commonwealth(1.0)]),
            screen("B", vec![in_commonwealth(0.0)]),
            screen("C", vec![in_commonwealth(1.0)]),
        ]);

        let outcome = run(&data, &[], &[1]);

        assert_eq!(outcome.chosen.as_deref(), Some("C"));
        assert_eq!(outcome.pick_sizes, vec![2]);
        assert_eq!(chosen(&data, &[], &[0]).as_deref(), Some("A"));
    }

    #[test]
    fn a_passing_conditioned_screen_beats_every_unconditional_one() {
        let data = catalog(vec![
            screen("TIP_A", vec![]),
            screen("PLACE", vec![in_commonwealth(1.0)]),
            screen("TIP_B", vec![]),
        ]);

        let outcome = run(&data, &[], &[0]);

        assert_eq!(outcome.chosen.as_deref(), Some("PLACE"));
        assert_eq!(outcome.pick_sizes, vec![1]);
    }

    #[test]
    fn unconditional_screens_are_the_fallback_when_no_conditioned_one_passes() {
        let data = catalog(vec![
            screen("TIP_A", vec![]),
            screen("ELSEWHERE", vec![in_commonwealth(0.0)]),
            screen("TIP_B", vec![]),
        ]);

        let outcome = run(&data, &[], &[1]);

        assert_eq!(outcome.chosen.as_deref(), Some("TIP_B"));
        assert_eq!(outcome.pick_sizes, vec![2]);
    }

    #[test]
    fn nothing_is_selected_when_no_screen_passes() {
        let data = catalog(vec![screen("ELSEWHERE", vec![in_commonwealth(0.0)])]);

        assert_eq!(chosen(&data, &[], &[]), None);
    }

    #[test]
    fn the_main_menu_screen_is_never_selected() {
        let mut menu = screen("MENU", vec![]);
        menu.displays_in_main_menu = true;
        let data = catalog(vec![menu]);

        assert_eq!(chosen(&data, &[], &[]), None);
    }

    #[test]
    fn a_screen_using_aliases_is_ineligible_even_when_its_conditions_would_pass() {
        let mut aliased = in_commonwealth(1.0);
        aliased.flags = 2;
        let data = catalog(vec![screen("ALIASED", vec![aliased])]);

        assert_eq!(chosen(&data, &[], &[]), None);
    }

    #[test]
    fn a_screen_with_an_unsupported_function_is_ineligible_and_does_not_stop_the_others() {
        let faction = with_params(
            condition(71, CompareOp::Equal, Comparand::Value(1.0)),
            record_param("FACTION"),
            ConditionParam::default(),
        );
        let data = catalog(vec![
            screen("FACTION", vec![faction]),
            screen("TIP", vec![]),
        ]);

        assert_eq!(chosen(&data, &[], &[0]).as_deref(), Some("TIP"));
    }

    #[test]
    fn a_screen_comparing_against_an_unknown_global_is_ineligible() {
        let unknown = with_params(
            condition(GET_GLOBAL_VALUE, CompareOp::Equal, Comparand::Value(1.0)),
            record_param("MISSING"),
            ConditionParam::default(),
        );
        let data = catalog(vec![screen("GLOBAL", vec![unknown])]);

        assert_eq!(chosen(&data, &[], &[]), None);
    }

    #[test]
    fn every_screen_is_evaluated_exactly_once() {
        let data = catalog(vec![
            screen("A", vec![percent_at_most(50.0)]),
            screen("B", vec![percent_at_most(50.0), in_commonwealth(1.0)]),
            screen("C", vec![]),
        ]);

        // Two rolls are scripted for the two random conditions; a third would panic.
        let outcome = run(&data, &[0.25, 0.75], &[0]);

        assert_eq!(outcome.chosen.as_deref(), Some("A"));
        assert_eq!(outcome.pick_sizes, vec![1]);
    }
}
