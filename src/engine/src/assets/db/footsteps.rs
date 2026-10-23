//! Footstep tables as plain data: the default footstep set, footstep sets per gait, impact data sets,
//! the material parent map and the sound records an impact plays. Values are raw Bethesda units;
//! interpretation belongs to the footstep audio logic.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use crate::assets::db::music::{Condition, SoundCategory};
use crate::assets::db::music::{collect_rows, load_conditions, narrow, real32, required};
use crate::assets::db::schema::require_footstep_tables;
use crate::footsteps::events::{FootTag, Gait};

const DEFAULT_FOOTSTEP_SET_SLOT: &str = "DFTS";
/// `descriptor_type` of a standard descriptor, the only kind that carries the BNAM mix values and looping flags.
pub const STANDARD_DESCRIPTOR_TYPE: u32 = 0x1EEF_540A;
const PLAYER_GAITS: [Gait; 3] = [Gait::Run, Gait::Sprint, Gait::Sneak];

#[derive(Debug, Clone, PartialEq)]
pub struct FootstepSet {
    pub form_key: String,
    pub editor_id: Option<String>,
    /// Footstep form keys per gait in the set's logical order.
    pub gaits: HashMap<Gait, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Footstep {
    pub form_key: String,
    pub editor_id: Option<String>,
    pub tag: FootTag,
    pub impact_data_set: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaterialImpact {
    /// `material_types.material_hash` of the listed material.
    pub material: u32,
    pub impact: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImpactDataSet {
    pub form_key: String,
    pub editor_id: Option<String>,
    /// Material to impact pairs in record order.
    pub entries: Vec<MaterialImpact>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Impact {
    pub form_key: String,
    pub editor_id: Option<String>,
    /// The sound descriptor played for the player.
    pub player_sound: Option<String>,
}

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
pub struct FootstepCatalog {
    pub default_footstep_set: String,
    pub footstep_sets: HashMap<String, FootstepSet>,
    pub footsteps: HashMap<String, Footstep>,
    pub impact_data_sets: HashMap<String, ImpactDataSet>,
    /// Material type form key to its hash.
    pub material_hashes: HashMap<String, u32>,
    /// Material hash to the hash of its parent material, `None` for a root material.
    pub material_parents: HashMap<u32, Option<u32>>,
    /// Material hash to the material's editor id, for diagnostics.
    pub material_names: HashMap<u32, String>,
    pub impacts: HashMap<String, Impact>,
    /// Only the descriptors an impact reaches through its player sound and compound children.
    pub sound_descriptors: HashMap<String, SoundDescriptor>,
    pub output_models: HashMap<String, SoundOutputModel>,
    pub sound_categories: HashMap<String, SoundCategory>,
    /// Landscape texture form key to the hash of its material, `None` when it names none.
    pub landscape_materials: HashMap<String, Option<u32>>,
}

impl FootstepCatalog {
    /// The hash with the material's editor id when the material table has it, for log lines.
    pub fn material_label(&self, hash: u32) -> String {
        match self.material_names.get(&hash) {
            Some(name) => format!("{name} ({hash:08X})"),
            None => format!("{hash:08X} (not in the material table)"),
        }
    }

    /// Loads everything once; errors when the installer has not extracted the footstep tables, a row is
    /// malformed or a record names one that does not exist.
    pub(crate) fn from_connection(conn: &Connection) -> anyhow::Result<Self> {
        require_footstep_tables(conn)?;
        let materials = load_materials(conn)?;
        let impacts = load_impacts(conn)?;
        let (footsteps, foreign_tags) = load_footsteps(conn)?;
        let mut catalog = Self {
            default_footstep_set: load_default_set(conn)?,
            footstep_sets: load_sets(conn)?,
            footsteps,
            impact_data_sets: load_impact_data_sets(conn, &materials.hashes)?,
            material_parents: materials.parents,
            material_names: materials.names,
            landscape_materials: load_landscape_materials(conn, &materials.hashes)?,
            material_hashes: materials.hashes,
            impacts,
            sound_descriptors: HashMap::new(),
            output_models: HashMap::new(),
            sound_categories: HashMap::new(),
        };
        catalog.drop_foreign_tag_footsteps(&foreign_tags)?;
        catalog.check_references()?;
        catalog.load_reachable_sounds(conn)?;
        catalog.check_sound_references()?;
        catalog.check_chains()?;
        Ok(catalog)
    }

    /// Footsteps whose tag is not a player `FootTag` (creature, power armor and NPC animation events;
    /// the vanilla default set itself lists power armor start and stop steps) can never match a
    /// player event, so they are removed from every set. The player must keep a step in each of the
    /// default set's gaits (`PLAYER_GAITS`), otherwise a tag-parsing failure would go unnoticed.
    fn drop_foreign_tag_footsteps(
        &mut self,
        foreign_tags: &HashMap<String, String>,
    ) -> anyhow::Result<()> {
        for set in self.footstep_sets.values_mut() {
            for footsteps in set.gaits.values_mut() {
                footsteps.retain(|key| !foreign_tags.contains_key(key));
            }
            set.gaits.retain(|_, footsteps| !footsteps.is_empty());
        }
        if let Some(default) = self.footstep_sets.get(&self.default_footstep_set) {
            for gait in PLAYER_GAITS {
                anyhow::ensure!(
                    default.gaits.contains_key(&gait),
                    "the default footstep set {} has no footstep with a player tag for gait {gait:?}",
                    self.default_footstep_set
                );
            }
        }
        Ok(())
    }

    fn check_references(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.footstep_sets.contains_key(&self.default_footstep_set),
            "default footstep set {} is not a footstep set",
            self.default_footstep_set
        );
        for set in self.footstep_sets.values() {
            for footstep in set.gaits.values().flatten() {
                ensure_known(
                    self.footsteps.contains_key(footstep),
                    &format!("footstep set {}", set.form_key),
                    "footstep",
                    footstep,
                )?;
            }
        }
        for footstep in self.footsteps.values() {
            if let Some(data_set) = &footstep.impact_data_set {
                ensure_known(
                    self.impact_data_sets.contains_key(data_set),
                    &format!("footstep {}", footstep.form_key),
                    "impact data set",
                    data_set,
                )?;
            }
        }
        for data_set in self.impact_data_sets.values() {
            for entry in &data_set.entries {
                ensure_known(
                    self.impacts.contains_key(&entry.impact),
                    &format!("impact data set {}", data_set.form_key),
                    "impact",
                    &entry.impact,
                )?;
            }
        }
        Ok(())
    }

    fn load_reachable_sounds(&mut self, conn: &Connection) -> anyhow::Result<()> {
        let mut descriptors = load_descriptors(conn)?;
        let mut pending: Vec<String> = self
            .impacts
            .values()
            .filter_map(|impact| impact.player_sound.clone())
            .collect();
        while let Some(key) = pending.pop() {
            if self.sound_descriptors.contains_key(&key) {
                continue;
            }
            let descriptor = descriptors.remove(&key).ok_or_else(|| {
                anyhow::anyhow!("a sound reference names unknown sound descriptor {key}")
            })?;
            pending.extend(descriptor.children.iter().cloned());
            pending.extend(descriptor.base_descriptor.iter().cloned());
            self.sound_descriptors.insert(key, descriptor);
        }
        let mut models = load_output_models(conn)?;
        let mut categories = load_categories(conn)?;
        let mut pending_categories: Vec<String> = Vec::new();
        for descriptor in self.sound_descriptors.values() {
            if let Some(model) = &descriptor.output_model
                && !self.output_models.contains_key(model)
            {
                let record = models.remove(model).ok_or_else(|| {
                    anyhow::anyhow!(
                        "sound descriptor {} names unknown output model {model}",
                        descriptor.form_key
                    )
                })?;
                self.output_models.insert(model.clone(), record);
            }
            pending_categories.extend(descriptor.category.clone());
        }
        while let Some(key) = pending_categories.pop() {
            if self.sound_categories.contains_key(&key) {
                continue;
            }
            let category = categories.remove(&key).ok_or_else(|| {
                anyhow::anyhow!("a sound reference names unknown sound category {key}")
            })?;
            pending_categories.extend(category.parent.clone());
            self.sound_categories.insert(key, category);
        }
        Ok(())
    }

    fn check_sound_references(&self) -> anyhow::Result<()> {
        for descriptor in self.sound_descriptors.values() {
            if let Some(base) = &descriptor.base_descriptor {
                ensure_known(
                    self.sound_descriptors.contains_key(base),
                    &format!("sound descriptor {}", descriptor.form_key),
                    "base descriptor",
                    base,
                )?;
            }
        }
        Ok(())
    }

    fn check_chains(&self) -> anyhow::Result<()> {
        for &material in self.material_parents.keys() {
            let mut seen = HashSet::new();
            let mut current = Some(material);
            while let Some(hash) = current {
                anyhow::ensure!(
                    seen.insert(hash),
                    "material {material:08X} has a parent cycle"
                );
                current = self.material_parents.get(&hash).copied().flatten();
            }
        }
        for key in self.sound_categories.keys() {
            let mut seen = HashSet::new();
            let mut current = Some(key);
            while let Some(category) = current {
                anyhow::ensure!(
                    seen.insert(category),
                    "sound category {key} has a parent cycle"
                );
                current = self
                    .sound_categories
                    .get(category)
                    .and_then(|c| c.parent.as_ref());
            }
        }
        Ok(())
    }
}

fn ensure_known(known: bool, owner: &str, kind: &str, key: &str) -> anyhow::Result<()> {
    anyhow::ensure!(known, "{owner} names unknown {kind} {key}");
    Ok(())
}

struct Materials {
    hashes: HashMap<String, u32>,
    names: HashMap<u32, String>,
    parents: HashMap<u32, Option<u32>>,
}

fn load_materials(conn: &Connection) -> anyhow::Result<Materials> {
    let rows = collect_rows(
        conn,
        "SELECT form_key, material_hash, parent, editor_id FROM material_types ORDER BY form_key",
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        },
    )?;
    let mut hashes = HashMap::new();
    let mut names = HashMap::new();
    let mut parent_keys = Vec::new();
    for (form_key, hash, parent, editor_id) in rows {
        let name = format!("material type {form_key}");
        let hash: u32 = narrow(
            required(hash, "material_hash", &name)?,
            "material_hash",
            &name,
        )?;
        hashes.insert(form_key.clone(), hash);
        names.insert(hash, editor_id.unwrap_or_else(|| form_key.clone()));
        parent_keys.push((form_key, parent));
    }
    let mut parents = HashMap::new();
    for (form_key, parent) in parent_keys {
        let parent_hash = parent
            .map(|key| {
                hashes.get(&key).copied().ok_or_else(|| {
                    anyhow::anyhow!("material type {form_key} names unknown parent material {key}")
                })
            })
            .transpose()?;
        parents.insert(hashes[&form_key], parent_hash);
    }
    Ok(Materials {
        hashes,
        names,
        parents,
    })
}

fn load_default_set(conn: &Connection) -> anyhow::Result<String> {
    let mut targets = collect_rows(
        conn,
        &format!(
            "SELECT target FROM default_object_slots WHERE slot = '{DEFAULT_FOOTSTEP_SET_SLOT}'"
        ),
        |row| row.get::<_, Option<String>>(0),
    )?;
    anyhow::ensure!(
        targets.len() == 1,
        "default_object_slots holds {} {DEFAULT_FOOTSTEP_SET_SLOT} entries, expected 1",
        targets.len()
    );
    required(
        targets.remove(0),
        "target",
        &format!("default object slot {DEFAULT_FOOTSTEP_SET_SLOT}"),
    )
}

fn load_sets(conn: &Connection) -> anyhow::Result<HashMap<String, FootstepSet>> {
    let mut sets: HashMap<String, FootstepSet> = collect_rows(
        conn,
        "SELECT form_key, editor_id FROM footstep_sets ORDER BY form_key",
        |row| {
            Ok(FootstepSet {
                form_key: row.get(0)?,
                editor_id: row.get(1)?,
                gaits: HashMap::new(),
            })
        },
    )?
    .into_iter()
    .map(|set| (set.form_key.clone(), set))
    .collect();
    let entries = collect_rows(
        conn,
        "SELECT set_key, gait, footstep FROM footstep_set_entries ORDER BY set_key, gait, ord",
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        },
    )?;
    for (set_key, gait, footstep) in entries {
        let name = format!("footstep set {set_key}");
        let gait =
            Gait::parse(&gait).ok_or_else(|| anyhow::anyhow!("{name}: unknown gait `{gait}`"))?;
        let set = sets
            .get_mut(&set_key)
            .ok_or_else(|| anyhow::anyhow!("footstep set entries name unknown set {set_key}"))?;
        set.gaits.entry(gait).or_default().push(footstep);
    }
    Ok(sets)
}

/// The footsteps with a player tag, and the raw tag of every other footstep by form key.
fn load_footsteps(
    conn: &Connection,
) -> anyhow::Result<(HashMap<String, Footstep>, HashMap<String, String>)> {
    let rows = collect_rows(
        conn,
        "SELECT form_key, editor_id, tag, impact_data_set FROM footsteps ORDER BY form_key",
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        },
    )?;
    let mut footsteps = HashMap::new();
    let mut foreign_tags = HashMap::new();
    for (form_key, editor_id, tag, impact_data_set) in rows {
        let name = format!("footstep {form_key}");
        let tag = required(tag, "tag", &name)?;
        match FootTag::parse(&tag) {
            Some(parsed) => {
                footsteps.insert(
                    form_key.clone(),
                    Footstep {
                        form_key,
                        editor_id,
                        tag: parsed,
                        impact_data_set,
                    },
                );
            }
            None => {
                foreign_tags.insert(form_key, tag);
            }
        }
    }
    Ok((footsteps, foreign_tags))
}

