//! `perks` rows (rank records linked by `next_perk`) and the `perk_chart` placement table.

#[derive(Clone, Debug, PartialEq)]
pub struct PerkRow {
    pub form_key: String,
    pub editor_id: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    /// Form key of the next rank's record.
    pub next_perk: Option<String>,
    pub level: u32,
    pub playable: bool,
    pub hidden: bool,
    /// `Interface`-relative Vault Boy movie path: as authored, or derived from the form id for chart
    /// perks the game gives no movie.
    pub swf_path: Option<String>,
    /// Form key of the first rank of this record's rank chain; the record's own key for a first rank.
    pub first_rank: Option<String>,
}

impl PerkRow {
    /// Every perk record, ranks included; the caller follows `next_perk` to build chains.
    /// Requires the Pip-Boy `perks` columns; the client skips the call on a database without them.
    pub fn list_all(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(
            "SELECT form_key, editor_id, name, description, next_perk, level, playable, hidden, swf_path, first_rank
             FROM perks ORDER BY form_key",
        )?;
        Ok(stmt
            .query_map([], |row| {
                Ok(Self {
                    form_key: row.get(0)?,
                    editor_id: row.get(1)?,
                    name: row.get(2)?,
                    description: row.get(3)?,
                    next_perk: row.get(4)?,
                    level: row.get::<_, Option<u32>>(5)?.unwrap_or(0),
                    playable: row.get::<_, Option<bool>>(6)?.unwrap_or(false),
                    hidden: row.get::<_, Option<bool>>(7)?.unwrap_or(false),
                    swf_path: row.get(8)?,
                    first_rank: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }
}

/// One perk-chart cell: the first-rank perk record (`perks.first_rank` equals its own key), its
/// SPECIAL column and its row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerkChartRow {
    pub perk: String,
    /// Lowercase SPECIAL name: `strength` .. `luck`.
    pub special: String,
    /// Attribute rank that unlocks the perk, `1..=10` (the chart row).
    pub required_special: u8,
}

impl PerkChartRow {
    pub fn list_all(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt =
            conn.prepare("SELECT perk, special, required_special FROM perk_chart ORDER BY perk")?;
        Ok(stmt
            .query_map([], |row| {
                Ok(Self {
                    perk: row.get(0)?,
                    special: row.get(1)?,
                    required_special: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }
}
