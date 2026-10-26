//! Turns a sound descriptor of the catalog into something to play: the file below the audio folder
//! and its gain, for a one-shot with its pitch as well. Pure: the catalog, the files known to be
//! absent and the random source are inputs, nothing is logged or remembered.
//!
//! `resolve_one_shot` and `resolve_loop` take a single standard sound and leave its conditions to
//! the caller. `resolve_layers` walks a compound descriptor: it plays every child whose conditions
//! hold, and a standard sound plays itself when its own conditions hold.

#![allow(
    clippy::implicit_hasher,
    reason = "the maps are the catalog's and the file probe's default-hasher collections"
)]

use std::collections::{BTreeMap, HashMap, HashSet};

use super::conditions::{ConditionContext, ConditionError, evaluate};
use super::sound_mix::{LayerMix, MixError, is_looping};
use crate::assets::db::sound_catalog::{
    COMPOUND_DESCRIPTOR, STANDARD_DESCRIPTOR_TYPE, SoundCatalog, SoundDescriptor, SoundOutputModel,
};
use crate::assets::layout::{AudioKeyError, audio_key};
use crate::random::RandomSource;

/// One sound to start: the file below the audio folder and how it plays.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerPlay {
    pub file_key: String,
    pub mix: LayerMix,
}

/// A looping sound to start: the file below the audio folder and its constant gain.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopPlay {
    pub file_key: String,
    pub gain_db: f32,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq)]
pub enum SoundError {
    #[error("the sound catalog has no {kind} {key}")]
    UnknownRecord { kind: &'static str, key: String },
    #[error("sound descriptor {descriptor} has the unsupported type {descriptor_type:08X}")]
    UnsupportedDescriptor {
        descriptor: String,
        descriptor_type: u32,
    },
    #[error("sound {0} loops; it was asked to play once")]
    Looping(String),
    #[error("sound {0} lists no sound file")]
    NoSoundFiles(String),
    #[error("sound {0} names no output model")]
    NoOutputModel(String),
    #[error("sound file path of {descriptor} is unusable: {source}")]
    FilePath {
        descriptor: String,
        source: AudioKeyError,
    },
    #[error("sound {descriptor}: {source}")]
    Mix {
        descriptor: String,
        source: MixError,
    },
}

fn lookup<'a, T>(
    map: &'a HashMap<String, T>,
    kind: &'static str,
    key: &str,
) -> Result<&'a T, SoundError> {
    map.get(key).ok_or_else(|| SoundError::UnknownRecord {
        kind,
        key: key.to_string(),
    })
}

pub fn lookup_descriptor<'a>(
    sounds: &'a SoundCatalog,
    key: &str,
) -> Result<&'a SoundDescriptor, SoundError> {
    lookup(&sounds.descriptors, "sound descriptor", key)
}

/// The file keys of the descriptor's sound files, in record order.
pub fn descriptor_file_keys(descriptor: &SoundDescriptor) -> Result<Vec<String>, SoundError> {
    descriptor
        .sound_files
        .iter()
        .map(|file| {
            audio_key(file).map_err(|source| SoundError::FilePath {
                descriptor: descriptor.form_key.clone(),
                source,
            })
        })
        .collect()
}

/// The sound the standard descriptor `key` plays once. `None` when every file of it is missing.
pub fn resolve_one_shot(
    sounds: &SoundCatalog,
    key: &str,
    missing_files: &HashSet<String>,
    rng: &mut dyn RandomSource,
) -> Result<Option<LayerPlay>, SoundError> {
    let descriptor = standard_descriptor(sounds, key)?;
    one_shot_layer(sounds, descriptor, missing_files, rng)
}

/// The sound the standard descriptor `key` plays in a loop, at its constant gain. `None` when every
/// file of it is missing. Whether the descriptor's looping flags are set is not checked: the caller
/// decides to loop it.
pub fn resolve_loop(
    sounds: &SoundCatalog,
    key: &str,
    missing_files: &HashSet<String>,
    rng: &mut dyn RandomSource,
) -> Result<Option<LoopPlay>, SoundError> {
    let descriptor = standard_descriptor(sounds, key)?;
    let model = output_model(sounds, descriptor)?;
    let Some(file_key) = pick_file(descriptor, missing_files, rng)? else {
        return Ok(None);
    };
    let gain_db = LayerMix::steady_gain_db(descriptor, model, &sounds.categories)
        .map_err(|source| mix_error(descriptor, source))?;
    Ok(Some(LoopPlay { file_key, gain_db }))
}

