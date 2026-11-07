use anyhow::{Context, bail};
use rusqlite::{Connection, OptionalExtension};

type SettingRow = (String, Option<i64>, Option<f64>);

/// The numeric value of the game setting named `editor_id`; `Ok(None)` when the database has no
/// such setting. An integer setting is widened to `f32`; a string or bool setting, or a numeric
/// one without a value, is an error.
pub fn get_game_setting_number(editor_id: &str, conn: &Connection) -> anyhow::Result<Option<f32>> {
    let row: Option<SettingRow> = conn
        .query_row(
            "SELECT value_type, value_int, value_float FROM game_settings WHERE editor_id = ?",
            rusqlite::params![editor_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .with_context(|| format!("reading game setting {editor_id}"))?;
    let Some((value_type, value_int, value_float)) = row else {
        return Ok(None);
    };
    let number = match value_type.as_str() {
        "int" => value_int.map(|value| value as f32),
        "float" => value_float.map(|value| value as f32),
        other => bail!("game setting {editor_id} is of type {other}, not a number"),
    };
    number
        .map(Some)
        .with_context(|| format!("game setting {editor_id} of type {value_type} has no value"))
}
