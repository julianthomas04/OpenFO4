//! Startup check of the converted sound files a feature can play: absent files are skipped and
//! logged once, every other problem is an error.

use std::collections::{BTreeSet, HashSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use kira::sound::FromFileError;

use super::music_output::open_stream;
use super::sound_resolve::ReachableSound;

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("the audio folder {0} is missing; run the installer")]
    DirectoryMissing(PathBuf),
    #[error("cannot check {path}: {source}")]
    Check {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "{label} {path} cannot be opened; the installer converts it from the game, \
         so run the installer again: {source}"
    )]
    Open {
        label: String,
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
        open(&path).map_err(|source| ProbeError::Open {
            label: label.to_string(),
            path,
            source,
        })?;
    }
    Ok(missing)
}

/// `probe_files` for the files of `sounds`, each opened as playback opens it. A sound left with no
/// file is logged once as `<label> <descriptor> has no file left; it is silent`.
pub(super) fn probe_sounds(
    label: &str,
    sounds: &[ReachableSound],
    audio_dir: &Path,
) -> Result<HashSet<String>, ProbeError> {
    let file_keys = sounds
        .iter()
        .flat_map(|sound| sound.file_keys.iter().map(String::as_str));
    let missing = probe_files(label, file_keys, audio_dir, |path| {
        open_stream(path, false).map(|_| ())
    })?;
    for sound in sounds
        .iter()
        .filter(|sound| sound.file_keys.iter().all(|key| missing.contains(key)))
    {
        tracing::warn!(
            "{label} {} has no file left; it is silent",
            sound.descriptor
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::test_support::{
        HALF_SECOND_SAMPLES, SAMPLE_RATE, SharedBuffer, sine_flac, write_audio_file,
    };

    const PRESENT: &str = "sound/present.flac";
    const ABSENT: &str = "sound/absent.flac";
    const OTHER_ABSENT: &str = "sound/other_absent.flac";

    fn sound(descriptor: &str, file_keys: &[&str]) -> ReachableSound {
        ReachableSound {
            descriptor: descriptor.to_string(),
            file_keys: file_keys.iter().map(ToString::to_string).collect(),
        }
    }

    fn install() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        write_audio_file(
            root.path(),
            PRESENT,
            &sine_flac(SAMPLE_RATE, HALF_SECOND_SAMPLES),
        );
        root
    }

    fn audio_dir(root: &tempfile::TempDir) -> PathBuf {
        root.path().join("audio")
    }

    #[test]
    fn the_absent_files_are_returned_and_each_is_logged_once() {
        let root = install();
        let log = SharedBuffer::default();
        let _guard = log.capture("openfo4=warn");
        let sounds = [sound("a", &[PRESENT, ABSENT]), sound("b", &[ABSENT])];

        let missing = probe_sounds("test sound", &sounds, &audio_dir(&root)).unwrap();

        assert_eq!(missing, HashSet::from([ABSENT.to_string()]));
        let logged = log.drain();
        let missing_lines = logged.lines().filter(|line| line.contains("is missing"));
        assert_eq!(missing_lines.count(), 1, "{logged}");
        assert!(logged.contains("test sound"), "{logged}");
    }

    #[test]
    fn a_sound_left_with_no_file_is_logged_and_one_with_a_file_left_is_not() {
        let root = install();
        let log = SharedBuffer::default();
        let _guard = log.capture("openfo4=warn");
        let sounds = [
            sound("kept", &[PRESENT, ABSENT]),
            sound("lost", &[ABSENT, OTHER_ABSENT]),
        ];

        probe_sounds("test sound", &sounds, &audio_dir(&root)).unwrap();

        let logged = log.drain();
        let silent_lines: Vec<&str> = logged
            .lines()
            .filter(|line| line.contains("has no file left"))
            .collect();
        assert_eq!(silent_lines.len(), 1, "{logged}");
        assert!(
            silent_lines[0].contains("test sound lost has no file left"),
            "{logged}"
        );
    }

    #[test]
    fn a_missing_audio_folder_is_an_error() {
        let root = tempfile::tempdir().unwrap();

        let error = probe_sounds("test sound", &[], &audio_dir(&root)).unwrap_err();

        assert!(
            matches!(&error, ProbeError::DirectoryMissing(path) if *path == audio_dir(&root)),
            "{error:?}"
        );
    }

    #[test]
    fn a_file_that_cannot_be_opened_is_an_error_naming_the_kind_of_sound_and_the_file() {
        let root = install();
        write_audio_file(root.path(), PRESENT, b"not audio");

        let error =
            probe_sounds("test sound", &[sound("a", &[PRESENT])], &audio_dir(&root)).unwrap_err();

        assert!(
            matches!(&error, ProbeError::Open { label, path, .. }
                if label == "test sound" && path.ends_with("present.flac")),
            "{error:?}"
        );
        assert!(error.to_string().starts_with("test sound "), "{error}");
    }
}
