//! Which sound layers a footstep event plays. Pure: the catalog, the conditions and the random
//! source are inputs, nothing is logged or remembered.
//!
//! The walk, INFERRED from the data layout and the Creation Kit documentation:
//! default footstep set -> the footsteps of the event's gait -> the first one whose tag is the
//! event's tag -> its impact data set -> the entry of the material, or of the nearest ancestor
//! material that has one -> the impact's player sound -> the layers: a compound sound plays every
//! child whose conditions hold, a standard sound plays itself when its own conditions hold.

use std::collections::{BTreeMap, HashSet};

use crate::assets::db::footsteps::{FootstepCatalog, ImpactDataSet};
use crate::assets::db::sound_catalog::{COMPOUND_DESCRIPTOR, STANDARD_DESCRIPTOR_TYPE};
use crate::audio::conditions::{ConditionContext, ConditionError, evaluate};
use crate::audio::sound_resolve::{
    LayerPlay, SoundError, descriptor_file_keys, lookup_descriptor, one_shot_layer,
};
use crate::footsteps::{FootTag, FootstepEvent, Gait};
use crate::random::RandomSource;

/// Why a step makes no sound; each is data the game leaves open, not an error.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SilentReason {
    NoMaterial,
    NoFootstep { gait: Gait, tag: FootTag },
    NoImpactDataSet { footstep: String },
    UnlistedMaterial(u32),
    NoPlayerSound { impact: String },
    NoLayerPlays { descriptor: String },
}

impl std::fmt::Display for SilentReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoMaterial => write!(f, "the surface has no material"),
            Self::NoFootstep { gait, tag } => {
                write!(
                    f,
                    "the default footstep set has no {tag:?} footstep for {gait:?}"
                )
            }
            Self::NoImpactDataSet { footstep } => {
                write!(f, "footstep {footstep} names no impact data set")
            }
            Self::UnlistedMaterial(hash) => write!(
                f,
                "neither material {hash:08X} nor any of its parents is in the impact data set"
            ),
            Self::NoPlayerSound { impact } => {
                write!(f, "impact {impact} has no player sound")
            }
            Self::NoLayerPlays { descriptor } => {
                write!(
                    f,
                    "no layer of sound {descriptor} can play: its conditions fail or its files are missing"
                )
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    Silent(SilentReason),
    Play(Vec<LayerPlay>),
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ResolveError {
    #[error("the footstep catalog has no {kind} {key}")]
    UnknownRecord { kind: &'static str, key: String },
    #[error("material {0:08X} has a parent cycle")]
    MaterialCycle(u32),
    #[error("sound descriptor {0} contains itself")]
    DescriptorCycle(String),
    #[error(transparent)]
    Condition(#[from] ConditionError),
    #[error(transparent)]
    Sound(#[from] SoundError),
}

/// The layers `event` plays on `material` (the hash of the surface's material, `None` when it
/// has none; a terrain surface is turned into its landscape texture's material by the caller).
/// `missing_files` are file keys known to be absent; a layer never picks them and a layer left
/// with no file plays nothing.
#[allow(
    clippy::implicit_hasher,
    reason = "the missing files are always the default-hasher set the probe returns"
)]
pub fn resolve(
    catalog: &FootstepCatalog,
    ctx: &dyn ConditionContext,
    event: FootstepEvent,
    material: Option<u32>,
    missing_files: &HashSet<String>,
    rng: &mut dyn RandomSource,
) -> Result<Resolution, ResolveError> {
    let Some(material) = material else {
        return Ok(Resolution::Silent(SilentReason::NoMaterial));
    };
    let Some(footstep) = footstep_for(catalog, event)? else {
        return Ok(Resolution::Silent(SilentReason::NoFootstep {
            gait: event.gait,
            tag: event.tag,
        }));
    };
    let Some(data_set) = &footstep.impact_data_set else {
        return Ok(Resolution::Silent(SilentReason::NoImpactDataSet {
            footstep: footstep.form_key.clone(),
        }));
    };
    let data_set = lookup(&catalog.impact_data_sets, "impact data set", data_set)?;
    let Some(impact) = impact_of(catalog, data_set, material)? else {
        return Ok(Resolution::Silent(SilentReason::UnlistedMaterial(material)));
    };
    let impact = lookup(&catalog.impacts, "impact", impact)?;
    let Some(sound) = &impact.player_sound else {
        return Ok(Resolution::Silent(SilentReason::NoPlayerSound {
            impact: impact.form_key.clone(),
        }));
    };
    let mut layers = Vec::new();
    collect_layers(
        catalog,
        ctx,
        sound,
        &mut Vec::new(),
        missing_files,
        rng,
        &mut layers,
    )?;
    Ok(if layers.is_empty() {
        Resolution::Silent(SilentReason::NoLayerPlays {
            descriptor: sound.clone(),
        })
    } else {
        Resolution::Play(layers)
    })
}

fn lookup<'a, T>(
    map: &'a std::collections::HashMap<String, T>,
    kind: &'static str,
    key: &str,
) -> Result<&'a T, ResolveError> {
    map.get(key).ok_or_else(|| ResolveError::UnknownRecord {
        kind,
        key: key.to_string(),
    })
}

