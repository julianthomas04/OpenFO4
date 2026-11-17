//! The components a miscellaneous item is made of (`misc_components`) and the names of the
//! components (`components`).

use std::collections::HashMap;

use anyhow::Context;

use super::schema::DbSchema;

#[derive(Clone, Debug, PartialEq)]
pub struct ItemComponentRow {
    pub item_form_key: String,
    /// Form key of the component record.
    pub component: String,
    pub count: u32,
}

impl ItemComponentRow {
    /// Every component entry in record order; `None` for a database converted before the
    /// installer extracted them.
    pub fn list_all(
        conn: &rusqlite::Connection,
        schema: DbSchema,
    ) -> anyhow::Result<Option<Vec<Self>>> {
        if !schema.misc_components_table_present {
            return Ok(None);
        }
        let mut stmt = conn.prepare(
            "SELECT form_key, component, count FROM misc_components ORDER BY form_key, rowid",
        )?;
        let stored = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        stored
            .into_iter()
            .map(|(item_form_key, component, count)| {
                let count = u32::try_from(count).with_context(|| {
                    format!("item '{item_form_key}' lists component '{component}' {count} times")
                })?;
                Ok(Self {
                    item_form_key,
                    component,
                    count,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()
            .map(Some)
    }
}

/// The display name of each named component record, by form key.
pub fn component_names(conn: &rusqlite::Connection) -> anyhow::Result<HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT form_key, name FROM components WHERE name IS NOT NULL")?;
    Ok(stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<HashMap<_, _>, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::database_with;
    use super::*;

    #[test]
    fn entries_come_back_per_item_in_record_order() {
        let db = database_with(
            "INSERT INTO misc_components (form_key, component, count) VALUES
                ('0B:T.esm', '0F:T.esm', 4),
                ('0A:T.esm', '0E:T.esm', 2),
                ('0A:T.esm', '0D:T.esm', 9);",
        );

        let rows = db.get_item_components().unwrap().unwrap();

        let listed: Vec<(&str, &str, u32)> = rows
            .iter()
            .map(|row| {
                (
                    row.item_form_key.as_str(),
                    row.component.as_str(),
                    row.count,
                )
            })
            .collect();
        assert_eq!(
            listed,
            [
                ("0A:T.esm", "0E:T.esm", 2),
                ("0A:T.esm", "0D:T.esm", 9),
                ("0B:T.esm", "0F:T.esm", 4),
            ]
        );
    }

    #[test]
    fn a_database_without_the_table_has_no_component_data() {
        let db = database_with("DROP TABLE misc_components;");

        assert_eq!(db.get_item_components().unwrap(), None);
    }

    #[test]
    fn an_empty_table_is_no_components_rather_than_missing_data() {
        let db = database_with("");

        assert_eq!(db.get_item_components().unwrap(), Some(Vec::new()));
    }

    #[test]
    fn a_negative_count_fails_naming_the_item_and_component() {
        let db = database_with(
            "INSERT INTO misc_components (form_key, component, count) VALUES ('0A:T.esm', '0E:T.esm', -3);",
        );

        let error = db.get_item_components().unwrap_err().to_string();

        assert_eq!(error, "item '0A:T.esm' lists component '0E:T.esm' -3 times");
    }

    #[test]
    fn component_names_skip_records_without_a_name() {
        let db = database_with(
            "INSERT INTO components (form_key, name) VALUES ('0E:T.esm', 'Brass'), ('0D:T.esm', NULL);",
        );

        assert_eq!(
            db.get_component_names().unwrap(),
            HashMap::from([("0E:T.esm".to_owned(), "Brass".to_owned())])
        );
    }
}
