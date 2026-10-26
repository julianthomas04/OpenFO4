//! Sound descriptors with the output models and categories they reference, as plain data. A catalog
//! holds only what is reachable from the root descriptors it was loaded for. Values are raw Bethesda
//! units; interpretation belongs to the audio logic.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use crate::assets::db::music::{Condition, SoundCategory};
use crate::assets::db::music::{collect_rows, load_conditions, narrow, real32, required};

/// `descriptor_type` of a standard descriptor, the only kind that carries the BNAM mix values and looping flags.
pub const STANDARD_DESCRIPTOR_TYPE: u32 = 0x1EEF_540A;
/// `descriptor_type` of a compound descriptor, which plays its children.
pub const COMPOUND_DESCRIPTOR: u32 = 0x5465_1A43;

#[derive(Debug, Clone, PartialEq)]
pub struct SoundDescriptor {
    pub form_key: String,
    pub editor_id: Option<String>,
    pub descriptor_type: u32,
    /// The five mix values and the looping flags exist for standard descriptors only; other types read 0.
    pub base_descriptor: Option<String>,
    pub freq_shift_pct: i32,
    pub freq_variance_pct: i32,
    pub priority: i32,
    pub db_variance: i32,
    pub static_attenuation_centidb: i32,
    pub looping_flags: u32,
    pub category: Option<String>,
    pub output_model: Option<String>,
    /// Archive-relative sound file paths in record order.
    pub sound_files: Vec<String>,
    /// Compound children in record order.
    pub children: Vec<String>,
    pub conditions: Vec<Condition>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputChannel {
    /// Input channel of the output matrix the share is taken from.
    pub input_channel: u32,
    pub speaker: String,
    pub percent: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SoundOutputModel {
    pub form_key: String,
    pub editor_id: Option<String>,
    pub output_type: u32,
    pub static_attenuation_db: f32,
    pub channels: Vec<OutputChannel>,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SoundCatalog {
    /// Only the descriptors reachable from the roots through compound children and base descriptors.
    pub descriptors: HashMap<String, SoundDescriptor>,
    pub output_models: HashMap<String, SoundOutputModel>,
    pub categories: HashMap<String, SoundCategory>,
}

impl SoundCatalog {
    /// Loads the descriptors reachable from `roots` and the output models and category chains they
    /// name; errors when a record names one that does not exist, a row is malformed or a category
    /// chain has a cycle.
    pub(crate) fn load_reachable(
        conn: &Connection,
        roots: impl IntoIterator<Item = String>,
    ) -> anyhow::Result<Self> {
        let mut catalog = Self::default();
        let mut descriptors = load_descriptors(conn)?;
        let mut pending: Vec<String> = roots.into_iter().collect();
        while let Some(key) = pending.pop() {
            if catalog.descriptors.contains_key(&key) {
                continue;
            }
            let descriptor = descriptors.remove(&key).ok_or_else(|| {
                anyhow::anyhow!("a sound reference names unknown sound descriptor {key}")
            })?;
            pending.extend(descriptor.children.iter().cloned());
            pending.extend(descriptor.base_descriptor.iter().cloned());
            catalog.descriptors.insert(key, descriptor);
        }
        let mut models = load_output_models(conn)?;
        let mut categories = load_categories(conn)?;
        let mut pending_categories: Vec<String> = Vec::new();
        for descriptor in catalog.descriptors.values() {
            if let Some(model) = &descriptor.output_model
                && !catalog.output_models.contains_key(model)
            {
                let record = models.remove(model).ok_or_else(|| {
                    anyhow::anyhow!(
                        "sound descriptor {} names unknown output model {model}",
                        descriptor.form_key
                    )
                })?;
                catalog.output_models.insert(model.clone(), record);
            }
            pending_categories.extend(descriptor.category.clone());
        }
        while let Some(key) = pending_categories.pop() {
            if catalog.categories.contains_key(&key) {
                continue;
            }
            let category = categories.remove(&key).ok_or_else(|| {
                anyhow::anyhow!("a sound reference names unknown sound category {key}")
            })?;
            pending_categories.extend(category.parent.clone());
            catalog.categories.insert(key, category);
        }
        catalog.check_base_descriptors()?;
        catalog.check_category_chains()?;
        Ok(catalog)
    }

    fn check_base_descriptors(&self) -> anyhow::Result<()> {
        for descriptor in self.descriptors.values() {
            if let Some(base) = &descriptor.base_descriptor {
                anyhow::ensure!(
                    self.descriptors.contains_key(base),
                    "sound descriptor {} names unknown base descriptor {base}",
                    descriptor.form_key
                );
            }
        }
        Ok(())
    }

    fn check_category_chains(&self) -> anyhow::Result<()> {
        for key in self.categories.keys() {
            let mut seen = HashSet::new();
            let mut current = Some(key);
            while let Some(category) = current {
                anyhow::ensure!(
                    seen.insert(category),
                    "sound category {key} has a parent cycle"
                );
                current = self
                    .categories
                    .get(category)
                    .and_then(|c| c.parent.as_ref());
            }
        }
        Ok(())
    }
}

struct DescriptorRow {
    form_key: String,
    editor_id: Option<String>,
    descriptor_type: Option<i64>,
    base_descriptor: Option<String>,
    freq_shift_pct: Option<i64>,
    freq_variance_pct: Option<i64>,
    priority: Option<i64>,
    db_variance: Option<i64>,
    static_attenuation_centidb: Option<i64>,
    looping_flags: Option<i64>,
    category: Option<String>,
    output_model: Option<String>,
    sound_files: Option<String>,
}

fn load_descriptors(conn: &Connection) -> anyhow::Result<HashMap<String, SoundDescriptor>> {
    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    for (descriptor, child) in collect_rows(
        conn,
        "SELECT descriptor, child FROM sound_descriptor_children ORDER BY descriptor, ord",
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )? {
        children.entry(descriptor).or_default().push(child);
    }
    let mut conditions = load_conditions(
        conn,
        "sound_descriptor_conditions",
        "descriptor",
        "sound descriptor",
    )?;
    let rows = collect_rows(
        conn,
        "SELECT form_key, editor_id, descriptor_type, base_descriptor, freq_shift_pct, \
         freq_variance_pct, priority, db_variance, static_attenuation_centidb, looping_flags, \
         category, output_model, sound_files FROM sound_descriptors ORDER BY form_key",
        |row| {
            Ok(DescriptorRow {
                form_key: row.get(0)?,
                editor_id: row.get(1)?,
                descriptor_type: row.get(2)?,
                base_descriptor: row.get(3)?,
                freq_shift_pct: row.get(4)?,
                freq_variance_pct: row.get(5)?,
                priority: row.get(6)?,
                db_variance: row.get(7)?,
                static_attenuation_centidb: row.get(8)?,
                looping_flags: row.get(9)?,
                category: row.get(10)?,
                output_model: row.get(11)?,
                sound_files: row.get(12)?,
            })
        },
    )?;
    rows.into_iter()
        .map(|row| {
            let name = format!("sound descriptor {}", row.form_key);
            let sound_files = match &row.sound_files {
                Some(json) => serde_json::from_str::<Vec<String>>(json).map_err(|e| {
                    anyhow::anyhow!("{name}: sound_files is not a JSON array of paths: {e}")
                })?,
                None => Vec::new(),
            };
            let descriptor_type: u32 = narrow(
                required(row.descriptor_type, "descriptor_type", &name)?,
                "descriptor_type",
                &name,
            )?;
            let standard = descriptor_type == STANDARD_DESCRIPTOR_TYPE;
            let descriptor = SoundDescriptor {
                descriptor_type,
                freq_shift_pct: standard_only(
                    row.freq_shift_pct,
                    "freq_shift_pct",
                    &name,
                    standard,
                )?,
                freq_variance_pct: standard_only(
                    row.freq_variance_pct,
                    "freq_variance_pct",
                    &name,
                    standard,
                )?,
                priority: standard_only(row.priority, "priority", &name, standard)?,
                db_variance: standard_only(row.db_variance, "db_variance", &name, standard)?,
                static_attenuation_centidb: standard_only(
                    row.static_attenuation_centidb,
                    "static_attenuation_centidb",
                    &name,
                    standard,
                )?,
                looping_flags: standard_only(row.looping_flags, "looping_flags", &name, standard)?,
                children: children.remove(&row.form_key).unwrap_or_default(),
                conditions: conditions.remove(&row.form_key).unwrap_or_default(),
                form_key: row.form_key.clone(),
                editor_id: row.editor_id,
                base_descriptor: row.base_descriptor,
                category: row.category,
                output_model: row.output_model,
                sound_files,
            };
            Ok((row.form_key, descriptor))
        })
        .collect()
}

/// A value only standard descriptors carry: required there, 0 for the other types.
fn standard_only<T: TryFrom<i64> + Default>(
    value: Option<i64>,
    column: &str,
    row: &str,
    standard: bool,
) -> anyhow::Result<T> {
    match value {
        Some(value) => narrow(value, column, row),
        None if standard => anyhow::bail!("{row}: {column} is NULL"),
        None => Ok(T::default()),
    }
}

fn load_output_models(conn: &Connection) -> anyhow::Result<HashMap<String, SoundOutputModel>> {
    let mut channels: HashMap<String, Vec<OutputChannel>> = HashMap::new();
    for (model, input_channel, speaker, percent) in collect_rows(
        conn,
        "SELECT model, ord, speaker, percent FROM sound_output_channels ORDER BY model, ord, speaker",
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        },
    )? {
        let name = format!("sound output model {model}");
        channels.entry(model).or_default().push(OutputChannel {
            input_channel: narrow(input_channel, "ord", &name)?,
            speaker,
            percent: narrow(percent, "percent", &name)?,
        });
    }
    collect_rows(
        conn,
        "SELECT form_key, editor_id, output_type, static_attenuation_db FROM sound_output_models \
         ORDER BY form_key",
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<f64>>(3)?,
            ))
        },
    )?
    .into_iter()
    .map(|(form_key, editor_id, output_type, attenuation)| {
        let name = format!("sound output model {form_key}");
        let model = SoundOutputModel {
            output_type: narrow(
                required(output_type, "output_type", &name)?,
                "output_type",
                &name,
            )?,
            static_attenuation_db: real32(required(attenuation, "static_attenuation_db", &name)?),
            channels: channels.remove(&form_key).unwrap_or_default(),
            form_key: form_key.clone(),
            editor_id,
        };
        Ok((form_key, model))
    })
    .collect()
}

