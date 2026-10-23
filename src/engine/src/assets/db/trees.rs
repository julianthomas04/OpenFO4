pub struct Tree {
    pub form_key: Option<String>,
    pub editor_id: Option<String>,
    pub name: Option<String>,
    pub model_path: Option<String>,
    pub ingredient: Option<String>,
    pub is_marker: bool,
}

fn row_to_tree(row: &rusqlite::Row) -> rusqlite::Result<Tree> {
    Ok(Tree {
        form_key: row.get(0)?,
        editor_id: row.get(1)?,
        name: row.get(2)?,
        model_path: row.get(3)?,
        ingredient: row.get(4)?,
        is_marker: row.get(5)?,
    })
}

impl Tree {
    pub fn get(form_key: &str, conn: &rusqlite::Connection) -> anyhow::Result<Self> {
        Ok(conn.query_row(
            "SELECT form_key, editor_id, name, model_path, ingredient, is_marker
             FROM trees WHERE form_key = ?",
            rusqlite::params![form_key],
            row_to_tree,
        )?)
    }

    pub fn get_all(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(
            "SELECT form_key, editor_id, name, model_path, ingredient, is_marker
             FROM trees",
        )?;

        Ok(stmt
            .query_map([], row_to_tree)?
            .collect::<Result<Vec<_>, _>>()?)
    }
}