/// The first footstep of the event's gait in the default set that carries the event's tag.
fn footstep_for(
    catalog: &FootstepCatalog,
    event: FootstepEvent,
) -> Result<Option<&crate::assets::db::footsteps::Footstep>, ResolveError> {
    let set = lookup(
        &catalog.footstep_sets,
        "footstep set",
        &catalog.default_footstep_set,
    )?;
    let Some(keys) = set.gaits.get(&event.gait) else {
        return Ok(None);
    };
    for key in keys {
        let footstep = lookup(&catalog.footsteps, "footstep", key)?;
        if footstep.tag == event.tag {
            return Ok(Some(footstep));
        }
    }
    Ok(None)
}

/// The impact listed for `material`, else for its nearest ancestor that is listed.
fn impact_of<'a>(
    catalog: &FootstepCatalog,
    data_set: &'a ImpactDataSet,
    material: u32,
) -> Result<Option<&'a String>, ResolveError> {
    let mut current = Some(material);
    for _ in 0..=catalog.material_parents.len() {
        let Some(hash) = current else {
            return Ok(None);
        };
        if let Some(entry) = data_set.entries.iter().find(|entry| entry.material == hash) {
            return Ok(Some(&entry.impact));
        }
        current = catalog.material_parents.get(&hash).copied().flatten();
    }
    Err(ResolveError::MaterialCycle(material))
}

fn collect_layers(
    catalog: &FootstepCatalog,
    ctx: &dyn ConditionContext,
    key: &str,
    path: &mut Vec<String>,
    missing_files: &HashSet<String>,
    rng: &mut dyn RandomSource,
    layers: &mut Vec<LayerPlay>,
) -> Result<(), ResolveError> {
    if path.iter().any(|ancestor| ancestor == key) {
        return Err(ResolveError::DescriptorCycle(key.to_string()));
    }
    let descriptor = lookup_descriptor(&catalog.sounds, key)?;
    if !evaluate(&descriptor.conditions, ctx)? {
        return Ok(());
    }
    match descriptor.descriptor_type {
        COMPOUND_DESCRIPTOR => {
            path.push(key.to_string());
            for child in &descriptor.children {
                collect_layers(catalog, ctx, child, path, missing_files, rng, layers)?;
            }
            path.pop();
            Ok(())
        }
        STANDARD_DESCRIPTOR_TYPE => {
            layers.extend(one_shot_layer(
                &catalog.sounds,
                descriptor,
                missing_files,
                rng,
            )?);
            Ok(())
        }
        descriptor_type => Err(SoundError::UnsupportedDescriptor {
            descriptor: key.to_string(),
            descriptor_type,
        }
        .into()),
    }
}

/// A sound any step of the default set can reach, and its files below the audio folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachableSound {
    pub descriptor: String,
    pub file_keys: Vec<String>,
}

/// Every sound with files that any step of the default set can reach, whatever its conditions,
/// ordered by descriptor key.
pub fn reachable_sounds(catalog: &FootstepCatalog) -> Result<Vec<ReachableSound>, ResolveError> {
    let set = lookup(
        &catalog.footstep_sets,
        "footstep set",
        &catalog.default_footstep_set,
    )?;
    let mut pending: Vec<&String> = Vec::new();
    for footstep in set.gaits.values().flatten() {
        let footstep = lookup(&catalog.footsteps, "footstep", footstep)?;
        let Some(data_set) = &footstep.impact_data_set else {
            continue;
        };
        for entry in &lookup(&catalog.impact_data_sets, "impact data set", data_set)?.entries {
            let impact = lookup(&catalog.impacts, "impact", &entry.impact)?;
            pending.extend(impact.player_sound.iter());
        }
    }
    let mut seen = HashSet::new();
    let mut sounds = BTreeMap::new();
    while let Some(key) = pending.pop() {
        if !seen.insert(key) {
            continue;
        }
        let descriptor = lookup_descriptor(&catalog.sounds, key)?;
        pending.extend(descriptor.children.iter());
        if !descriptor.sound_files.is_empty() {
            sounds.insert(key.clone(), descriptor_file_keys(descriptor)?);
        }
    }
    Ok(sounds
        .into_iter()
        .map(|(descriptor, file_keys)| ReachableSound {
            descriptor,
            file_keys,
        })
        .collect())
}
