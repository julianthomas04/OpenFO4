//! Shared helpers for turning database rows into typed values with errors that name the row.

use anyhow::Context;
use rusqlite::{Connection, Row};

pub(super) fn narrow<T: TryFrom<i64>>(value: i64, column: &str, row: &str) -> anyhow::Result<T> {
    T::try_from(value).map_err(|_| anyhow::anyhow!("{row}: {column} {value} is out of range"))
}

pub(super) fn required<T>(value: Option<T>, column: &str, row: &str) -> anyhow::Result<T> {
    value.ok_or_else(|| anyhow::anyhow!("{row}: {column} is NULL"))
}

pub(super) fn flag(value: i64, column: &str, row: &str) -> anyhow::Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        other => anyhow::bail!("{row}: {column} {other} is not 0 or 1"),
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the database stores f32 values widened to REAL"
)]
pub(super) fn real32(value: f64) -> f32 {
    value as f32
}

pub(super) fn collect_rows<T>(
    conn: &Connection,
    sql: &str,
    map: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
) -> anyhow::Result<Vec<T>> {
    let mut stmt = conn
        .prepare(sql)
        .with_context(|| format!("preparing `{sql}`"))?;
    let rows = stmt
        .query_map([], map)?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("reading `{sql}`"))?;
    Ok(rows)
}