fn load_categories(conn: &Connection) -> anyhow::Result<HashMap<String, SoundCategory>> {
    Ok(collect_rows(
        conn,
        "SELECT form_key, editor_id, parent, static_attenuation FROM sound_categories \
         ORDER BY form_key",
        |row| {
            Ok(SoundCategory {
                form_key: row.get(0)?,
                editor_id: row.get(1)?,
                parent: row.get(2)?,
                static_multiplier: row.get::<_, Option<f64>>(3)?.map(real32),
            })
        },
    )?
    .into_iter()
    .map(|category| (category.form_key.clone(), category))
    .collect())
}

#[cfg(test)]
mod tests {
    use rusqlite::types::Value;

    use super::*;

    const SCHEMA: &str = include_str!("../../../../installer/file_converters/esm_schema.sql");

    const ROOT: &str = "000100:Test.esm";
    const CHILD: &str = "000101:Test.esm";
    const GRANDCHILD: &str = "000102:Test.esm";
    const BASE: &str = "000103:Test.esm";
    const OTHER_ROOT: &str = "000104:Test.esm";
    const UNREACHED: &str = "000105:Test.esm";
    const MODEL: &str = "000110:Test.esm";
    const CATEGORY: &str = "000120:Test.esm";
    const CATEGORY_PARENT: &str = "000121:Test.esm";

