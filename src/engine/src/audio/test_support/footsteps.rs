//! A small footstep catalog for tests: a default set with run, sprint and sneak footsteps, three
//! materials with a parent chain, and a compound dirt sound with one layer per condition case.

use std::collections::HashMap;

use crate::assets::db::footsteps::{
    Footstep, FootstepCatalog, FootstepSet, Impact, ImpactDataSet, MaterialImpact,
};
use crate::assets::db::music::{Comparand, CompareOp, Condition, SoundCategory};
use crate::assets::db::sound_catalog::{
    COMPOUND_DESCRIPTOR, OutputChannel, STANDARD_DESCRIPTOR_TYPE, SoundCatalog, SoundDescriptor,
    SoundOutputModel,
};
use crate::audio::conditions::{HAS_SILENT_RUNNING_PERK, IS_FIRST_PERSON};
use crate::audio::test_support::condition::condition;
use crate::footsteps::{FootTag, Gait};

pub const DIRT: u32 = 0xD1;
pub const STONE: u32 = 0x57;
/// A child material of dirt that no impact data set lists.
pub const GRASS: u32 = 0x67;
/// A material without parent that no impact data set lists.
pub const ORPHAN: u32 = 0x0F;
/// A material the catalog does not know.
pub const UNKNOWN: u32 = 0xBAD;

pub const MODEL_ATTENUATION_DB: f32 = 2.0;
pub const CHANNEL_PERCENT: i32 = 50;
pub const FOOTSTEP_CATEGORY_MULTIPLIER: f32 = 0.5;
pub const MASTER_CATEGORY_MULTIPLIER: f32 = 0.8;
pub const STATIC_ATTENUATION_CENTIDB: i32 = 100;

pub const DIRT_FIRST_PERSON_FILES: [&str; 2] =
    ["sound/fx/fst/dirt/1p_a.flac", "sound/fx/fst/dirt/1p_b.flac"];
pub const DIRT_QUIET_FILE: &str = "sound/fx/fst/dirt/quiet.flac";
pub const STONE_FILE: &str = "sound/fx/fst/stone/step.flac";
pub const SPRINT_FILE: &str = "sound/fx/fst/dirt/sprint.flac";
pub const SNEAK_FILE: &str = "sound/fx/fst/dirt/sneak.flac";
/// Files of layers the conditions silence; no test should ever play them.
pub const THIRD_PERSON_FILE: &str = "sound/fx/fst/dirt/3p.flac";
pub const SILENT_RUNNING_FILE: &str = "sound/fx/fst/dirt/perk.flac";

/// Every file the catalog's default set can reach.
pub fn all_files() -> Vec<&'static str> {
    let mut files = DIRT_FIRST_PERSON_FILES.to_vec();
    files.extend([
        DIRT_QUIET_FILE,
        STONE_FILE,
        SPRINT_FILE,
        SNEAK_FILE,
        THIRD_PERSON_FILE,
        SILENT_RUNNING_FILE,
    ]);
    files
}

fn archive_path(file_key: &str) -> String {
    let path = file_key.replace('/', "\\").replace(".flac", ".wav");
    format!("Sound\\{}", path.trim_start_matches("sound\\"))
}

fn equals(function: u16, value: f32) -> Vec<Condition> {
    vec![condition(
        function,
        CompareOp::Equal,
        Comparand::Value(value),
    )]
}

pub fn first_person(value: f32) -> Vec<Condition> {
    equals(IS_FIRST_PERSON, value)
}

pub fn silent_running(value: f32) -> Vec<Condition> {
    equals(HAS_SILENT_RUNNING_PERK, value)
}

pub fn standard(key: &str, files: &[&str], conditions: Vec<Condition>) -> SoundDescriptor {
    SoundDescriptor {
        form_key: key.to_string(),
        editor_id: None,
        descriptor_type: STANDARD_DESCRIPTOR_TYPE,
        base_descriptor: None,
        freq_shift_pct: 0,
        freq_variance_pct: 0,
        priority: 0,
        db_variance: 0,
        static_attenuation_centidb: STATIC_ATTENUATION_CENTIDB,
        looping_flags: 0x0100,
        category: Some("footstep".to_string()),
        output_model: Some("model".to_string()),
        sound_files: files.iter().map(|file| archive_path(file)).collect(),
        children: vec![],
        conditions,
    }
}

pub fn compound(key: &str, children: &[&str]) -> SoundDescriptor {
    SoundDescriptor {
        descriptor_type: COMPOUND_DESCRIPTOR,
        category: None,
        output_model: None,
        children: children.iter().map(ToString::to_string).collect(),
        ..standard(key, &[], vec![])
    }
}

fn footstep(key: &str, tag: FootTag, data_set: &str) -> (String, Footstep) {
    (
        key.to_string(),
        Footstep {
            form_key: key.to_string(),
            editor_id: None,
            tag,
            impact_data_set: Some(data_set.to_string()),
        },
    )
}

fn data_set(key: &str, entries: &[(u32, &str)]) -> (String, ImpactDataSet) {
    (
        key.to_string(),
        ImpactDataSet {
            form_key: key.to_string(),
            editor_id: None,
            entries: entries
                .iter()
                .map(|&(material, impact)| MaterialImpact {
                    material,
                    impact: impact.to_string(),
                })
                .collect(),
        },
    )
}

