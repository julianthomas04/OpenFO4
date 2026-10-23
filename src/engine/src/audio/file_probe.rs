//! Startup check of the converted sound files a feature can play: absent files are skipped and
//! logged once, every other problem is an error.

use std::collections::{BTreeSet, HashSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use kira::sound::FromFileError;

#[derive(Debug, thiserror::Error)]
pub(super) enum ProbeError {
    #[error("the audio folder {0} does not exist")]
    DirectoryMissing(PathBuf),
    #[error("cannot check {path}: {source}")]
    Check {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path} cannot be opened: {source}")]
    Open {
        path: PathBuf,
        source: FromFileError,
    },
}

/// Checks the audio folder and every file key under it, and returns the keys of the files that
/// are absent; each absent file is logged once as `<label> <path> is missing and will not play`.
/// A missing audio folder, a file that cannot be checked, and a present file that `open` rejects
/// are errors.
pub(super) fn probe_files<'a>(
    label: &str,
    file_keys: impl IntoIterator<Item = &'a str>,
    audio_dir: &Path,
    open: impl Fn(&Path) -> Result<(), FromFileError>,
) -> Result<HashSet<String>, ProbeError> {
    require_directory(audio_dir)?;
    let mut missing = HashSet::new();
    for file_key in file_keys.into_iter().collect::<BTreeSet<_>>() {
        let path = audio_dir.join(file_key);
        let present = path.try_exists().map_err(|source| ProbeError::Check {
            path: path.clone(),
            source,
        })?;
        if !present {
            tracing::error!("{label} {} is missing and will not play", path.display());
            missing.insert(file_key.to_string());
            continue;
        }
        open(&path).map_err(|source| ProbeError::Open { path, source })?;
    }
    Ok(missing)
}

fn require_directory(audio_dir: &Path) -> Result<(), ProbeError> {
    match std::fs::metadata(audio_dir) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(ProbeError::DirectoryMissing(audio_dir.to_path_buf())),
        Err(source) if source.kind() == ErrorKind::NotFound => {
            Err(ProbeError::DirectoryMissing(audio_dir.to_path_buf()))
        }
        Err(source) => Err(ProbeError::Check {
            path: audio_dir.to_path_buf(),
            source,
        }),
    }
}
