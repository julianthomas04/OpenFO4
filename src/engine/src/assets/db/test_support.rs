//! A database built from the installer's own schema, for tests elsewhere in the crate.

use rusqlite::Connection;

use super::DbClient;

const SCHEMA: &str = include_str!("../../../../installer/file_converters/esm_schema.sql");

/// The real schema in memory; `statements` run on it before the client reads it, and may drop
/// columns to stand for a database from an older installer.
pub(crate) fn database_with(statements: &str) -> DbClient {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(SCHEMA).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
    conn.execute_batch(statements).unwrap();
    DbClient::from_connection(conn).unwrap()
}
