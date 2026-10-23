//! Footstep sounds: what a step event plays, decided from the installed footstep catalog.
//!
//! `Footsteps` is the stateful shell around the pure `resolve`: it holds the catalog, the
//! condition context and the random source, checks the converted files at startup and logs a step
//! that finds no sound once per cause. A source file the game never shipped is absent from the
//! install; startup logs it once and leaves it out of every pick, and a sound left with no file
//! is silent. The caller passes the surface's material as a hash: a
//! `SurfaceMaterial::Hash` gives `Some(hash)`, `SurfaceMaterial::None` gives `None`, and a
//! `SurfaceMaterial::Terrain` is first turned into its landscape texture's material hash, which
//! is `None` when that texture names no material. The event's own `surface` is not read.

pub mod mix;
pub mod resolve;
#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use kira::sound::FromFileError;
use kira::sound::streaming::StreamingSoundData;

pub use mix::{LayerMix, MixError};
pub use resolve::{LayerPlay, Resolution, ResolveError, SilentReason, resolve};

use super::conditions::FreshWorldContext;
use super::file_probe::{ProbeError, probe_files};
use super::rng::SplitMix64;
use crate::assets::db::footsteps::FootstepCatalog;
use crate::footsteps::FootstepEvent;

/// The tracing target of everything the footsteps decide.
pub const LOG_TARGET: &str = "openfo4::audio::footsteps";

#[derive(Debug, thiserror::Error)]
pub enum FootstepError {
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    #[error("the audio folder {0} is missing; run the installer")]
    AudioDirectoryMissing(PathBuf),
    #[error("cannot check {path}: {source}")]
    FileCheck {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "footstep sound {path} cannot be opened; the installer converts it from the game, \
         so run the installer again: {source}"
    )]
    UnreadableFile {
        path: PathBuf,
        source: FromFileError,
    },
}

impl From<ProbeError> for FootstepError {
    fn from(error: ProbeError) -> Self {
        match error {
            ProbeError::DirectoryMissing(path) => Self::AudioDirectoryMissing(path),
            ProbeError::Check { path, source } => Self::FileCheck { path, source },
            ProbeError::Open { path, source } => Self::UnreadableFile { path, source },
        }
    }
}

pub struct Footsteps {
    catalog: FootstepCatalog,
    context: FreshWorldContext,
    rng: SplitMix64,
    missing_files: HashSet<String>,
    reported: HashSet<SilentReason>,
}

impl Footsteps {
    /// `audio_dir` is where the converted sound files live. Every file a step of the default
    /// footstep set can reach, whatever its conditions, must be present with a readable header,
    /// or absent: an absent file is logged once and skipped.
    pub fn new(
        catalog: FootstepCatalog,
        audio_dir: &Path,
        seed: u64,
    ) -> Result<Self, FootstepError> {
        let sounds = resolve::reachable_sounds(&catalog)?;
        let missing_files = probe_files(
            "footstep sound",
            sounds
                .iter()
                .flat_map(|sound| sound.file_keys.iter().map(String::as_str)),
            audio_dir,
            |path| StreamingSoundData::from_file(path).map(|_| ()),
        )?;
        for sound in &sounds {
            if sound
                .file_keys
                .iter()
                .all(|file_key| missing_files.contains(file_key))
            {
                tracing::warn!(
                    target: LOG_TARGET,
                    "footstep sound {} has no file left; its steps are silent",
                    sound.descriptor
                );
            }
        }
        Ok(Self {
            catalog,
            context: FreshWorldContext::new(0.0, []),
            rng: SplitMix64::new(seed),
            missing_files,
            reported: HashSet::new(),
        })
    }

    /// The file keys known to be absent from the install.
    pub fn missing_files(&self) -> &HashSet<String> {
        &self.missing_files
    }

    /// The sounds one step starts; empty when the step finds none, which is logged once per cause
    /// at warn level and on every step at debug level.
    pub fn step(
        &mut self,
        event: FootstepEvent,
        material: Option<u32>,
    ) -> Result<Vec<LayerPlay>, FootstepError> {
        let resolution = resolve(
            &self.catalog,
            &self.context,
            event,
            material,
            &self.missing_files,
            &mut self.rng,
        )?;
        let surface = self.surface_label(material);
        match resolution {
            Resolution::Play(layers) => {
                tracing::debug!(
                    target: LOG_TARGET,
                    "{:?} {:?} on {surface} plays {}",
                    event.gait,
                    event.tag,
                    layers
                        .iter()
                        .map(|layer| layer.file_key.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                Ok(layers)
            }
            Resolution::Silent(reason) => {
                let cause = format!("{reason} (surface {surface})");
                tracing::debug!(
                    target: LOG_TARGET,
                    "{:?} {:?} is silent: {cause}",
                    event.gait,
                    event.tag
                );
                if self.reported.insert(reason) {
                    tracing::warn!(target: LOG_TARGET, "footstep is silent: {cause}");
                }
                Ok(Vec::new())
            }
        }
    }

    fn surface_label(&self, material: Option<u32>) -> String {
        material.map_or_else(
            || "without material".to_string(),
            |hash| self.catalog.material_label(hash),
        )
    }
}
