use anyhow::{Context, bail};
use rusqlite::{Connection, OptionalExtension};

struct SettingRow {
    kind: String,
    integer: Option<i64>,
    float: Option<f64>,
    string: Option<String>,
}

fn fetch_setting(editor_id: &str, conn: &Connection) -> anyhow::Result<Option<SettingRow>> {
    conn.query_row(
        "SELECT value_type, value_int, value_float, value_string
         FROM game_settings WHERE editor_id = ?",
        rusqlite::params![editor_id],
        |row| {
            Ok(SettingRow {
                kind: row.get(0)?,
                integer: row.get(1)?,
                float: row.get(2)?,
                string: row.get(3)?,
            })
        },
    )
    .optional()
    .with_context(|| format!("reading game setting {editor_id}"))
}

/// The numeric value of the game setting named `editor_id`; `Ok(None)` when the database has no
/// such setting. An integer setting is widened to `f32`; a string or bool setting, or a numeric
/// one without a value, is an error.
pub fn get_game_setting_number(editor_id: &str, conn: &Connection) -> anyhow::Result<Option<f32>> {
    let Some(setting) = fetch_setting(editor_id, conn)? else {
        return Ok(None);
    };
    let number = match setting.kind.as_str() {
        "int" => setting.integer.map(|value| value as f32),
        "float" => setting.float.map(|value| value as f32),
        other => bail!("game setting {editor_id} is of type {other}, not a number"),
    };
    number.map(Some).with_context(|| {
        format!(
            "game setting {editor_id} of type {} has no value",
            setting.kind
        )
    })
}

/// The text of the game setting named `editor_id`; `Ok(None)` when the database has no such
/// setting. A setting of another type, or a string setting without a value, is an error.
pub fn get_game_setting_string(
    editor_id: &str,
    conn: &Connection,
) -> anyhow::Result<Option<String>> {
    let Some(setting) = fetch_setting(editor_id, conn)? else {
        return Ok(None);
    };
    if setting.kind != "string" {
        bail!(
            "game setting {editor_id} is of type {}, not a string",
            setting.kind
        );
    }
    setting
        .string
        .map(Some)
        .with_context(|| format!("game setting {editor_id} of type string has no value"))
}
