//! Builders for the catalog's condition type, so tests can state a condition in one line.

use crate::assets::db::music::{Comparand, CompareOp, Condition, ConditionParam};
use crate::audio::conditions::GET_CURRENT_TIME;

pub fn condition(function: u16, op: CompareOp, comparand: Comparand) -> Condition {
    Condition {
        function,
        op,
        or_flag: false,
        comparand,
        run_on: 0,
        reference: None,
        param1: ConditionParam::default(),
        param2: ConditionParam::default(),
        param3: None,
    }
}

/// `GetCurrentTime <op> value`.
pub fn time(op: CompareOp, value: f32) -> Condition {
    condition(GET_CURRENT_TIME, op, Comparand::Value(value))
}

/// Joins the condition with the one that follows it.
pub fn or(condition: Condition) -> Condition {
    Condition {
        or_flag: true,
        ..condition
    }
}

pub fn with_params(
    condition: Condition,
    param1: ConditionParam,
    param2: ConditionParam,
) -> Condition {
    Condition {
        param1,
        param2,
        ..condition
    }
}

pub fn record_param(key: &str) -> ConditionParam {
    ConditionParam {
        record: Some(key.to_string()),
        ..ConditionParam::default()
    }
}

pub fn number_param(number: i64) -> ConditionParam {
    ConditionParam {
        num: Some(number),
        ..ConditionParam::default()
    }
}

pub fn text_param(text: &str) -> ConditionParam {
    ConditionParam {
        string: Some(text.to_string()),
        ..ConditionParam::default()
    }
}

/// Day is 6:00 through 20:00.
pub fn day() -> Vec<Condition> {
    vec![
        time(CompareOp::GreaterOrEqual, 6.0),
        time(CompareOp::LessOrEqual, 20.0),
    ]
}

/// Night is before 6:00 or after 20:00.
pub fn night() -> Vec<Condition> {
    vec![
        or(time(CompareOp::Less, 6.0)),
        time(CompareOp::Greater, 20.0),
    ]
}
