use crate::assets::db::schema::{DbSchema, activator_water_select_list};

pub struct Activator {
    pub form_key: Option<String>,
    pub editor_id: Option<String>,
    pub name: Option<String>,
    pub model_path: Option<String>,
    pub script: Option<String>,
    pub activation_sound: Option<String>,
    pub flags: Option<String>,
    pub is_marker: bool,
    /// WATR form key of a placed water mesh (`ACTI.WNAM`); `None` for every
    /// other activator and for databases predating water extraction.
    pub water_type: Option<String>,
}

fn row_to_activator(row: &rusqlite::Row) -> rusqlite::Result<Activator> {
    Ok(Activator {
        form_key: row.get(0)?,
        editor_id: row.get(1)?,
        name: row.get(2)?,
        model_path: row.get(3)?,
        script: row.get(4)?,
        activation_sound: row.get(5)?,
        flags: row.get(6)?,
        is_marker: row.get(7)?,
        water_type: row.get(8)?,
    })
}

impl Activator {
    /// Column list feeding [`row_to_activator`], whose positional `row.get`
    /// indices follow this order.
    fn select_columns(schema: DbSchema) -> String {
        format!(
            "form_key, editor_id, name, model_path, script, activation_sound, flags, is_marker, {}",
            activator_water_select_list(schema, "")
        )
    }

    pub fn get(
        form_key: &str,
        conn: &rusqlite::Connection,
        schema: DbSchema,
    ) -> anyhow::Result<Self> {
        Ok(conn.query_row(
            &format!(
                "SELECT {} FROM activators WHERE form_key = ?",
                Self::select_columns(schema)
            ),
            rusqlite::params![form_key],
            row_to_activator,
        )?)
    }

    pub fn get_all(conn: &rusqlite::Connection, schema: DbSchema) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM activators",
            Self::select_columns(schema)
        ))?;

        Ok(stmt
            .query_map([], row_to_activator)?
            .collect::<Result<Vec<_>, _>>()?)
    }
}
