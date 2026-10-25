//! Music track conditions: the subset of Creation Kit condition functions that ambient music uses.
//!
//! Grouping follows the Creation Kit wiki page "Condition": the OR flag of a condition joins it with
//! the condition that follows it, runs of OR-joined conditions form one block, and the blocks are
//! `AND`ed. The wiki's example `A AND B OR C AND D` therefore reads `A AND (B OR C) AND D`. The OR flag
//! of the last condition has no successor and is ignored.

use std::collections::HashMap;

use crate::assets::db::music::{Comparand, CompareOp, Condition, MusicCatalog};

pub const GET_CURRENT_TIME: u16 = 18;
pub const GET_QUEST_RUNNING: u16 = 56;
pub const GET_STAGE: u16 = 58;
pub const GET_STAGE_DONE: u16 = 59;
pub const GET_GLOBAL_VALUE: u16 = 74;
pub const GET_VM_QUEST_VARIABLE: u16 = 629;
/// Function 391; the name is inferred from the footstep sound conditions it guards: the first-person
/// layer of a compound footstep sound requires it to be 1.
pub const IS_FIRST_PERSON: u16 = 391;
/// Function 448; the name is inferred from the footstep sound conditions it guards: the layer that
/// is silenced by the Silent Running perk requires it to be 0.
pub const HAS_SILENT_RUNNING_PERK: u16 = 448;

pub const GAME_HOUR_EDITOR_ID: &str = "GameHour";
pub const GAME_DAYS_PASSED_EDITOR_ID: &str = "GameDaysPassed";

/// The world clock as the condition layer reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GameTime {
    /// The `GameHour` global: the hour of the day with the minutes as a fraction.
    pub hour: f32,
    /// The `GameDaysPassed` global: whole days passed plus the elapsed fraction of today.
    pub days_passed: f32,
}

impl GameTime {
    pub fn new(hour: f32, whole_days_passed: u32) -> Self {
        Self {
            hour,
            days_passed: whole_days_passed as f32 + hour / HOURS_PER_DAY,
        }
    }

    pub fn at_hour(hour: f32) -> Self {
        Self::new(hour, 0)
    }
}

const HOURS_PER_DAY: f32 = 24.0;

#[derive(Debug, Clone, Copy)]
enum LiveGlobal {
    Hour,
    DaysPassed,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ConditionError {
    #[error("condition function {0} is not supported")]
    UnsupportedFunction(u16),
    #[error("condition function {function} needs {expected} as parameter {position}")]
    BadParameter {
        function: u16,
        position: u8,
        expected: &'static str,
    },
    #[error("condition compares against the unknown global {0}")]
    UnknownGlobal(String),
}

#[allow(
    clippy::float_cmp,
    reason = "the game compares condition values for exact equality"
)]
fn holds(op: CompareOp, left: f32, right: f32) -> bool {
    match op {
        CompareOp::Equal => left == right,
        CompareOp::NotEqual => left != right,
        CompareOp::Greater => left > right,
        CompareOp::GreaterOrEqual => left >= right,
        CompareOp::Less => left < right,
        CompareOp::LessOrEqual => left <= right,
    }
}

pub trait ConditionContext {
    fn game_hour(&self) -> f32;
    fn global(&self, key: &str) -> Option<f32>;
    fn quest_running(&self, quest: &str) -> bool;
    fn quest_stage(&self, quest: &str) -> i32;
    fn stage_done(&self, quest: &str, stage: i32) -> bool;
    fn vm_quest_variable(&self, quest: &str, variable: &str) -> f32;
    fn is_first_person(&self) -> bool;
    fn has_silent_running_perk(&self) -> bool;
}

#[derive(Debug, Clone)]
pub struct GlobalEntry {
    pub key: String,
    pub editor_id: Option<String>,
    pub value: f32,
}

/// Every global of the catalog with its editor id where it has one.
pub fn global_entries(catalog: &MusicCatalog) -> Vec<GlobalEntry> {
    let editor_ids: HashMap<&str, &str> = catalog
        .global_form_keys
        .iter()
        .map(|(editor_id, key)| (key.as_str(), editor_id.as_str()))
        .collect();
    catalog
        .globals
        .iter()
        .map(|(key, &value)| GlobalEntry {
            key: key.clone(),
            editor_id: editor_ids.get(key.as_str()).map(ToString::to_string),
            value,
        })
        .collect()
}

/// A new game before any quest started; quest state is empty and `GameHour` follows the clock.
#[derive(Debug, Clone)]
pub struct FreshWorldContext {
    time: GameTime,
    globals: HashMap<String, f32>,
    live_globals: HashMap<String, LiveGlobal>,
}

