//! MATT material types: the buoyancy (`BNAM`) of each Havok material, keyed by
//! the material hash collision shapes carry. Read once at startup for the water simulation.

pub struct MaterialBuoyancyRow {
    /// CRC-32 of the lower-cased MATT material name (`material_types.material_hash`).
    pub material_hash: u32,
    pub buoyancy: f32,
}

impl MaterialBuoyancyRow {
    /// Every material with a name (hence a hash) and a buoyancy.
    pub fn get_all(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(
            "SELECT material_hash, buoyancy FROM material_types
             WHERE material_hash IS NOT NULL AND buoyancy IS NOT NULL",
        )?;
        stmt.query_map([], |row| {
            Ok(Self {
                material_hash: row.get(0)?,
                buoyancy: row.get(1)?,
            })
        })?
        .map(|row| row.map_err(Into::into))
        .collect()
    }
}
