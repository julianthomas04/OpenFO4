//! Item pick-up and put-down sounds: the item's own sound descriptor, else the default of its record
//! type's slot, each side on its own.
//!
//! `ItemSounds` is the stateful shell around the pure `descriptor_for`: it checks the converted
//! files at startup, rehearses every descriptor once so unusable data fails there and never
//! mid-game, and warns once about an item the catalog does not know. A source file the game never
//! shipped is absent from the install; startup logs it once and leaves it out, and a sound left
//! with no file is silent.

pub mod resolve;
#[cfg(test)]
mod tests;

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

pub use resolve::descriptor_for;

use super::file_probe::{ProbeError, probe_sounds};
use super::sound_mix::SourcePlacement;
pub use super::sound_resolve::LayerPlay;
use super::sound_resolve::{SoundError, reachable_sounds, resolve_one_shot_placed};
use crate::assets::db::item_sounds::ItemSoundCatalog;
use crate::random::SplitMix64;

/// An item is picked up or put down by the player: the sound has no direction and no distance.
const PLACEMENT: SourcePlacement = SourcePlacement::AtListener;

/// The tracing target of everything the item sounds decide.
pub const LOG_TARGET: &str = "openfo4::audio::item_sounds";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemSoundKind {
    PickUp,
    PutDown,
}

/// A compound, looping or output-model-less descriptor surfaces as `Resolve(SoundError::..)`
/// (`UnsupportedDescriptor`, `Looping`, `NoOutputModel`); looping is `sound_mix::is_looping`
/// (`looping_flags` 1 and 2 are not looping).
#[derive(Debug, thiserror::Error)]
pub enum ItemSoundError {
    #[error(transparent)]
    Resolve(#[from] SoundError),
    #[error(transparent)]
    Probe(#[from] ProbeError),
}

pub struct ItemSounds {
    catalog: ItemSoundCatalog,
    rng: SplitMix64,
    missing_files: HashSet<String>,
    reported: HashSet<String>,
}

impl ItemSounds {
    /// Probes every file any entry or default can reach and rehearses every descriptor once through
    /// `resolve_one_shot_placed`, so unusable data fails here and never mid-game. An absent file is logged
    /// once and skipped.
    pub fn new(
        catalog: ItemSoundCatalog,
        audio_dir: &Path,
        seed: u64,
    ) -> Result<Self, ItemSoundError> {
        let roots = Self::roots(&catalog);
        let sounds = reachable_sounds(&catalog.sounds, roots.iter().copied())?;
        let missing_files = probe_sounds("item sound", &sounds, audio_dir)?;
        let mut rng = SplitMix64::new(seed);
        for root in &roots {
            resolve_one_shot_placed(&catalog.sounds, root, &missing_files, PLACEMENT, &mut rng)?;
        }
        Ok(Self {
            catalog,
            rng,
            missing_files,
            reported: HashSet::new(),
        })
    }

    /// The sound `kind` plays for `item`: its own descriptor, else its slot's default. `None` when
    /// the item is unknown to the catalog (warned once per item) or when every file of the
    /// descriptor is absent.
    pub fn sound(
        &mut self,
        item: &str,
        kind: ItemSoundKind,
    ) -> Result<Option<LayerPlay>, ItemSoundError> {
        let Some(entry) = self.catalog.items.get(item) else {
            if self.reported.insert(item.to_string()) {
                tracing::warn!(target: LOG_TARGET, "item {item} has no item sounds");
            }
            return Ok(None);
        };
        let key =
            descriptor_for(&self.catalog, item, kind).ok_or_else(|| SoundError::UnknownRecord {
                kind: "default sound slot",
                key: match kind {
                    ItemSoundKind::PickUp => entry.slot.pickup_slot(),
                    ItemSoundKind::PutDown => entry.slot.put_down_slot(),
                }
                .to_string(),
            })?;
        Ok(resolve_one_shot_placed(
            &self.catalog.sounds,
            key,
            &self.missing_files,
            PLACEMENT,
            &mut self.rng,
        )?)
    }

    fn roots(catalog: &ItemSoundCatalog) -> BTreeSet<&str> {
        catalog
            .items
            .values()
            .flat_map(|entry| [&entry.pickup, &entry.put_down])
            .flatten()
            .chain(catalog.defaults.values())
            .map(String::as_str)
            .collect()
    }
}