/// Draw order: the file, then the pitch variance, then the volume variance. `None` when every
/// file of the descriptor is missing.
pub fn one_shot_layer(
    sounds: &SoundCatalog,
    descriptor: &SoundDescriptor,
    missing_files: &HashSet<String>,
    rng: &mut dyn RandomSource,
) -> Result<Option<LayerPlay>, SoundError> {
    if is_looping(descriptor) {
        return Err(SoundError::Looping(descriptor.form_key.clone()));
    }
    let model = output_model(sounds, descriptor)?;
    let Some(file_key) = pick_file(descriptor, missing_files, rng)? else {
        return Ok(None);
    };
    let mix = LayerMix::draw(descriptor, model, &sounds.categories, rng)
        .map_err(|source| mix_error(descriptor, source))?;
    Ok(Some(LayerPlay { file_key, mix }))
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum LayersError {
    #[error("sound descriptor {0} contains itself")]
    Cycle(String),
    #[error(transparent)]
    Condition(#[from] ConditionError),
    #[error(transparent)]
    Sound(#[from] SoundError),
}

/// The layers the descriptor `key` plays once. Its compound children are walked depth first in
/// record order, and a descriptor whose conditions fail plays nothing, children included. Every
/// layer is a standard sound and picks its own file and variance. A layer whose files are all
/// missing plays nothing, so the result is empty when no layer plays.
pub fn resolve_layers(
    sounds: &SoundCatalog,
    ctx: &dyn ConditionContext,
    key: &str,
    missing_files: &HashSet<String>,
    rng: &mut dyn RandomSource,
) -> Result<Vec<LayerPlay>, LayersError> {
    let mut layers = Vec::new();
    collect_layers(
        sounds,
        ctx,
        key,
        &mut Vec::new(),
        missing_files,
        rng,
        &mut layers,
    )?;
    Ok(layers)
}

fn collect_layers(
    sounds: &SoundCatalog,
    ctx: &dyn ConditionContext,
    key: &str,
    path: &mut Vec<String>,
    missing_files: &HashSet<String>,
    rng: &mut dyn RandomSource,
    layers: &mut Vec<LayerPlay>,
) -> Result<(), LayersError> {
    if path.iter().any(|ancestor| ancestor == key) {
        return Err(LayersError::Cycle(key.to_string()));
    }
    let descriptor = lookup_descriptor(sounds, key)?;
    if !evaluate(&descriptor.conditions, ctx)? {
        return Ok(());
    }
    match descriptor.descriptor_type {
        COMPOUND_DESCRIPTOR => {
            path.push(key.to_string());
            for child in &descriptor.children {
                collect_layers(sounds, ctx, child, path, missing_files, rng, layers)?;
            }
            path.pop();
            Ok(())
        }
        STANDARD_DESCRIPTOR_TYPE => {
            layers.extend(one_shot_layer(sounds, descriptor, missing_files, rng)?);
            Ok(())
        }
        descriptor_type => Err(SoundError::UnsupportedDescriptor {
            descriptor: key.to_string(),
            descriptor_type,
        }
        .into()),
    }
}

/// A sound with files that some roots can reach, and those files below the audio folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachableSound {
    pub descriptor: String,
    pub file_keys: Vec<String>,
}

/// Every sound with files that the descriptors `roots` reach through compound children, whatever
/// their conditions, ordered by descriptor key.
pub fn reachable_sounds<'a>(
    sounds: &'a SoundCatalog,
    roots: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<ReachableSound>, SoundError> {
    let mut pending: Vec<&str> = roots.into_iter().collect();
    let mut seen = HashSet::new();
    let mut reachable = BTreeMap::new();
    while let Some(key) = pending.pop() {
        if !seen.insert(key) {
            continue;
        }
        let descriptor = lookup_descriptor(sounds, key)?;
        pending.extend(descriptor.children.iter().map(String::as_str));
        if !descriptor.sound_files.is_empty() {
            reachable.insert(key.to_string(), descriptor_file_keys(descriptor)?);
        }
    }
    Ok(reachable
        .into_iter()
        .map(|(descriptor, file_keys)| ReachableSound {
            descriptor,
            file_keys,
        })
        .collect())
}

fn standard_descriptor<'a>(
    sounds: &'a SoundCatalog,
    key: &str,
) -> Result<&'a SoundDescriptor, SoundError> {
    let descriptor = lookup_descriptor(sounds, key)?;
    if descriptor.descriptor_type == STANDARD_DESCRIPTOR_TYPE {
        Ok(descriptor)
    } else {
        Err(SoundError::UnsupportedDescriptor {
            descriptor: key.to_string(),
            descriptor_type: descriptor.descriptor_type,
        })
    }
}

