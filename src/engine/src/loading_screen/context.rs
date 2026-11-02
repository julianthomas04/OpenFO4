//! The condition context for a loading screen: the player as if already at the destination.

use std::cell::RefCell;

use super::classify::Place;
use crate::assets::db::load_screens::LoadScreenCatalog;
use crate::conditions::{ConditionContext, FreshWorldContext, global_entries};
use crate::random::RandomSource;

/// The clock of a fresh game; loading screen conditions do not read it.
const FRESH_GAME_HOUR: f32 = 0.0;
const PERCENT_SCALE: f32 = 100.0;

/// Answers location, worldspace, cell and random conditions for the destination, and everything
/// else (quests, globals, the clock) like a game that has just started: there is no quest system.
pub struct LoadScreenContext<R> {
    fresh_world: FreshWorldContext,
    worldspace: Option<String>,
    cell: String,
    /// The location the destination cell names, then its parent, then that one's parent, and so on.
    locations: Vec<String>,
    rolls: RefCell<R>,
}

impl<R: RandomSource> LoadScreenContext<R> {
    /// `cell` is the form key of the destination cell. `rolls` feeds `GetRandomPercent`, one fresh
    /// roll per evaluation.
    pub fn new(catalog: &LoadScreenCatalog, destination: &Place, cell: &str, rolls: R) -> Self {
        let mut fresh_world = FreshWorldContext::new(
            FRESH_GAME_HOUR,
            global_entries(&catalog.globals, &catalog.global_form_keys),
        );
        fresh_world.set_interior(matches!(destination, Place::Interior));
        Self {
            fresh_world,
            worldspace: match destination {
                Place::Interior => None,
                Place::Exterior { worldspace } => Some(worldspace.clone()),
            },
            cell: cell.to_string(),
            locations: location_chain(catalog, cell),
            rolls: RefCell::new(rolls),
        }
    }
}

fn location_chain(catalog: &LoadScreenCatalog, cell: &str) -> Vec<String> {
    let mut chain: Vec<String> = Vec::new();
    let mut next = catalog.cell_locations.get(cell);
    while let Some(location) = next {
        if chain.contains(location) {
            crate::utils::warn_once(&format!("location-cycle:{location}"), || {
                format!("location {location} is its own ancestor; the chain stops at the cycle")
            });
            break;
        }
        chain.push(location.clone());
        next = catalog.location_parents.get(location);
    }
    chain
}

