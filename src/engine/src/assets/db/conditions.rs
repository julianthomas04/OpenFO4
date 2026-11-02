//! Condition (CTDA) rows and the globals they compare against, shared by every catalog that
//! stores conditions. Values are raw Bethesda units; evaluation lives in `crate::conditions`.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::assets::db::music::{collect_rows, flag, narrow, real32, required};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Equal,
    NotEqual,
    Greater,
    GreaterOrEqual,
    Less,
    LessOrEqual,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Comparand {
    Value(f32),
    Global(String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConditionParam {
    pub num: Option<i64>,
    pub record: Option<String>,
    pub string: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    pub function: u16,
    pub op: CompareOp,
    /// True when this condition is OR-ed with the next one.
    pub or_flag: bool,
    /// The stored CTDA type bits other than the operator and OR flag (see [`Self::uses_aliases`]).
    pub flags: u32,
    pub comparand: Comparand,
    pub run_on: u8,
    pub reference: Option<String>,
    pub param1: ConditionParam,
    pub param2: ConditionParam,
    pub param3: Option<i64>,
}

impl Condition {
    /// CTDA flag bit: the parameters name quest aliases instead of records.
    const FLAG_USES_ALIASES: u32 = 2;

    pub fn uses_aliases(&self) -> bool {
        self.flags & Self::FLAG_USES_ALIASES != 0
    }
}

struct ConditionRow {
    track: String,
    ord: i64,
    function: i64,
    compare_op: i64,
    or_flag: i64,
    flags: i64,
    compare_value: Option<f64>,
    compare_global: Option<String>,
    run_on: i64,
    reference: Option<String>,
    param1: ConditionParam,
    param2: ConditionParam,
    param3: Option<i64>,
}

pub(crate) fn load_conditions(
    conn: &Connection,
    table: &str,
    owner_column: &str,
    owner_label: &str,
) -> anyhow::Result<HashMap<String, Vec<Condition>>> {
    let rows = collect_rows(
        conn,
        &format!(
            "SELECT {owner_column}, ord, function, compare_op, or_flag, flags, compare_value, compare_global, run_on, \
         reference, param1_num, param1_record, param1_string, param2_num, param2_record, \
         param2_string, param3 FROM {table} ORDER BY {owner_column}, ord"
        ),
        |row| {
            Ok(ConditionRow {
                track: row.get(0)?,
                ord: row.get(1)?,
                function: row.get(2)?,
                compare_op: row.get(3)?,
                or_flag: row.get(4)?,
                flags: row.get(5)?,
                compare_value: row.get(6)?,
                compare_global: row.get(7)?,
                run_on: row.get(8)?,
                reference: row.get(9)?,
                param1: ConditionParam {
                    num: row.get(10)?,
                    record: row.get(11)?,
                    string: row.get(12)?,
                },
                param2: ConditionParam {
                    num: row.get(13)?,
                    record: row.get(14)?,
                    string: row.get(15)?,
                },
                param3: row.get(16)?,
            })
        },
    )?;
    let mut grouped: HashMap<String, Vec<Condition>> = HashMap::new();
    for row in rows {
        let name = format!("{owner_label} {} condition {}", row.track, row.ord);
        let comparand = match (row.compare_value, row.compare_global) {
            (Some(value), None) => Comparand::Value(real32(value)),
            (None, Some(global)) => Comparand::Global(global),
            (Some(_), Some(_)) => {
                anyhow::bail!("{name}: both compare_value and compare_global are set")
            }
            (None, None) => {
                anyhow::bail!("{name}: neither compare_value nor compare_global is set")
            }
        };
        let condition = Condition {
            function: narrow(row.function, "function", &name)?,
            op: compare_op(row.compare_op, &name)?,
            or_flag: flag(row.or_flag, "or_flag", &name)?,
            flags: narrow(row.flags, "flags", &name)?,
            comparand,
            run_on: narrow(row.run_on, "run_on", &name)?,
            reference: row.reference,
            param1: row.param1,
            param2: row.param2,
            param3: row.param3,
        };
        grouped.entry(row.track).or_default().push(condition);
    }
    Ok(grouped)
}

fn compare_op(value: i64, row: &str) -> anyhow::Result<CompareOp> {
    Ok(match value {
        0 => CompareOp::Equal,
        1 => CompareOp::NotEqual,
        2 => CompareOp::Greater,
        3 => CompareOp::GreaterOrEqual,
        4 => CompareOp::Less,
        5 => CompareOp::LessOrEqual,
        other => anyhow::bail!("{row}: unknown compare_op {other}"),
    })
}

/// Global values by form key, and global editor ids to form keys.
pub(crate) type Globals = (HashMap<String, f32>, HashMap<String, String>);

pub(crate) fn load_globals(conn: &Connection) -> anyhow::Result<Globals> {
    let rows = collect_rows(
        conn,
        "SELECT form_key, editor_id, value FROM globals",
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<f64>>(2)?,
            ))
        },
    )?;
    let mut values = HashMap::new();
    let mut form_keys = HashMap::new();
    for (form_key, editor_id, value) in rows {
        let value = required(value, "value", &format!("global {form_key}"))?;
        if let Some(editor_id) = editor_id {
            form_keys.insert(editor_id, form_key.clone());
        }
        values.insert(form_key, real32(value));
    }
    Ok((values, form_keys))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: &str = include_str!("../../../../installer/file_converters/esm_schema.sql");

    fn connection_with_conditions(rows: &[(i64, i64)]) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        for &(ord, flags) in rows {
            conn.execute(
                "INSERT INTO sound_descriptor_conditions \
                 (descriptor, ord, function, compare_op, or_flag, flags, compare_value, run_on) \
                 VALUES ('01:Fallout4.esm', ?1, 74, 0, 0, ?2, 1.0, 0)",
                rusqlite::params![ord, flags],
            )
            .unwrap();
        }
        conn
    }

    fn load(conn: &Connection) -> Vec<Condition> {
        load_conditions(
            conn,
            "sound_descriptor_conditions",
            "descriptor",
            "sound descriptor",
        )
        .unwrap()
        .remove("01:Fallout4.esm")
        .unwrap()
    }

    #[test]
    fn the_stored_ctda_flags_are_read_and_the_alias_bit_is_told_apart_from_the_others() {
        let conn = connection_with_conditions(&[(0, 0), (1, 2), (2, 8 | 16), (3, 2 | 8)]);

        let conditions = load(&conn);

        let loaded: Vec<(u32, bool)> = conditions
            .iter()
            .map(|c| (c.flags, c.uses_aliases()))
            .collect();
        assert_eq!(loaded, vec![(0, false), (2, true), (24, false), (10, true)]);
    }

    #[test]
    fn a_negative_stored_flags_value_is_rejected_by_naming_the_condition() {
        let conn = connection_with_conditions(&[(4, -1)]);

        let error = load_conditions(
            &conn,
            "sound_descriptor_conditions",
            "descriptor",
            "sound descriptor",
        )
        .unwrap_err();

        let message = format!("{error:#}");
        assert!(
            message.contains("condition 4") && message.contains("flags -1"),
            "{message}"
        );
    }
}