fn output_model<'a>(
    sounds: &'a SoundCatalog,
    descriptor: &SoundDescriptor,
) -> Result<&'a SoundOutputModel, SoundError> {
    let name = &descriptor.form_key;
    if descriptor.sound_files.is_empty() {
        return Err(SoundError::NoSoundFiles(name.clone()));
    }
    let model = descriptor
        .output_model
        .as_ref()
        .ok_or_else(|| SoundError::NoOutputModel(name.clone()))?;
    lookup(&sounds.output_models, "sound output model", model)
}

fn pick_file(
    descriptor: &SoundDescriptor,
    missing_files: &HashSet<String>,
    rng: &mut dyn RandomSource,
) -> Result<Option<String>, SoundError> {
    let mut playable = descriptor_file_keys(descriptor)?;
    playable.retain(|file_key| !missing_files.contains(file_key));
    if playable.is_empty() {
        return Ok(None);
    }
    Ok(Some(playable.swap_remove(rng.below(playable.len()))))
}

fn mix_error(descriptor: &SoundDescriptor, source: MixError) -> SoundError {
    SoundError::Mix {
        descriptor: descriptor.form_key.clone(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::db::music::{Comparand, CompareOp};
    use crate::audio::conditions::FreshWorldContext;
    use crate::audio::test_support::SEED;
    use crate::audio::test_support::condition::condition;
    use crate::audio::test_support::footsteps::{
        DIRT_FIRST_PERSON_FILES, DIRT_QUIET_FILE, STONE_FILE, catalog, compound, expected_gain_db,
        silent_running,
    };
    use crate::random::SplitMix64;

    const GAIN_TOLERANCE: f32 = 1e-4;

    fn sounds() -> SoundCatalog {
        catalog().sounds
    }

    fn modify(sounds: &mut SoundCatalog, key: &str, change: impl FnOnce(&mut SoundDescriptor)) {
        change(sounds.descriptors.get_mut(key).unwrap());
    }

    fn one_shot(sounds: &SoundCatalog, key: &str) -> Result<Option<LayerPlay>, SoundError> {
        resolve_one_shot(sounds, key, &HashSet::new(), &mut SplitMix64::new(SEED))
    }

    fn looped(
        sounds: &SoundCatalog,
        key: &str,
        missing: &HashSet<String>,
    ) -> Result<Option<LoopPlay>, SoundError> {
        resolve_loop(sounds, key, missing, &mut SplitMix64::new(SEED))
    }

    #[test]
    fn a_one_shot_plays_a_file_of_the_descriptor_at_the_mixed_level() {
        let play = one_shot(&sounds(), "stone_sound").unwrap().unwrap();

        assert_eq!(play.file_key, STONE_FILE);
        assert!((play.mix.gain_db - expected_gain_db()).abs() < GAIN_TOLERANCE);
    }

    #[test]
    fn a_one_shot_never_picks_a_missing_file_and_is_none_when_all_are_missing() {
        let sounds = sounds();
        let first = DIRT_FIRST_PERSON_FILES[0].to_string();
        let second = DIRT_FIRST_PERSON_FILES[1].to_string();

        for seed in 0..20 {
            let play = resolve_one_shot(
                &sounds,
                "dirt_1p",
                &HashSet::from([first.clone()]),
                &mut SplitMix64::new(seed),
            )
            .unwrap()
            .unwrap();
            assert_eq!(play.file_key, second);
        }
        let none = resolve_one_shot(
            &sounds,
            "dirt_1p",
            &HashSet::from([first, second]),
            &mut SplitMix64::new(SEED),
        );
        assert_eq!(none, Ok(None));
    }

    #[test]
    fn a_one_shot_rejects_a_looping_descriptor() {
        let mut sounds = sounds();
        modify(&mut sounds, "stone_sound", |d| d.looping_flags = 0x0800);

        assert_eq!(
            one_shot(&sounds, "stone_sound"),
            Err(SoundError::Looping("stone_sound".to_string()))
        );
    }

    #[test]
    fn a_loop_plays_a_file_at_the_constant_gain_whatever_the_variance() {
        let mut sounds = sounds();
        modify(&mut sounds, "stone_sound", |d| {
            d.looping_flags = 0x0800;
            d.db_variance = 6;
            d.freq_variance_pct = 10;
        });

        let plays: Vec<LoopPlay> = (0..20)
            .map(|seed| {
                resolve_loop(
                    &sounds,
                    "stone_sound",
                    &HashSet::new(),
                    &mut SplitMix64::new(seed),
                )
                .unwrap()
                .unwrap()
            })
            .collect();

        assert!(plays.iter().all(|play| play.file_key == STONE_FILE));
        assert!(
            plays
                .iter()
                .all(|play| (play.gain_db - expected_gain_db()).abs() < GAIN_TOLERANCE)
        );
    }

    #[test]
    fn a_loop_skips_missing_files() {
        let missing = HashSet::from([STONE_FILE.to_string()]);

        assert_eq!(looped(&sounds(), "stone_sound", &missing), Ok(None));
    }

    #[test]
    fn a_loop_does_not_require_the_looping_flags() {
        assert!(
            looped(&sounds(), "stone_sound", &HashSet::new())
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn a_compound_descriptor_is_unsupported_for_both() {
        let sounds = sounds();
        let expected = SoundError::UnsupportedDescriptor {
            descriptor: "dirt_sound".to_string(),
            descriptor_type: COMPOUND_DESCRIPTOR,
        };

        assert_eq!(one_shot(&sounds, "dirt_sound"), Err(expected.clone()));
        assert_eq!(
            looped(&sounds, "dirt_sound", &HashSet::new()),
            Err(expected)
        );
    }

    #[test]
    fn an_unknown_descriptor_is_an_error_naming_it() {
        let expected = SoundError::UnknownRecord {
            kind: "sound descriptor",
            key: "nope".to_string(),
        };

        assert_eq!(one_shot(&sounds(), "nope"), Err(expected.clone()));
        assert_eq!(looped(&sounds(), "nope", &HashSet::new()), Err(expected));
    }

    #[test]
    fn a_loop_without_an_output_model_or_files_is_an_error() {
        let mut no_model = sounds();
        modify(&mut no_model, "stone_sound", |d| d.output_model = None);
        let mut no_files = sounds();
        modify(&mut no_files, "stone_sound", |d| d.sound_files.clear());

        assert_eq!(
            looped(&no_model, "stone_sound", &HashSet::new()),
            Err(SoundError::NoOutputModel("stone_sound".to_string()))
        );
        assert_eq!(
            looped(&no_files, "stone_sound", &HashSet::new()),
            Err(SoundError::NoSoundFiles("stone_sound".to_string()))
        );
    }

    #[test]
    fn a_loop_with_an_unusable_category_chain_is_an_error_naming_the_sound() {
        let mut sounds = sounds();
        sounds.categories.clear();

        let error = looped(&sounds, "stone_sound", &HashSet::new()).unwrap_err();

        assert!(
            matches!(&error, SoundError::Mix { descriptor, source: MixError::UnknownCategory(_) } if descriptor == "stone_sound"),
            "{error:?}"
        );
    }

    fn world() -> FreshWorldContext {
        FreshWorldContext::new(0.0, [])
    }

    fn layers(
        sounds: &SoundCatalog,
        key: &str,
        missing: &HashSet<String>,
    ) -> Result<Vec<LayerPlay>, LayersError> {
        resolve_layers(sounds, &world(), key, missing, &mut SplitMix64::new(SEED))
    }

    fn file_keys(layers: &[LayerPlay]) -> Vec<&str> {
        layers.iter().map(|layer| layer.file_key.as_str()).collect()
    }

    #[test]
    fn a_compound_plays_every_child_whose_conditions_hold_in_record_order() {
        let played = layers(&sounds(), "dirt_sound", &HashSet::new()).unwrap();

        let files = file_keys(&played);
        assert_eq!(files.len(), 2, "{files:?}");
        assert!(DIRT_FIRST_PERSON_FILES.contains(&files[0]), "{files:?}");
        assert_eq!(files[1], DIRT_QUIET_FILE);
    }

    #[test]
    fn a_standard_descriptor_plays_itself() {
        let played = layers(&sounds(), "stone_sound", &HashSet::new()).unwrap();

        assert_eq!(file_keys(&played), [STONE_FILE]);
    }

    #[test]
    fn a_descriptor_whose_conditions_fail_plays_nothing_and_neither_do_its_children() {
        let mut sounds = sounds();
        modify(&mut sounds, "dirt_sound", |d| {
            d.conditions = silent_running(1.0);
        });

        assert_eq!(layers(&sounds, "dirt_sound", &HashSet::new()), Ok(vec![]));
    }

    #[test]
    fn a_compound_inside_a_compound_plays_the_layers_of_both() {
        let mut sounds = sounds();
        sounds.descriptors.insert(
            "outer".to_string(),
            compound("outer", &["dirt_sound", "stone_sound"]),
        );

        let played = layers(&sounds, "outer", &HashSet::new()).unwrap();

        assert_eq!(played.len(), 3);
        assert_eq!(played[2].file_key, STONE_FILE);
    }

    #[test]
    fn a_layer_with_every_file_missing_plays_nothing_while_its_siblings_still_play() {
        let missing = HashSet::from([DIRT_QUIET_FILE.to_string()]);

        let played = layers(&sounds(), "dirt_sound", &missing).unwrap();

        assert_eq!(played.len(), 1);
        assert!(DIRT_FIRST_PERSON_FILES.contains(&played[0].file_key.as_str()));
    }

    #[test]
    fn a_compound_that_contains_itself_is_a_cycle_naming_it() {
        let mut sounds = sounds();
        modify(&mut sounds, "dirt_sound", |d| {
            d.children.push("dirt_sound".to_string());
        });

        assert_eq!(
            layers(&sounds, "dirt_sound", &HashSet::new()),
            Err(LayersError::Cycle("dirt_sound".to_string()))
        );
    }

    #[test]
    fn a_condition_function_the_evaluator_lacks_is_an_error() {
        let mut sounds = sounds();
        modify(&mut sounds, "stone_sound", |d| {
            d.conditions = vec![condition(47, CompareOp::Equal, Comparand::Value(1.0))];
        });

        assert_eq!(
            layers(&sounds, "stone_sound", &HashSet::new()),
            Err(LayersError::Condition(ConditionError::UnsupportedFunction(
                47
            )))
        );
    }

    #[test]
    fn a_child_the_catalog_lacks_is_an_error_naming_it() {
        let mut sounds = sounds();
        modify(&mut sounds, "dirt_sound", |d| {
            d.children.push("gone".to_string());
        });

        assert_eq!(
            layers(&sounds, "dirt_sound", &HashSet::new()),
            Err(LayersError::Sound(SoundError::UnknownRecord {
                kind: "sound descriptor",
                key: "gone".to_string()
            }))
        );
    }

    fn reachable(sounds: &SoundCatalog, roots: &[&str]) -> Result<Vec<ReachableSound>, SoundError> {
        reachable_sounds(sounds, roots.iter().copied())
    }

    #[test]
    fn the_reachable_sounds_are_the_ones_with_files_below_the_roots_whatever_their_conditions() {
        let reachable = reachable(&sounds(), &["dirt_sound"]).unwrap();

        let listed: Vec<(&str, usize)> = reachable
            .iter()
            .map(|sound| (sound.descriptor.as_str(), sound.file_keys.len()))
            .collect();
        assert_eq!(
            listed,
            [
                ("dirt_1p", 2),
                ("dirt_3p", 1),
                ("dirt_perk", 1),
                ("dirt_quiet", 1)
            ],
            "ordered by key, without the compound that has no file of its own"
        );
    }

    #[test]
    fn a_sound_reached_by_two_roots_is_listed_once() {
        let reachable = reachable(&sounds(), &["stone_sound", "stone_sound"]).unwrap();

        assert_eq!(reachable.len(), 1);
        assert_eq!(reachable[0].file_keys, [STONE_FILE]);
    }

    #[test]
    fn an_unknown_root_is_an_error_naming_it_when_collecting_reachable_sounds() {
        assert_eq!(
            reachable(&sounds(), &["nope"]),
            Err(SoundError::UnknownRecord {
                kind: "sound descriptor",
                key: "nope".to_string()
            })
        );
    }
}
