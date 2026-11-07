use rusqlite::{Connection, OptionalExtension};

/// The value of the global named `editor_id`; `Ok(None)` when the database has no such global.
/// A global whose value is NULL is an error.
pub fn get_global(editor_id: &str, conn: &Connection) -> rusqlite::Result<Option<f32>> {
    conn.query_row(
        "SELECT value FROM globals WHERE editor_id = ?",
        rusqlite::params![editor_id],
        |row| row.get::<_, f64>(0).map(|value| value as f32),
    )
    .optional()
}