fn same_key(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

impl<R: RandomSource> ConditionContext for LoadScreenContext<R> {
    fn game_hour(&self) -> f32 {
        self.fresh_world.game_hour()
    }

    fn global(&self, key: &str) -> Option<f32> {
        self.fresh_world.global(key)
    }

    fn quest_running(&self, quest: &str) -> bool {
        self.fresh_world.quest_running(quest)
    }

    fn quest_stage(&self, quest: &str) -> i32 {
        self.fresh_world.quest_stage(quest)
    }

    fn stage_done(&self, quest: &str, stage: i32) -> bool {
        self.fresh_world.stage_done(quest, stage)
    }

    fn vm_quest_variable(&self, quest: &str, variable: &str) -> f32 {
        self.fresh_world.vm_quest_variable(quest, variable)
    }

    fn is_first_person(&self) -> bool {
        self.fresh_world.is_first_person()
    }

    fn has_silent_running_perk(&self) -> bool {
        self.fresh_world.has_silent_running_perk()
    }

    fn is_in_interior(&self) -> Option<bool> {
        self.fresh_world.is_in_interior()
    }

    fn quest_completed(&self, quest: &str) -> Option<bool> {
        self.fresh_world.quest_completed(quest)
    }

    fn random_percent(&self) -> Option<f32> {
        Some(self.rolls.borrow_mut().unit() * PERCENT_SCALE)
    }

    fn in_worldspace(&self, worldspace: &str) -> Option<bool> {
        Some(
            self.worldspace
                .as_deref()
                .is_some_and(|current| same_key(current, worldspace)),
        )
    }

    fn in_cell(&self, cell: &str) -> Option<bool> {
        Some(same_key(&self.cell, cell))
    }

    fn in_current_location(&self, location: &str) -> Option<bool> {
        Some(self.locations.iter().any(|own| same_key(own, location)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::db::conditions::{Comparand, CompareOp, Condition, ConditionParam};
    use crate::audio::test_support::condition::{
        condition, number_param, record_param, text_param, with_params,
    };
    use crate::conditions::{
        ConditionError, GET_IN_CELL, GET_IN_CURRENT_LOCATION, GET_IN_WORLDSPACE,
        GET_QUEST_COMPLETED, GET_QUEST_RUNNING, GET_RANDOM_PERCENT, GET_STAGE, GET_STAGE_DONE,
        GET_VM_QUEST_VARIABLE, IS_IN_INTERIOR, evaluate,
    };
    use crate::loading_screen::test_support::ScriptedRandom;

    const COMMONWEALTH: &str = "003C:Fallout4.esm";
    const SANCTUARY_CELL: &str = "0A01:Fallout4.esm";
    const VAULT_CELL: &str = "0B02:Fallout4.esm";
    const SANCTUARY: &str = "L1:Fallout4.esm";
    const SANCTUARY_HILLS: &str = "L2:Fallout4.esm";
    const COMMONWEALTH_LOCATION: &str = "L3:Fallout4.esm";
    const OTHER_LOCATION: &str = "L4:Fallout4.esm";

    fn key_map(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        pairs
            .iter()
            .map(|(from, to)| (from.to_string(), to.to_string()))
            .collect()
    }

    fn catalog() -> LoadScreenCatalog {
        LoadScreenCatalog {
            cell_locations: key_map(&[(SANCTUARY_CELL, SANCTUARY), (VAULT_CELL, OTHER_LOCATION)]),
            location_parents: key_map(&[
                (SANCTUARY, SANCTUARY_HILLS),
                (SANCTUARY_HILLS, COMMONWEALTH_LOCATION),
            ]),
            globals: [("00AA:Fallout4.esm".to_string(), 1.0)].into(),
            global_form_keys: key_map(&[("HC_Rule", "00AA:Fallout4.esm")]),
            ..LoadScreenCatalog::default()
        }
    }

    fn exterior() -> Place {
        Place::Exterior {
            worldspace: COMMONWEALTH.to_string(),
        }
    }

    fn at(destination: &Place, cell: &str) -> LoadScreenContext<ScriptedRandom> {
        LoadScreenContext::new(&catalog(), destination, cell, ScriptedRandom::default())
    }

    fn holds(ctx: &impl ConditionContext, condition: Condition) -> bool {
        evaluate(&[condition], ctx).unwrap()
    }

    fn is(function: u16, record: &str, expected: f32) -> Condition {
        with_params(
            condition(function, CompareOp::Equal, Comparand::Value(expected)),
            record_param(record),
            ConditionParam::default(),
        )
    }

    #[test]
    fn the_worldspace_condition_matches_only_the_destination_worldspace() {
        let ctx = at(&exterior(), SANCTUARY_CELL);

        assert!(holds(&ctx, is(GET_IN_WORLDSPACE, COMMONWEALTH, 1.0)));
        assert!(!holds(
            &ctx,
            is(GET_IN_WORLDSPACE, "OTHER:Fallout4.esm", 1.0)
        ));
        assert!(holds(
            &ctx,
            is(GET_IN_WORLDSPACE, "OTHER:Fallout4.esm", 0.0)
        ));
    }

    #[test]
    fn an_interior_destination_is_in_no_worldspace_and_in_an_interior() {
        let ctx = at(&Place::Interior, VAULT_CELL);
        let interior =
            |expected| condition(IS_IN_INTERIOR, CompareOp::Equal, Comparand::Value(expected));

        assert!(holds(&ctx, is(GET_IN_WORLDSPACE, COMMONWEALTH, 0.0)));
        assert!(holds(&ctx, interior(1.0)));
        assert!(holds(&at(&exterior(), SANCTUARY_CELL), interior(0.0)));
    }

    #[test]
    fn the_cell_condition_matches_only_the_destination_cell() {
        let ctx = at(&exterior(), SANCTUARY_CELL);

        assert!(holds(&ctx, is(GET_IN_CELL, SANCTUARY_CELL, 1.0)));
        assert!(holds(&ctx, is(GET_IN_CELL, VAULT_CELL, 0.0)));
    }

    #[test]
    fn form_keys_compare_without_regard_to_case() {
        let ctx = at(&exterior(), SANCTUARY_CELL);

        assert!(holds(
            &ctx,
            is(GET_IN_CELL, &SANCTUARY_CELL.to_lowercase(), 1.0)
        ));
        assert!(holds(
            &ctx,
            is(GET_IN_WORLDSPACE, &COMMONWEALTH.to_lowercase(), 1.0)
        ));
    }

    #[test]
    fn the_location_condition_matches_the_cells_location_and_every_ancestor() {
        let ctx = at(&exterior(), SANCTUARY_CELL);

        for location in [SANCTUARY, SANCTUARY_HILLS, COMMONWEALTH_LOCATION] {
            assert!(
                holds(&ctx, is(GET_IN_CURRENT_LOCATION, location, 1.0)),
                "{location}"
            );
        }
        assert!(holds(
            &ctx,
            is(GET_IN_CURRENT_LOCATION, OTHER_LOCATION, 0.0)
        ));
    }

    #[test]
    fn a_location_is_not_in_its_own_descendants() {
        let mut data = catalog();
        data.cell_locations
            .insert(SANCTUARY_CELL.to_string(), SANCTUARY_HILLS.to_string());
        let hills = LoadScreenContext::new(
            &data,
            &exterior(),
            SANCTUARY_CELL,
            ScriptedRandom::default(),
        );

        assert!(holds(
            &hills,
            is(GET_IN_CURRENT_LOCATION, SANCTUARY_HILLS, 1.0)
        ));
        assert!(holds(&hills, is(GET_IN_CURRENT_LOCATION, SANCTUARY, 0.0)));
    }

    #[test]
    fn a_cell_without_a_location_is_in_no_location() {
        let ctx = at(&exterior(), "NOLOC:Fallout4.esm");

        assert!(holds(&ctx, is(GET_IN_CURRENT_LOCATION, SANCTUARY, 0.0)));
    }

    #[test]
    fn a_cycle_in_the_location_parents_ends_the_chain() {
        let mut data = catalog();
        data.location_parents
            .insert(COMMONWEALTH_LOCATION.to_string(), SANCTUARY.to_string());
        let ctx = LoadScreenContext::new(
            &data,
            &exterior(),
            SANCTUARY_CELL,
            ScriptedRandom::default(),
        );

        assert!(holds(
            &ctx,
            is(GET_IN_CURRENT_LOCATION, COMMONWEALTH_LOCATION, 1.0)
        ));
        assert!(holds(
            &ctx,
            is(GET_IN_CURRENT_LOCATION, OTHER_LOCATION, 0.0)
        ));
    }

    fn quest_probe(function: u16) -> Condition {
        let param2 = match function {
            GET_STAGE_DONE => number_param(10),
            GET_VM_QUEST_VARIABLE => text_param("::x_var"),
            _ => ConditionParam::default(),
        };
        with_params(
            condition(function, CompareOp::Equal, Comparand::Value(0.0)),
            record_param("QUEST"),
            param2,
        )
    }

    #[test]
    fn quest_state_answers_like_a_fresh_game() {
        let ctx = at(&exterior(), SANCTUARY_CELL);
        let fresh = FreshWorldContext::new(0.0, []);

        for function in [
            GET_QUEST_RUNNING,
            GET_QUEST_COMPLETED,
            GET_STAGE,
            GET_STAGE_DONE,
            GET_VM_QUEST_VARIABLE,
        ] {
            assert_eq!(
                evaluate(&[quest_probe(function)], &ctx),
                evaluate(&[quest_probe(function)], &fresh),
                "function {function}"
            );
            assert!(holds(&ctx, quest_probe(function)), "function {function}");
        }
    }

    #[test]
    fn globals_are_read_by_form_key_and_editor_id() {
        let ctx = at(&exterior(), SANCTUARY_CELL);

        assert_eq!(ctx.global("00AA:Fallout4.esm"), Some(1.0));
        assert_eq!(ctx.global("HC_Rule"), Some(1.0));
        assert_eq!(ctx.global("missing"), None);
    }

    #[test]
    fn random_percent_scales_a_fresh_roll_to_zero_up_to_one_hundred() {
        let rolls = ScriptedRandom::with_units([0.0, 0.5, 0.75]);
        let ctx = LoadScreenContext::new(&catalog(), &exterior(), SANCTUARY_CELL, rolls);

        let drawn: Vec<f32> = (0..3).map(|_| ctx.random_percent().unwrap()).collect();

        assert_eq!(drawn, vec![0.0, 50.0, 75.0]);
    }

    #[test]
    fn every_random_condition_gets_its_own_roll() {
        let rolls = ScriptedRandom::with_units([0.25, 0.75]);
        let ctx = LoadScreenContext::new(&catalog(), &exterior(), SANCTUARY_CELL, rolls);
        let at_most_50 = condition(
            GET_RANDOM_PERCENT,
            CompareOp::LessOrEqual,
            Comparand::Value(50.0),
        );

        assert!(holds(&ctx, at_most_50.clone()));
        assert!(!holds(&ctx, at_most_50));
    }

    #[test]
    fn unsupported_functions_stay_unsupported() {
        let ctx = at(&exterior(), SANCTUARY_CELL);

        assert_eq!(
            evaluate(&[is(71, "FACTION", 1.0)], &ctx),
            Err(ConditionError::UnsupportedFunction(71))
        );
    }
}
