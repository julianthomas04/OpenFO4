//! A database built from the installer's own schema, for tests elsewhere in the crate.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::DbClient;
use crate::assets::layout::data_dir;
use crate::assets::{AssetRootEnv, resolve_asset_root};

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

/// The converted installation's database: `OPENFO4_DB`, else the install `OPENFO4_INSTALL_DIR`
/// names. For ignored real-install tests.
pub(crate) fn installed_database() -> DbClient {
    let path = std::env::var_os("OPENFO4_DB").map_or_else(
        || {
            let root = resolve_asset_root(
                &AssetRootEnv::from_process(),
                Path::new(env!("CARGO_MANIFEST_DIR")),
            )
            .expect("set OPENFO4_INSTALL_DIR to a converted installation");
            PathBuf::from(format!("{}/data.db", data_dir(root.to_str().unwrap())))
        },
        PathBuf::from,
    );
    DbClient::new(path.to_str().unwrap()).unwrap()
}
