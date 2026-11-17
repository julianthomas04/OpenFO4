//! The faction memberships of NPC base records.

use super::schema::DbSchema;

#[derive(Clone, Debug, PartialEq)]
pub struct NpcFactionRow {
    pub npc: String,
    pub faction: String,
    pub rank: i64,
}

impl NpcFactionRow {
    /// The factions of `npc` (none for an unknown NPC); `None` for a database converted before
    /// the installer extracted them.
    pub fn for_npc(
        conn: &rusqlite::Connection,
        schema: DbSchema,
        npc: &str,
    ) -> anyhow::Result<Option<Vec<Self>>> {
        if !schema.npc_factions_table_present {
            return Ok(None);
        }
        let mut stmt = conn.prepare_cached(
            "SELECT npc, faction, rank FROM npc_factions WHERE npc = ? ORDER BY faction, rank",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![npc], |row| {
                Ok(Self {
                    npc: row.get(0)?,
                    faction: row.get(1)?,
                    rank: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(rows))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::database_with;

    #[test]
    fn an_npcs_factions_come_back_and_other_npcs_are_left_out() {
        let db = database_with(
            "INSERT INTO npc_factions (npc, faction, rank) VALUES
                ('0A:T.esm', '0F:T.esm', 2),
                ('0A:T.esm', '0D:T.esm', 0),
                ('0B:T.esm', '0E:T.esm', 1);",
        );

        let rows = db.get_npc_factions("0A:T.esm").unwrap().unwrap();

        let listed: Vec<(&str, i64)> = rows
            .iter()
            .map(|row| (row.faction.as_str(), row.rank))
            .collect();
        assert_eq!(listed, [("0D:T.esm", 0), ("0F:T.esm", 2)]);
        assert!(rows.iter().all(|row| row.npc == "0A:T.esm"));
    }

    #[test]
    fn an_npc_without_factions_has_an_empty_list() {
        let db = database_with("");

        assert_eq!(db.get_npc_factions("0A:T.esm").unwrap(), Some(Vec::new()));
    }

    #[test]
    fn a_database_without_the_table_has_no_faction_data() {
        let db = database_with("DROP TABLE npc_factions;");

        assert_eq!(db.get_npc_factions("0A:T.esm").unwrap(), None);
    }
}
