//! `items` rows joined with their `weapons` / `armors` extension rows.

use anyhow::Context;

use super::schema::{DbSchema, item_marker_select_list};

#[derive(Clone, Debug, PartialEq)]
pub struct WeaponRow {
    pub damage: Option<i64>,
    pub fire_rate: Option<f64>,
    pub range: Option<f64>,
    pub accuracy: Option<f64>,
    pub ammo_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArmorRow {
    pub armor_rating: Option<i64>,
    pub body_slot: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemRow {
    pub form_key: String,
    /// Record signature: `WEAP`, `ARMO`, `AMMO`, `MISC`, `BOOK`, `ALCH`, `KEYM`, ...
    pub item_type: String,
    pub editor_id: Option<String>,
    pub name: Option<String>,
    pub value: i64,
    pub weight: f64,
    pub weapon: Option<WeaponRow>,
    pub armor: Option<ArmorRow>,
    pub model_path: Option<String>,
    /// Form keys of the record's keywords; `None` when the record stores no keyword list.
    pub keywords: Option<Vec<String>>,
    /// The CK-only marker flag; `None` (unknown) when the database predates the column, never
    /// defaulted: the consumer decides what an unknown flag means.
    pub is_marker: Option<bool>,
}

/// The keyword list of an item as the installer stores it: a JSON array of form keys, or NULL.
fn parse_keywords(form_key: &str, stored: Option<&str>) -> anyhow::Result<Option<Vec<String>>> {
    stored
        .map(serde_json::from_str)
        .transpose()
        .with_context(|| format!("item '{form_key}' has malformed keywords"))
}

impl ItemRow {
    pub fn list_all(conn: &rusqlite::Connection, schema: DbSchema) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(&format!(
            "SELECT i.form_key, i.item_type, i.editor_id, i.name, i.value, i.weight,
                    w.form_key IS NOT NULL, w.damage, w.fire_rate, w.range, w.accuracy, w.ammo_type,
                    a.form_key IS NOT NULL, a.armor_rating, a.body_slot,
                    i.model_path, i.keywords, {}
             FROM items i
             LEFT JOIN weapons w ON w.form_key = i.form_key
             LEFT JOIN armors a ON a.form_key = i.form_key
             ORDER BY i.form_key",
            item_marker_select_list(schema, "i.")
        ))?;
        let rows = stmt
            .query_map([], |row| {
                let weapon = if row.get::<_, bool>(6)? {
                    Some(WeaponRow {
                        damage: row.get(7)?,
                        fire_rate: row.get(8)?,
                        range: row.get(9)?,
                        accuracy: row.get(10)?,
                        ammo_type: row.get(11)?,
                    })
                } else {
                    None
                };
                let armor = if row.get::<_, bool>(12)? {
                    Some(ArmorRow {
                        armor_rating: row.get(13)?,
                        body_slot: row.get(14)?,
                    })
                } else {
                    None
                };
                let stored_keywords: Option<String> = row.get(16)?;
                Ok((
                    Self {
                        form_key: row.get(0)?,
                        item_type: row.get(1)?,
                        editor_id: row.get(2)?,
                        name: row.get(3)?,
                        value: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                        weight: row.get::<_, Option<f64>>(5)?.unwrap_or(0.0),
                        weapon,
                        armor,
                        model_path: row.get(15)?,
                        keywords: None,
                        is_marker: row.get(17)?,
                    },
                    stored_keywords,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(mut item, stored_keywords)| {
                item.keywords = parse_keywords(&item.form_key, stored_keywords.as_deref())?;
                Ok(item)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::database_with;
    use super::*;

    #[test]
    fn keywords_are_a_json_array_of_form_keys_or_absent() {
        assert_eq!(
            parse_keywords("0A:T.esm", Some(r#"["0B:T.esm","0C:T.esm"]"#)).unwrap(),
            Some(vec!["0B:T.esm".to_owned(), "0C:T.esm".to_owned()])
        );
        assert_eq!(
            parse_keywords("0A:T.esm", Some("[]")).unwrap(),
            Some(vec![])
        );
        assert_eq!(parse_keywords("0A:T.esm", None).unwrap(), None);
    }

    #[test]
    fn malformed_keywords_fail_naming_the_item() {
        for stored in ["not json", r#"{"0B:T.esm":1}"#, "[1,2]"] {
            let error = parse_keywords("0A:T.esm", Some(stored))
                .unwrap_err()
                .to_string();
            assert_eq!(error, "item '0A:T.esm' has malformed keywords", "{stored}");
        }
    }

    #[test]
    fn rows_carry_the_model_the_keywords_and_the_marker_flag() {
        let db = database_with(
            r#"INSERT INTO items (form_key, item_type, name, model_path, keywords, is_marker) VALUES
                ('0A:T.esm', 'MISC', 'Gadget', 'Props\Gadget.nif', '["0B:T.esm"]', 1),
                ('0C:T.esm', 'MISC', 'Plain', NULL, NULL, 0);"#,
        );

        let rows = db.get_items().unwrap();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].model_path.as_deref(), Some("Props\\Gadget.nif"));
        assert_eq!(rows[0].keywords, Some(vec!["0B:T.esm".to_owned()]));
        assert_eq!(rows[0].is_marker, Some(true));
        assert_eq!(rows[1].model_path, None);
        assert_eq!(rows[1].keywords, None);
        assert_eq!(rows[1].is_marker, Some(false));
    }

    #[test]
    fn a_database_without_the_marker_column_reads_the_flag_as_unknown() {
        let db = database_with(
            "ALTER TABLE items DROP COLUMN is_marker;
             INSERT INTO items (form_key, item_type, name) VALUES ('0A:T.esm', 'MISC', 'Gadget');",
        );

        let rows = db.get_items().unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].is_marker, None);
    }

    #[test]
    fn a_malformed_keyword_cell_fails_the_whole_read() {
        let db = database_with(
            "INSERT INTO items (form_key, item_type, name, keywords) VALUES ('0A:T.esm', 'MISC', 'Gadget', 'oops');",
        );

        let error = db.get_items().unwrap_err().to_string();

        assert!(error.contains("'0A:T.esm'"), "{error}");
    }
}
