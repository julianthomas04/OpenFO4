//! `special_attributes` rows: the name and description of each SPECIAL attribute.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpecialAttributeRow {
    /// Lowercase attribute name: `strength` .. `luck`.
    pub special: String,
    pub name: Option<String>,
    pub description: Option<String>,
}

impl SpecialAttributeRow {
    pub fn list_all(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(
            "SELECT special, name, description FROM special_attributes ORDER BY special",
        )?;
        Ok(stmt
            .query_map([], |row| {
                Ok(Self {
                    special: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }
}