fn impact(key: &str, sound: &str) -> (String, Impact) {
    (
        key.to_string(),
        Impact {
            form_key: key.to_string(),
            editor_id: None,
            player_sound: Some(sound.to_string()),
        },
    )
}

fn category(key: &str, multiplier: f32, parent: Option<&str>) -> (String, SoundCategory) {
    (
        key.to_string(),
        SoundCategory {
            form_key: key.to_string(),
            editor_id: None,
            parent: parent.map(ToString::to_string),
            static_multiplier: Some(multiplier),
        },
    )
}

/// Run and sneak have a right and a left foot, sprint only a right one; the walk gait is absent.
/// Dirt and stone are listed for run, dirt alone for sprint and sneak, and grass is a child of
/// dirt. Dirt plays the compound `dirt_sound`: a first-person layer with two files, a quiet layer
/// that needs no Silent Running perk, and two layers the conditions silence.
pub fn catalog() -> FootstepCatalog {
    let descriptors = [
        standard("dirt_1p", &DIRT_FIRST_PERSON_FILES, first_person(1.0)),
        standard("dirt_3p", &[THIRD_PERSON_FILE], first_person(0.0)),
        standard("dirt_quiet", &[DIRT_QUIET_FILE], silent_running(0.0)),
        standard("dirt_perk", &[SILENT_RUNNING_FILE], silent_running(1.0)),
        compound(
            "dirt_sound",
            &["dirt_1p", "dirt_3p", "dirt_quiet", "dirt_perk"],
        ),
        standard("stone_sound", &[STONE_FILE], vec![]),
        standard("sprint_sound", &[SPRINT_FILE], vec![]),
        standard("sneak_sound", &[SNEAK_FILE], vec![]),
    ];
    FootstepCatalog {
        default_footstep_set: "set".to_string(),
        footstep_sets: HashMap::from([(
            "set".to_string(),
            FootstepSet {
                form_key: "set".to_string(),
                editor_id: None,
                gaits: HashMap::from([
                    (Gait::Run, vec!["run_r".to_string(), "run_l".to_string()]),
                    (Gait::Sprint, vec!["sprint_r".to_string()]),
                    (
                        Gait::Sneak,
                        vec!["sneak_r".to_string(), "sneak_l".to_string()],
                    ),
                ]),
            },
        )]),
        footsteps: HashMap::from([
            footstep("run_r", FootTag::FootRight, "run_data"),
            footstep("run_l", FootTag::FootLeft, "run_data"),
            footstep("sprint_r", FootTag::FootSprintRight, "sprint_data"),
            footstep("sneak_r", FootTag::FootRight, "sneak_data"),
            footstep("sneak_l", FootTag::FootLeft, "sneak_data"),
        ]),
        impact_data_sets: HashMap::from([
            data_set(
                "run_data",
                &[(DIRT, "dirt_impact"), (STONE, "stone_impact")],
            ),
            data_set("sprint_data", &[(DIRT, "sprint_impact")]),
            data_set("sneak_data", &[(DIRT, "sneak_impact")]),
        ]),
        material_hashes: HashMap::new(),
        material_names: HashMap::new(),
        material_parents: HashMap::from([
            (DIRT, None),
            (STONE, None),
            (GRASS, Some(DIRT)),
            (ORPHAN, None),
        ]),
        impacts: HashMap::from([
            impact("dirt_impact", "dirt_sound"),
            impact("stone_impact", "stone_sound"),
            impact("sprint_impact", "sprint_sound"),
            impact("sneak_impact", "sneak_sound"),
        ]),
        sounds: SoundCatalog {
            descriptors: descriptors
                .into_iter()
                .map(|descriptor| (descriptor.form_key.clone(), descriptor))
                .collect(),
            output_models: HashMap::from([(
                "model".to_string(),
                SoundOutputModel {
                    form_key: "model".to_string(),
                    editor_id: None,
                    output_type: 1,
                    static_attenuation_db: MODEL_ATTENUATION_DB,
                    channels: ["L", "R"]
                        .into_iter()
                        .map(|speaker| OutputChannel {
                            input_channel: 0,
                            speaker: speaker.to_string(),
                            percent: CHANNEL_PERCENT,
                        })
                        .collect(),
                },
            )]),
            categories: HashMap::from([
                category("footstep", FOOTSTEP_CATEGORY_MULTIPLIER, Some("master")),
                category("master", MASTER_CATEGORY_MULTIPLIER, None),
            ]),
        },
        landscape_materials: HashMap::new(),
    }
}

/// The level every layer of `catalog()` has when its variance draws sit at their middle.
pub fn expected_gain_db() -> f32 {
    -(STATIC_ATTENUATION_CENTIDB as f32) / 100.0 - MODEL_ATTENUATION_DB
        + 20.0 * (CHANNEL_PERCENT as f32 / 100.0).log10()
        + 20.0 * FOOTSTEP_CATEGORY_MULTIPLIER.log10()
        + 20.0 * MASTER_CATEGORY_MULTIPLIER.log10()
}