impl FreshWorldContext {
    /// Globals are reachable by form key and by editor id (both case-insensitive). Every global whose
    /// editor id is `GameHour` or `GameDaysPassed` is answered from the clock instead of its ESM
    /// value. The clock starts at `hour` with no days passed; [`set_time`](Self::set_time) follows it.
    pub fn new(hour: f32, globals: impl IntoIterator<Item = GlobalEntry>) -> Self {
        let mut map = HashMap::new();
        let mut live_globals = HashMap::new();
        for entry in globals {
            let key = entry.key.to_ascii_lowercase();
            map.insert(key.clone(), entry.value);
            if let Some(editor_id) = entry.editor_id {
                let editor_id = editor_id.to_ascii_lowercase();
                let live = [
                    (GAME_HOUR_EDITOR_ID, LiveGlobal::Hour),
                    (GAME_DAYS_PASSED_EDITOR_ID, LiveGlobal::DaysPassed),
                ]
                .into_iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(&editor_id));
                if let Some((_, live)) = live {
                    live_globals.insert(key, live);
                    live_globals.insert(editor_id.clone(), live);
                }
                map.insert(editor_id, entry.value);
            }
        }
        Self {
            time: GameTime::at_hour(hour),
            globals: map,
            live_globals,
        }
    }
}

impl FreshWorldContext {
    pub fn set_time(&mut self, time: GameTime) {
        self.time = time;
    }
}

impl ConditionContext for FreshWorldContext {
    fn game_hour(&self) -> f32 {
        self.time.hour
    }

    fn global(&self, key: &str) -> Option<f32> {
        let key = key.to_ascii_lowercase();
        match self.live_globals.get(&key) {
            Some(LiveGlobal::Hour) => Some(self.time.hour),
            Some(LiveGlobal::DaysPassed) => Some(self.time.days_passed),
            None => self.globals.get(&key).copied(),
        }
    }

    fn quest_running(&self, _quest: &str) -> bool {
        false
    }

    fn quest_stage(&self, _quest: &str) -> i32 {
        0
    }

    fn stage_done(&self, _quest: &str, _stage: i32) -> bool {
        false
    }

    fn vm_quest_variable(&self, _quest: &str, _variable: &str) -> f32 {
        0.0
    }

    fn is_first_person(&self) -> bool {
        true
    }

    fn has_silent_running_perk(&self) -> bool {
        false
    }
}

fn bad_parameter(condition: &Condition, position: u8, expected: &'static str) -> ConditionError {
    ConditionError::BadParameter {
        function: condition.function,
        position,
        expected,
    }
}

fn quest_param(condition: &Condition) -> Result<&str, ConditionError> {
    condition
        .param1
        .record
        .as_deref()
        .ok_or_else(|| bad_parameter(condition, 1, "a record"))
}

fn function_value(
    condition: &Condition,
    ctx: &dyn ConditionContext,
) -> Result<f32, ConditionError> {
    match condition.function {
        GET_CURRENT_TIME => Ok(ctx.game_hour()),
        GET_QUEST_RUNNING => Ok(f32::from(u8::from(
            ctx.quest_running(quest_param(condition)?),
        ))),
        GET_STAGE => Ok(ctx.quest_stage(quest_param(condition)?) as f32),
        GET_STAGE_DONE => {
            let quest = quest_param(condition)?;
            let stage = condition
                .param2
                .num
                .and_then(|stage| i32::try_from(stage).ok())
                .ok_or_else(|| bad_parameter(condition, 2, "a stage number"))?;
            Ok(f32::from(u8::from(ctx.stage_done(quest, stage))))
        }
        GET_GLOBAL_VALUE => {
            let key = quest_param(condition)?;
            ctx.global(key)
                .ok_or_else(|| ConditionError::UnknownGlobal(key.to_string()))
        }
        GET_VM_QUEST_VARIABLE => {
            let quest = quest_param(condition)?;
            let variable = condition
                .param2
                .string
                .as_deref()
                .ok_or_else(|| bad_parameter(condition, 2, "a variable name"))?;
            Ok(ctx.vm_quest_variable(quest, variable))
        }
        IS_FIRST_PERSON => Ok(f32::from(u8::from(ctx.is_first_person()))),
        HAS_SILENT_RUNNING_PERK => Ok(f32::from(u8::from(ctx.has_silent_running_perk()))),
        other => Err(ConditionError::UnsupportedFunction(other)),
    }
}

fn compare_value(condition: &Condition, ctx: &dyn ConditionContext) -> Result<f32, ConditionError> {
    match &condition.comparand {
        Comparand::Value(value) => Ok(*value),
        Comparand::Global(key) => ctx
            .global(key)
            .ok_or_else(|| ConditionError::UnknownGlobal(key.clone())),
    }
}

