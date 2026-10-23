//! Navigation mesh rows (NAVM). Geometry stays raw here; `navmesh::decode` turns a row into a mesh.

use rusqlite::{Connection, Row};

use crate::navmesh::NavMeshRecord;

/// Rows an older installer left as stubs (key and editor id only) have no `format_version`.
const SELECT_DECODABLE: &str = "SELECT form_key, form_id, format_version,
            vertex_count, triangle_count, edge_link_count, door_link_count,
            vertices, triangles, edge_links, door_links
     FROM navigation_meshes
     WHERE format_version IS NOT NULL";

fn count(row: &Row, index: usize) -> rusqlite::Result<usize> {
    let value: i64 = row.get(index)?;
    usize::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

fn blob(row: &Row, index: usize) -> rusqlite::Result<Vec<u8>> {
    Ok(row.get::<_, Option<Vec<u8>>>(index)?.unwrap_or_default())
}

/// A row that could not be read, with the form key that identifies it in the log.
struct BadRow {
    form_key: String,
    error: rusqlite::Error,
}

fn record_from_row(row: &Row, form_key: String) -> Result<NavMeshRecord, BadRow> {
    let read = || -> rusqlite::Result<NavMeshRecord> {
        let form_id: i64 = row.get(1)?;
        let format_version: i64 = row.get(2)?;
        Ok(NavMeshRecord {
            form_key: form_key.clone(),
            form_id: u32::try_from(form_id)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(1, form_id))?,
            format_version: u32::try_from(format_version)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(2, format_version))?,
            vertex_count: count(row, 3)?,
            triangle_count: count(row, 4)?,
            edge_link_count: count(row, 5)?,
            door_link_count: count(row, 6)?,
            vertices: blob(row, 7)?,
            triangles: blob(row, 8)?,
            edge_links: blob(row, 9)?,
            door_links: blob(row, 10)?,
        })
    };
    read().map_err(|error| BadRow { form_key, error })
}

/// Keeps the readable rows and logs each unreadable one by its form key: one corrupt row must not
/// take the other navmeshes of its cell with it.
fn keep_readable(rows: Vec<Result<NavMeshRecord, BadRow>>) -> Vec<NavMeshRecord> {
    rows.into_iter()
        .filter_map(|row| match row {
            Ok(record) => Some(record),
            Err(bad) => {
                tracing::error!(
                    "navmesh {} skipped: unreadable row: {}",
                    bad.form_key,
                    bad.error
                );
                None
            }
        })
        .collect()
}

fn read_rows(
    stmt: &mut rusqlite::Statement,
    params: impl rusqlite::Params,
) -> anyhow::Result<Vec<NavMeshRecord>> {
    let rows = stmt
        .query_map(params, |row| {
            let form_key: String = row.get(0)?;
            Ok(record_from_row(row, form_key))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(keep_readable(rows))
}

pub fn get_for_cell(cell_form_key: &str, conn: &Connection) -> anyhow::Result<Vec<NavMeshRecord>> {
    let mut stmt = conn.prepare_cached(&format!("{SELECT_DECODABLE} AND cell = ?"))?;
    read_rows(&mut stmt, [cell_form_key])
}

pub fn get_all(conn: &Connection) -> anyhow::Result<Vec<NavMeshRecord>> {
    let mut stmt = conn.prepare(SELECT_DECODABLE)?;
    read_rows(&mut stmt, [])
}