fn load_impact_data_sets(
    conn: &Connection,
    materials: &HashMap<String, u32>,
) -> anyhow::Result<HashMap<String, ImpactDataSet>> {
    let mut sets: HashMap<String, ImpactDataSet> = collect_rows(
        conn,
        "SELECT form_key, editor_id FROM impact_data_sets ORDER BY form_key",
        |row| {
            Ok(ImpactDataSet {
                form_key: row.get(0)?,
                editor_id: row.get(1)?,
                entries: Vec::new(),
            })
        },
    )?
    .into_iter()
    .map(|set| (set.form_key.clone(), set))
    .collect();
    let entries = collect_rows(
        conn,
        "SELECT set_key, material, impact FROM impact_data_set_entries ORDER BY set_key, ord",
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        },
    )?;
    for (set_key, material, impact) in entries {
        let name = format!("impact data set {set_key}");
        let material = *materials
            .get(&material)
            .ok_or_else(|| anyhow::anyhow!("{name} names unknown material {material}"))?;
        sets.get_mut(&set_key)
            .ok_or_else(|| anyhow::anyhow!("impact data set entries name unknown set {set_key}"))?
            .entries
            .push(MaterialImpact { material, impact });
    }
    Ok(sets)
}

fn load_impacts(conn: &Connection) -> anyhow::Result<HashMap<String, Impact>> {
    Ok(collect_rows(
        conn,
        "SELECT form_key, editor_id, player_sound FROM impacts ORDER BY form_key",
        |row| {
            Ok(Impact {
                form_key: row.get(0)?,
                editor_id: row.get(1)?,
                player_sound: row.get(2)?,
            })
        },
    )?
    .into_iter()
    .map(|impact| (impact.form_key.clone(), impact))
    .collect())
}