fn evaluate_one(condition: &Condition, ctx: &dyn ConditionContext) -> Result<bool, ConditionError> {
    let left = function_value(condition, ctx)?;
    let right = compare_value(condition, ctx)?;
    Ok(holds(condition.op, left, right))
}

/// An empty list is true.
pub fn evaluate(
    conditions: &[Condition],
    ctx: &dyn ConditionContext,
) -> Result<bool, ConditionError> {
    let mut all_blocks = true;
    let mut block = false;
    for condition in conditions {
        block |= evaluate_one(condition, ctx)?;
        if !condition.or_flag {
            all_blocks &= block;
            block = false;
        }
    }
    if conditions.last().is_some_and(|last| last.or_flag) {
        all_blocks &= block;
    }
    Ok(all_blocks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::db::music::ConditionParam;
    use crate::audio::test_support::condition::{
        condition, day, night, number_param, or, record_param, text_param, time, with_params,
    };

    fn clock(hour: f32) -> FreshWorldContext {
        FreshWorldContext::new(hour, [])
    }

    fn is_day(hour: f32) -> bool {
        evaluate(&day(), &clock(hour)).unwrap()
    }

    fn is_night(hour: f32) -> bool {
        evaluate(&night(), &clock(hour)).unwrap()
    }

    #[test]
    fn day_and_night_split_the_clock_at_the_documented_hours() {
        let cases = [
            (5.0, false),
            (6.0, true),
            (12.0, true),
            (20.0, true),
            (20.5, false),
            (21.0, false),
            (9.783, true),
        ];
        for (hour, expected_day) in cases {
            assert_eq!(is_day(hour), expected_day, "day at {hour}");
            assert_eq!(is_night(hour), !expected_day, "night at {hour}");
        }
    }

    // Creation Kit wiki "Condition": `A AND B OR C AND D` is `A AND (B OR C) AND D`.
    #[test]
    fn or_binds_tighter_than_and_per_the_creation_kit_grouping() {
        let conditions = [
            time(CompareOp::GreaterOrEqual, 1.0),
            or(time(CompareOp::Equal, 99.0)),
            time(CompareOp::Equal, 5.0),
            time(CompareOp::Less, 10.0),
        ];
        assert!(evaluate(&conditions, &clock(5.0)).unwrap());
        assert!(!evaluate(&conditions, &clock(7.0)).unwrap());
        assert!(!evaluate(&conditions, &clock(0.5)).unwrap());
    }

    #[test]
    fn standard_boolean_precedence_would_give_the_other_answer() {
        // Plain logic `(A AND B) OR (C AND D)` is true at 12 through A AND B; the Creation Kit
        // reading `A AND (B OR C) AND D` is true there only if D (< 10) holds, which it does not.
        let conditions = [
            time(CompareOp::GreaterOrEqual, 1.0),
            or(time(CompareOp::Equal, 12.0)),
            time(CompareOp::Equal, 5.0),
            time(CompareOp::Less, 10.0),
        ];
        assert!(!evaluate(&conditions, &clock(12.0)).unwrap());
    }

    #[test]
    fn trailing_or_flag_closes_its_block() {
        let conditions = [or(time(CompareOp::Equal, 3.0))];
        assert!(!evaluate(&conditions, &clock(4.0)).unwrap());
        assert!(evaluate(&conditions, &clock(3.0)).unwrap());
    }

    #[test]
    fn empty_conditions_are_true() {
        assert!(evaluate(&[], &clock(0.0)).unwrap());
    }

    fn global_entry(key: &str, editor_id: &str, value: f32) -> GlobalEntry {
        GlobalEntry {
            key: key.to_string(),
            editor_id: Some(editor_id.to_string()),
            value,
        }
    }

    #[test]
    fn global_compare_value_reads_the_global() {
        let night_start = || global_entry("000ABC:Fallout4.esm", "NightStart", 20.0);
        let at_most_night_start = condition(
            GET_CURRENT_TIME,
            CompareOp::LessOrEqual,
            Comparand::Global("000ABC:Fallout4.esm".to_string()),
        );
        let evening = FreshWorldContext::new(19.0, [night_start()]);
        let late = FreshWorldContext::new(21.0, [night_start()]);
        assert!(evaluate(std::slice::from_ref(&at_most_night_start), &evening).unwrap());
        assert!(!evaluate(&[at_most_night_start], &late).unwrap());
    }

    #[test]
    fn unknown_global_is_an_error() {
        let condition = condition(
            GET_CURRENT_TIME,
            CompareOp::Less,
            Comparand::Global("missing".to_string()),
        );
        assert_eq!(
            evaluate(&[condition], &clock(1.0)),
            Err(ConditionError::UnknownGlobal("missing".to_string()))
        );
    }

    #[test]
    fn get_global_value_reads_esm_values_and_game_hour_follows_the_clock() {
        let ctx = FreshWorldContext::new(
            9.783,
            [
                global_entry("000038:Fallout4.esm", "GameHour", 8.0),
                global_entry("00003A:Fallout4.esm", "TimeScale", 20.0),
            ],
        );
        let global_is = |key: &str, value: f32| {
            let condition = with_params(
                condition(GET_GLOBAL_VALUE, CompareOp::Equal, Comparand::Value(value)),
                record_param(key),
                ConditionParam::default(),
            );
            evaluate(&[condition], &ctx).unwrap()
        };
        assert!(global_is("00003A:Fallout4.esm", 20.0));
        assert!(global_is("000038:Fallout4.esm", 9.783));
        assert!(global_is("gamehour", 9.783));
    }

    #[test]
    fn game_days_passed_follows_the_clock_as_days_plus_the_fraction_of_today() {
        let mut ctx = FreshWorldContext::new(
            0.0,
            [global_entry("000039:Fallout4.esm", "GameDaysPassed", 1.0)],
        );
        ctx.set_time(GameTime::new(12.0, 3));
        assert_eq!(ctx.global("000039:Fallout4.esm"), Some(3.5));
        assert_eq!(ctx.global("GameDaysPassed"), Some(3.5));
        assert_eq!(ctx.global("gamedayspassed"), Some(3.5));
        ctx.set_time(GameTime::new(18.0, 4));
        assert_eq!(ctx.global("GameDaysPassed"), Some(4.75));
    }

    #[test]
    fn the_hour_and_days_passed_globals_are_answered_independently() {
        let mut ctx = FreshWorldContext::new(
            0.0,
            [
                global_entry("000038:Fallout4.esm", "GameHour", 8.0),
                global_entry("000039:Fallout4.esm", "GameDaysPassed", 1.0),
            ],
        );
        ctx.set_time(GameTime::new(6.0, 2));
        assert_eq!(ctx.global("GameHour"), Some(6.0));
        assert_eq!(ctx.global("GameDaysPassed"), Some(2.25));
    }

    #[test]
    fn fresh_world_has_no_quest_state() {
        let ctx = clock(0.0);
        let eval = |function, param2| {
            let condition = with_params(
                condition(function, CompareOp::Equal, Comparand::Value(0.0)),
                record_param("quest"),
                param2,
            );
            evaluate(&[condition], &ctx).unwrap()
        };
        assert!(eval(GET_QUEST_RUNNING, ConditionParam::default()));
        assert!(eval(GET_STAGE, ConditionParam::default()));
        assert!(eval(GET_STAGE_DONE, number_param(10)));
        assert!(eval(GET_VM_QUEST_VARIABLE, text_param("::x_var")));
    }

    fn flag_condition(function: u16, expected: f32) -> Condition {
        condition(function, CompareOp::Equal, Comparand::Value(expected))
    }

    #[test]
    fn first_person_function_is_true_in_a_fresh_world() {
        assert!(evaluate(&[flag_condition(IS_FIRST_PERSON, 1.0)], &clock(0.0)).unwrap());
        assert!(!evaluate(&[flag_condition(IS_FIRST_PERSON, 0.0)], &clock(0.0)).unwrap());
    }

    #[test]
    fn silent_running_function_is_false_in_a_fresh_world() {
        assert!(evaluate(&[flag_condition(HAS_SILENT_RUNNING_PERK, 0.0)], &clock(0.0)).unwrap());
        assert!(!evaluate(&[flag_condition(HAS_SILENT_RUNNING_PERK, 1.0)], &clock(0.0)).unwrap());
    }

    #[test]
    fn unknown_function_is_rejected_by_name() {
        let conditions = [
            time(CompareOp::Equal, 1.0),
            condition(47, CompareOp::Equal, Comparand::Value(1.0)),
        ];
        assert_eq!(
            evaluate(&conditions, &clock(1.0)),
            Err(ConditionError::UnsupportedFunction(47))
        );
    }

    #[test]
    fn a_missing_parameter_is_rejected_by_function_and_position() {
        let no_quest = condition(GET_STAGE, CompareOp::Equal, Comparand::Value(0.0));
        assert_eq!(
            evaluate(&[no_quest], &clock(1.0)),
            Err(ConditionError::BadParameter {
                function: GET_STAGE,
                position: 1,
                expected: "a record"
            })
        );
    }
}