    fn text(value: &str) -> Value {
        Value::Text(value.to_string())
    }

    fn int(value: i64) -> Value {
        Value::Integer(value)
    }

    fn insert(conn: &Connection, table: &str, row: &[(&str, Value)]) {
        let columns = row.iter().map(|(c, _)| *c).collect::<Vec<_>>().join(", ");
        let placeholders = vec!["?"; row.len()].join(", ");
        conn.execute(
            &format!("INSERT INTO {table} ({columns}) VALUES ({placeholders})"),
            rusqlite::params_from_iter(row.iter().map(|(_, v)| v)),
        )
        .unwrap();
    }

    fn descriptor(conn: &Connection, key: &str, extra: &[(&str, Value)]) {
        let mut row = vec![
            ("form_key", text(key)),
            ("descriptor_type", int(i64::from(STANDARD_DESCRIPTOR_TYPE))),
            ("freq_shift_pct", int(0)),
            ("freq_variance_pct", int(0)),
            ("priority", int(0)),
            ("db_variance", int(0)),
            ("static_attenuation_centidb", int(0)),
            ("looping_flags", int(0)),
        ];
        row.extend(extra.iter().cloned());
        insert(conn, "sound_descriptors", &row);
    }

    fn child(conn: &Connection, parent: &str, ord: i64, child: &str) {
        insert(
            conn,
            "sound_descriptor_children",
            &[
                ("descriptor", text(parent)),
                ("ord", int(ord)),
                ("child", text(child)),
            ],
        );
    }

    fn category(conn: &Connection, key: &str, parent: Option<&str>) {
        let mut row = vec![
            ("form_key", text(key)),
            ("static_attenuation", Value::Real(0.5)),
        ];
        if let Some(parent) = parent {
            row.push(("parent", text(parent)));
        }
        insert(conn, "sound_categories", &row);
    }