fn load_landscape_materials(
    conn: &Connection,
    materials: &HashMap<String, u32>,
) -> anyhow::Result<HashMap<String, Option<u32>>> {
    collect_rows(
        conn,
        "SELECT form_key, material_type FROM landscape_textures ORDER BY form_key",
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
    )?
    .into_iter()
    .map(|(form_key, material)| {
        let hash = material
            .map(|key| {
                materials.get(&key).copied().ok_or_else(|| {
                    anyhow::anyhow!("landscape texture {form_key} names unknown material {key}")
                })
            })
            .transpose()?;
        Ok((form_key, hash))
    })
    .collect()
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

    const SET: &str = "000100:Test.esm";
    const STEP_RIGHT: &str = "000101:Test.esm";
    const STEP_LEFT: &str = "000102:Test.esm";
    const DATA_SET: &str = "000110:Test.esm";
    const IMPACT: &str = "000120:Test.esm";
    const COMPOUND: &str = "000130:Test.esm";
    const LAYER_FIRST_PERSON: &str = "000131:Test.esm";
    const LAYER_THIRD_PERSON: &str = "000132:Test.esm";
    const MODEL: &str = "000140:Test.esm";
    const CATEGORY_LEAF: &str = "000150:Test.esm";
    const CATEGORY_ROOT: &str = "000151:Test.esm";
    const MATERIAL_DIRT: &str = "000160:Test.esm";
    const MATERIAL_GRASS: &str = "000161:Test.esm";
    const DIRT_HASH: i64 = 0x1234_5678;
    const GRASS_HASH: i64 = 0xFEDC_BA98;
    const LTEX: &str = "000170:Test.esm";

    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        conn
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

    fn text(s: &str) -> Value {
        Value::Text(s.to_string())
    }

    fn int(n: i64) -> Value {
        Value::Integer(n)
    }

    fn descriptor(conn: &Connection, key: &str, extra: &[(&str, Value)]) {
        let mut row = vec![
            ("form_key", text(key)),
            ("editor_id", text("Sound")),
            ("descriptor_type", int(0x1EEF_540A)),
            ("freq_shift_pct", int(0)),
            ("freq_variance_pct", int(0)),
            ("priority", int(0)),
            ("db_variance", int(0)),
            ("static_attenuation_centidb", int(0)),
            ("looping_flags", int(0)),
        ];
        row.retain(|(column, _)| !extra.iter().any(|(name, _)| name == column));
        row.extend(extra.iter().cloned());
        insert(conn, "sound_descriptors", &row);
    }

    fn condition(conn: &Connection, descriptor: &str, function: i64, value: f64) {
        insert(
            conn,
            "sound_descriptor_conditions",
            &[
                ("descriptor", text(descriptor)),
                ("ord", int(0)),
                ("function", int(function)),
                ("compare_op", int(0)),
                ("or_flag", int(0)),
                ("compare_value", Value::Real(value)),
                ("run_on", int(0)),
            ],
        );
    }

    fn material(conn: &Connection, key: &str, hash: i64, parent: Option<&str>) {
        let mut row = vec![
            ("form_key", text(key)),
            ("editor_id", text(&format!("Editor{key}"))),
            ("material_hash", int(hash)),
        ];
        if let Some(parent) = parent {
            row.push(("parent", text(parent)));
        }
        insert(conn, "material_types", &row);
    }

    /// Dirt (root) and Grass (parent Dirt); one run set stepping right then left, whose steps map
    /// Dirt to an impact playing a compound sound with a first-person and a third-person layer.
    fn world(conn: &Connection) {
        insert(
            conn,
            "default_object_slots",
            &[
                ("manager", text("000001:Test.esm")),
                ("slot", text("DFTS")),
                ("target", text(SET)),
            ],
        );
        insert(
            conn,
            "footstep_sets",
            &[("form_key", text(SET)), ("editor_id", text("TestSet"))],
        );
        for (ord, (step, tag)) in [(STEP_RIGHT, "FootRight"), (STEP_LEFT, "FootLeft")]
            .into_iter()
            .enumerate()
        {
            insert(
                conn,
                "footstep_set_entries",
                &[
                    ("set_key", text(SET)),
                    ("gait", text("run")),
                    ("ord", int(ord as i64)),
                    ("footstep", text(step)),
                ],
            );
            insert(
                conn,
                "footsteps",
                &[
                    ("form_key", text(step)),
                    ("editor_id", text(tag)),
                    ("tag", text(tag)),
                    ("impact_data_set", text(DATA_SET)),
                ],
            );
        }
        add_gait(conn, "sprint");
        add_gait(conn, "sneak");
        insert(
            conn,
            "impact_data_sets",
            &[("form_key", text(DATA_SET)), ("editor_id", text("Data"))],
        );
        material(conn, MATERIAL_DIRT, DIRT_HASH, None);
        material(conn, MATERIAL_GRASS, GRASS_HASH, Some(MATERIAL_DIRT));
        insert(
            conn,
            "impact_data_set_entries",
            &[
                ("set_key", text(DATA_SET)),
                ("ord", int(0)),
                ("material", text(MATERIAL_DIRT)),
                ("impact", text(IMPACT)),
            ],
        );
        insert(
            conn,
            "impacts",
            &[
                ("form_key", text(IMPACT)),
                ("editor_id", text("Impact")),
                ("player_sound", text(COMPOUND)),
            ],
        );
        descriptor(
            conn,
            COMPOUND,
            &[
                ("descriptor_type", int(0x5465_1A43)),
                ("freq_shift_pct", Value::Null),
                ("freq_variance_pct", Value::Null),
                ("priority", Value::Null),
                ("db_variance", Value::Null),
                ("static_attenuation_centidb", Value::Null),
                ("category", text(CATEGORY_LEAF)),
            ],
        );
        for (ord, child) in [LAYER_FIRST_PERSON, LAYER_THIRD_PERSON]
            .into_iter()
            .enumerate()
        {
            insert(
                conn,
                "sound_descriptor_children",
                &[
                    ("descriptor", text(COMPOUND)),
                    ("ord", int(ord as i64)),
                    ("child", text(child)),
                ],
            );
        }
        descriptor(
            conn,
            LAYER_FIRST_PERSON,
            &[
                ("freq_shift_pct", int(-5)),
                ("freq_variance_pct", int(3)),
                ("db_variance", int(2)),
                ("static_attenuation_centidb", int(-250)),
                ("category", text(CATEGORY_LEAF)),
                ("output_model", text(MODEL)),
                (
                    "sound_files",
                    text(r#"["Sound\\Fx\\step1.xwm","Sound\\Fx\\step2.xwm"]"#),
                ),
            ],
        );
        condition(conn, LAYER_FIRST_PERSON, 391, 1.0);
        descriptor(
            conn,
            LAYER_THIRD_PERSON,
            &[("sound_files", text(r#"["Sound\\Fx\\other.xwm"]"#))],
        );
        insert(
            conn,
            "sound_output_models",
            &[
                ("form_key", text(MODEL)),
                ("output_type", int(1)),
                ("static_attenuation_db", Value::Real(1.75)),
            ],
        );
        for (speaker, percent) in [("L", 75), ("R", 75)] {
            insert(
                conn,
                "sound_output_channels",
                &[
                    ("model", text(MODEL)),
                    ("ord", int(0)),
                    ("speaker", text(speaker)),
                    ("percent", int(percent)),
                ],
            );
        }
        insert(
            conn,
            "sound_categories",
            &[
                ("form_key", text(CATEGORY_LEAF)),
                ("parent", text(CATEGORY_ROOT)),
                ("static_attenuation", Value::Real(0.5)),
            ],
        );
        insert(
            conn,
            "sound_categories",
            &[
                ("form_key", text(CATEGORY_ROOT)),
                ("static_attenuation", Value::Real(0.25)),
            ],
        );
        insert(
            conn,
            "landscape_textures",
            &[
                ("form_key", text(LTEX)),
                ("material_type", text(MATERIAL_GRASS)),
            ],
        );
    }

    fn loaded() -> anyhow::Result<FootstepCatalog> {
        load_after(|_| {})
    }

    fn load_after(change: impl FnOnce(&Connection)) -> anyhow::Result<FootstepCatalog> {
        let conn = connection();
        world(&conn);
        change(&conn);
        FootstepCatalog::from_connection(&conn)
    }

    fn error_text(result: anyhow::Result<FootstepCatalog>) -> String {
        format!("{:#}", result.unwrap_err())
    }

    fn execute(conn: &Connection, sql: &str) {
        conn.execute_batch(sql).unwrap();
    }

    #[test]
    fn default_set_lists_footsteps_per_gait_in_order_with_parsed_tags() {
        let catalog = loaded().unwrap();

        assert_eq!(catalog.default_footstep_set, SET);
        let set = &catalog.footstep_sets[SET];
        assert_eq!(set.editor_id.as_deref(), Some("TestSet"));
        assert_eq!(set.gaits[&Gait::Run], [STEP_RIGHT, STEP_LEFT]);
        assert_eq!(set.gaits[&Gait::Sprint], [STEP_RIGHT]);
        assert_eq!(catalog.footsteps[STEP_RIGHT].tag, FootTag::FootRight);
        assert_eq!(catalog.footsteps[STEP_LEFT].tag, FootTag::FootLeft);
        assert_eq!(
            catalog.footsteps[STEP_RIGHT].impact_data_set.as_deref(),
            Some(DATA_SET)
        );
    }

    #[test]
    fn impact_data_set_resolves_material_form_keys_to_hashes_in_record_order() {
        let catalog = load_after(|conn| {
            insert(
                conn,
                "impact_data_set_entries",
                &[
                    ("set_key", text(DATA_SET)),
                    ("ord", int(1)),
                    ("material", text(MATERIAL_GRASS)),
                    ("impact", text(IMPACT)),
                ],
            );
        })
        .unwrap();

        let entries = &catalog.impact_data_sets[DATA_SET].entries;
        let materials: Vec<u32> = entries.iter().map(|e| e.material).collect();
        assert_eq!(materials, [0x1234_5678, 0xFEDC_BA98]);
        assert_eq!(entries[0].impact, IMPACT);
    }

    #[test]
    fn material_parents_map_hashes_and_root_materials_have_none() {
        let catalog = loaded().unwrap();

        assert_eq!(catalog.material_parents[&0x1234_5678], None);
        assert_eq!(catalog.material_parents[&0xFEDC_BA98], Some(0x1234_5678));
    }

    #[test]
    fn material_labels_carry_the_editor_id_or_say_the_hash_is_unknown() {
        let catalog = loaded().unwrap();

        assert_eq!(
            catalog.material_label(0x1234_5678),
            format!("Editor{MATERIAL_DIRT} (12345678)")
        );
        assert_eq!(
            catalog.material_label(0xFCB3_7EA0),
            "FCB37EA0 (not in the material table)"
        );
    }

    #[test]
    fn landscape_textures_map_to_their_material_hash() {
        let catalog = load_after(|conn| {
            insert(
                conn,
                "landscape_textures",
                &[("form_key", text("000171:Test.esm"))],
            );
        })
        .unwrap();

        assert_eq!(catalog.landscape_materials[LTEX], Some(0xFEDC_BA98));
        assert_eq!(catalog.landscape_materials["000171:Test.esm"], None);
    }

    #[test]
    fn player_sound_reaches_children_with_their_values_files_and_conditions() {
        let catalog = loaded().unwrap();

        assert_eq!(
            catalog.impacts[IMPACT].player_sound.as_deref(),
            Some(COMPOUND)
        );
        let compound = &catalog.sound_descriptors[COMPOUND];
        assert_eq!(compound.children, [LAYER_FIRST_PERSON, LAYER_THIRD_PERSON]);
        assert!(compound.sound_files.is_empty());
        assert_eq!(compound.static_attenuation_centidb, 0);
        let layer = &catalog.sound_descriptors[LAYER_FIRST_PERSON];
        assert_eq!(layer.freq_shift_pct, -5);
        assert_eq!(layer.freq_variance_pct, 3);
        assert_eq!(layer.db_variance, 2);
        assert_eq!(layer.static_attenuation_centidb, -250);
        assert_eq!(
            layer.sound_files,
            [r"Sound\Fx\step1.xwm", r"Sound\Fx\step2.xwm"]
        );
        assert_eq!(layer.conditions.len(), 1);
        assert_eq!(layer.conditions[0].function, 391);
        assert!(
            catalog.sound_descriptors[LAYER_THIRD_PERSON]
                .conditions
                .is_empty()
        );
    }

    #[test]
    fn output_model_and_category_chain_are_loaded_for_reachable_sounds() {
        let catalog = loaded().unwrap();

        let model = &catalog.output_models[MODEL];
        assert_eq!(model.output_type, 1);
        assert_eq!(model.static_attenuation_db.to_bits(), 1.75_f32.to_bits());
        let channels: Vec<(u32, &str, i32)> = model
            .channels
            .iter()
            .map(|c| (c.input_channel, c.speaker.as_str(), c.percent))
            .collect();
        assert_eq!(channels, [(0, "L", 75), (0, "R", 75)]);
        let leaf = &catalog.sound_categories[CATEGORY_LEAF];
        assert_eq!(leaf.parent.as_deref(), Some(CATEGORY_ROOT));
        assert_eq!(leaf.static_multiplier, Some(0.5));
        assert_eq!(
            catalog.sound_categories[CATEGORY_ROOT].static_multiplier,
            Some(0.25)
        );
    }

    #[test]
    fn unreachable_sound_records_are_neither_loaded_nor_validated() {
        let catalog = load_after(|conn| {
            descriptor(
                conn,
                "000190:Test.esm",
                &[("output_model", text("0FFFFF:Test.esm"))],
            );
        })
        .unwrap();

        assert!(!catalog.sound_descriptors.contains_key("000190:Test.esm"));
    }

    #[test]
    fn missing_column_error_names_it_and_asks_for_the_installer() {
        let conn = connection();
        world(&conn);
        execute(&conn, "ALTER TABLE footsteps DROP COLUMN tag;");

        let message = error_text(FootstepCatalog::from_connection(&conn));

        assert!(message.contains("`footsteps`"), "{message}");
        assert!(message.contains("tag"), "{message}");
        assert!(message.contains("re-run the installer"), "{message}");
    }

    #[test]
    fn missing_table_reads_as_all_columns_missing() {
        let conn = connection();
        world(&conn);
        execute(&conn, "DROP TABLE sound_output_channels;");

        let message = error_text(FootstepCatalog::from_connection(&conn));

        assert!(message.contains("`sound_output_channels`"), "{message}");
        assert!(
            message.contains("model, ord, speaker, percent"),
            "{message}"
        );
    }

    #[test]
    fn dangling_footstep_in_a_set_is_an_error_naming_it() {
        let message = error_text(load_after(|conn| {
            insert(
                conn,
                "footstep_set_entries",
                &[
                    ("set_key", text(SET)),
                    ("gait", text("swim")),
                    ("ord", int(0)),
                    ("footstep", text("0DEAD1:Test.esm")),
                ],
            );
        }));

        assert!(message.contains("0DEAD1:Test.esm"), "{message}");
        assert!(message.contains(SET), "{message}");
    }

    #[test]
    fn dangling_impact_material_and_sound_are_errors_naming_them() {
        let impact = error_text(load_after(|conn| {
            execute(
                conn,
                "UPDATE impact_data_set_entries SET impact = '0DEAD2:Test.esm'",
            );
        }));
        let material = error_text(load_after(|conn| {
            execute(
                conn,
                "UPDATE impact_data_set_entries SET material = '0DEAD3:Test.esm'",
            );
        }));
        let sound = error_text(load_after(|conn| {
            execute(conn, "UPDATE impacts SET player_sound = '0DEAD4:Test.esm'");
        }));

        assert!(impact.contains("0DEAD2:Test.esm"), "{impact}");
        assert!(material.contains("0DEAD3:Test.esm"), "{material}");
        assert!(sound.contains("0DEAD4:Test.esm"), "{sound}");
    }

    #[test]
    fn dangling_child_category_and_output_model_are_errors_naming_them() {
        let child = error_text(load_after(|conn| {
            insert(
                conn,
                "sound_descriptor_children",
                &[
                    ("descriptor", text(COMPOUND)),
                    ("ord", int(2)),
                    ("child", text("0DEAD5:Test.esm")),
                ],
            );
        }));
        let category = error_text(load_after(|conn| {
            execute(
                conn,
                "UPDATE sound_categories SET parent = '0DEAD6:Test.esm' WHERE parent IS NOT NULL",
            );
        }));
        let model = error_text(load_after(|conn| {
            execute(
                conn,
                "UPDATE sound_descriptors SET output_model = '0DEAD7:Test.esm' \
                 WHERE output_model IS NOT NULL",
            );
        }));

        assert!(child.contains("0DEAD5:Test.esm"), "{child}");
        assert!(category.contains("0DEAD6:Test.esm"), "{category}");
        assert!(model.contains("0DEAD7:Test.esm"), "{model}");
    }

    #[test]
    fn unknown_gait_is_an_error() {
        let gait = error_text(load_after(|conn| {
            execute(
                conn,
                "UPDATE footstep_set_entries SET gait = 'crawl' WHERE gait = 'sneak'",
            );
        }));

        assert!(gait.contains("crawl"), "{gait}");
    }

    const CREATURE_SET: &str = "000180:Test.esm";
    const CREATURE_STEP: &str = "000181:Test.esm";

    fn add_creature_set(conn: &Connection) {
        insert(conn, "footstep_sets", &[("form_key", text(CREATURE_SET))]);
        insert(
            conn,
            "footsteps",
            &[
                ("form_key", text(CREATURE_STEP)),
                ("tag", text("FootFront")),
                ("impact_data_set", text(DATA_SET)),
            ],
        );
        insert(
            conn,
            "footstep_set_entries",
            &[
                ("set_key", text(CREATURE_SET)),
                ("gait", text("walk")),
                ("ord", int(0)),
                ("footstep", text(CREATURE_STEP)),
            ],
        );
    }

    fn add_gait(conn: &Connection, gait: &str) {
        insert(
            conn,
            "footstep_set_entries",
            &[
                ("set_key", text(SET)),
                ("gait", text(gait)),
                ("ord", int(0)),
                ("footstep", text(STEP_RIGHT)),
            ],
        );
    }

    #[test]
    fn footsteps_with_creature_tags_outside_the_default_set_are_left_out() {
        let catalog = load_after(add_creature_set).unwrap();

        assert!(!catalog.footsteps.contains_key(CREATURE_STEP));
        assert!(catalog.footstep_sets[CREATURE_SET].gaits.is_empty());
        assert_eq!(catalog.footstep_sets[SET].gaits[&Gait::Run].len(), 2);
    }

    #[test]
    fn default_set_without_a_player_tagged_footstep_in_a_gait_is_an_error() {
        let message = error_text(load_after(|conn| {
            execute(conn, "UPDATE footsteps SET tag = 'PAMoveStart'");
        }));

        assert!(message.contains("Run"), "{message}");
    }

    #[test]
    fn default_set_keeps_player_steps_next_to_power_armor_steps() {
        let catalog = load_after(|conn| {
            execute(
                conn,
                &format!("UPDATE footsteps SET tag = 'PAMoveStart' WHERE form_key = '{STEP_LEFT}'"),
            );
        })
        .unwrap();

        assert_eq!(catalog.footstep_sets[SET].gaits[&Gait::Run], [STEP_RIGHT]);
    }

    #[test]
    fn default_slot_must_exist_and_name_a_set() {
        let missing = error_text(load_after(|conn| {
            execute(conn, "DELETE FROM default_object_slots");
        }));
        let not_a_set = error_text(load_after(|conn| {
            execute(
                conn,
                "UPDATE default_object_slots SET target = '0DEAD8:Test.esm'",
            );
        }));

        assert!(missing.contains("DFTS"), "{missing}");
        assert!(not_a_set.contains("0DEAD8:Test.esm"), "{not_a_set}");
    }

    #[test]
    fn material_parent_cycle_is_an_error() {
        let message = error_text(load_after(|conn| {
            execute(
                conn,
                &format!(
                    "UPDATE material_types SET parent = '{MATERIAL_GRASS}' \
                     WHERE form_key = '{MATERIAL_DIRT}'"
                ),
            );
        }));

        assert!(message.contains("cycle"), "{message}");
    }

    #[test]
    fn null_required_descriptor_value_is_an_error_naming_the_column() {
        let message = error_text(load_after(|conn| {
            execute(
                conn,
                "UPDATE sound_descriptors SET db_variance = NULL WHERE descriptor_type = 0x1EEF540A",
            );
        }));

        assert!(message.contains("db_variance"), "{message}");
    }
}
