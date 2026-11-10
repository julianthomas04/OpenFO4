//! Regions that carry a map entry: the map name and priority the original game uses to name
//! an open-world place, with the polygons that bound the region. Read once at boot.

use rusqlite::{Connection, Row};

use super::music::decode_points;
use super::schema::DbSchema;

#[derive(Debug, Clone, PartialEq)]
pub struct MapRegion {
    pub form_key: String,
    pub worldspace: String,
    pub name: String,
    pub priority: i64,
    /// Polygons in Bethesda world units; a point inside any of them is inside the region.
    pub areas: Vec<Vec<(f32, f32)>>,
}

struct MapRegionRow {
    form_key: String,
    worldspace: String,
    name: String,
    priority: i64,
    points: Option<Vec<u8>>,
}

impl MapRegionRow {
    fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            form_key: row.get(0)?,
            worldspace: row.get(1)?,
            name: row.get(2)?,
            priority: row.get(3)?,
            points: row.get(4)?,
        })
    }
}

/// The regions with a non-blank map name, a map priority and a worldspace, ordered by form key.
/// A region without polygons is kept with no areas, so it never contains a point. A database
/// without the map priority column has no map regions.
pub fn get_all(conn: &Connection, schema: DbSchema) -> anyhow::Result<Vec<MapRegion>> {
    if !schema.region_map_column_present {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT r.form_key, r.worldspace, r.name, r.map_priority, a.points
         FROM regions r
         LEFT JOIN region_areas a ON a.region = r.form_key
         WHERE r.worldspace IS NOT NULL
           AND r.map_priority IS NOT NULL
           AND r.name IS NOT NULL AND trim(r.name) <> ''
         ORDER BY r.form_key, a.area",
    )?;
    let rows = stmt
        .query_map([], MapRegionRow::from_row)?
        .collect::<Result<Vec<_>, _>>()?;

    let mut regions: Vec<MapRegion> = Vec::new();
    for row in rows {
        if regions
            .last()
            .is_none_or(|last| last.form_key != row.form_key)
        {
            regions.push(MapRegion {
                form_key: row.form_key.clone(),
                worldspace: row.worldspace,
                name: row.name,
                priority: row.priority,
                areas: Vec::new(),
            });
        }
        if let Some(blob) = row.points {
            let area = decode_points(&blob, &format!("map region {}", row.form_key))?;
            regions
                .last_mut()
                .expect("a region was pushed above")
                .areas
                .push(area);
        }
    }
    Ok(regions)
}