    /// `ROOT` is compound over `CHILD`, which is compound over `GRANDCHILD`; `GRANDCHILD` has the model,
    /// the category chain and `BASE` as its base descriptor. `OTHER_ROOT` and `UNREACHED` stand apart.
    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        let compound = i64::from(COMPOUND_DESCRIPTOR);
        descriptor(&conn, ROOT, &[("descriptor_type", int(compound))]);
        descriptor(&conn, CHILD, &[("descriptor_type", int(compound))]);
        descriptor(
            &conn,
            GRANDCHILD,
            &[
                ("base_descriptor", text(BASE)),
                ("category", text(CATEGORY)),
                ("output_model", text(MODEL)),
            ],
        );
        descriptor(&conn, BASE, &[]);
        descriptor(&conn, OTHER_ROOT, &[]);
        descriptor(
            &conn,
            UNREACHED,
            &[("output_model", text("0FFFFF:Test.esm"))],
        );
        child(&conn, ROOT, 0, CHILD);
        child(&conn, CHILD, 0, GRANDCHILD);
        insert(
            &conn,
            "sound_output_models",
            &[
                ("form_key", text(MODEL)),
                ("output_type", int(1)),
                ("static_attenuation_db", Value::Real(0.0)),
            ],
        );
        category(&conn, CATEGORY, Some(CATEGORY_PARENT));
        category(&conn, CATEGORY_PARENT, None);
        conn
    }

    fn load(conn: &Connection, roots: &[&str]) -> anyhow::Result<SoundCatalog> {
        SoundCatalog::load_reachable(conn, roots.iter().map(ToString::to_string))
    }

    fn keys(catalog: &SoundCatalog) -> Vec<&str> {
        let mut keys: Vec<&str> = catalog.descriptors.keys().map(String::as_str).collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn loading_follows_children_and_base_descriptors_transitively() {
        let catalog = load(&connection(), &[ROOT]).unwrap();

        assert_eq!(keys(&catalog), [ROOT, CHILD, GRANDCHILD, BASE]);
        assert_eq!(catalog.descriptors[ROOT].children, [CHILD]);
    }

    #[test]
    fn loading_pulls_in_the_models_and_category_chain_of_reached_descriptors() {
        let catalog = load(&connection(), &[ROOT]).unwrap();

        assert!(catalog.output_models.contains_key(MODEL));
        assert_eq!(catalog.categories.len(), 2);
        assert_eq!(
            catalog.categories[CATEGORY].parent.as_deref(),
            Some(CATEGORY_PARENT)
        );
    }

    #[test]
    fn several_roots_load_the_union_of_their_descriptors() {
        let catalog = load(&connection(), &[GRANDCHILD, OTHER_ROOT]).unwrap();

        assert_eq!(keys(&catalog), [GRANDCHILD, BASE, OTHER_ROOT]);
    }

    #[test]
    fn no_roots_load_nothing() {
        let catalog = load(&connection(), &[]).unwrap();

        assert_eq!(catalog, SoundCatalog::default());
    }

    #[test]
    fn unreached_records_are_neither_loaded_nor_validated() {
        let catalog = load(&connection(), &[OTHER_ROOT]).unwrap();

        assert_eq!(keys(&catalog), [OTHER_ROOT]);
        assert!(catalog.output_models.is_empty());
        assert!(catalog.categories.is_empty());
    }

    #[test]
    fn an_unknown_root_is_an_error_naming_it() {
        let error = load(&connection(), &["0DEAD1:Test.esm"]).unwrap_err();

        assert!(error.to_string().contains("0DEAD1:Test.esm"), "{error}");
    }

    #[test]
    fn an_unknown_child_is_an_error_naming_it() {
        let conn = connection();
        child(&conn, OTHER_ROOT, 0, "0DEAD2:Test.esm");

        let error = load(&conn, &[OTHER_ROOT]).unwrap_err();

        assert!(error.to_string().contains("0DEAD2:Test.esm"), "{error}");
    }

    #[test]
    fn a_category_parent_cycle_is_an_error() {
        let conn = connection();
        conn.execute(
            "UPDATE sound_categories SET parent = ?1 WHERE form_key = ?2",
            [CATEGORY, CATEGORY_PARENT],
        )
        .unwrap();

        let error = load(&conn, &[GRANDCHILD]).unwrap_err();

        assert!(error.to_string().contains("parent cycle"), "{error}");
    }
}
