//! The sound effect files below the audio folder, decoded into memory the first time they are
//! asked for.

use std::collections::HashMap;
use std::path::PathBuf;

use kira::sound::static_sound::StaticSoundData;

use super::error::AudioError;

pub struct DecodedFiles {
    audio_dir: PathBuf,
    /// Decoded files by file key; a clone shares the samples.
    decoded: HashMap<String, StaticSoundData>,
}

impl DecodedFiles {
    pub fn new(audio_dir: PathBuf) -> Self {
        Self {
            audio_dir,
            decoded: HashMap::new(),
        }
    }

    pub fn path(&self, file_key: &str) -> PathBuf {
        self.audio_dir.join(file_key)
    }

    /// The decoded file `file_key`; the first request for a file decodes it. A file that cannot be
    /// opened or decoded is an error.
    pub fn get(&mut self, file_key: &str) -> Result<StaticSoundData, AudioError> {
        if let Some(data) = self.decoded.get(file_key) {
            return Ok(data.clone());
        }
        let path = self.path(file_key);
        let data = StaticSoundData::from_file(&path)
            .map_err(|source| AudioError::SfxFile { path, source })?;
        self.decoded.insert(file_key.to_string(), data.clone());
        Ok(data)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.decoded.len()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::audio::test_support::{
        HALF_SECOND_SAMPLES, SAMPLE_RATE, sine_flac, write_audio_file,
    };

    const FILE: &str = "sound/fx/fst/step.flac";

    fn files_with(file_key: &str) -> (tempfile::TempDir, DecodedFiles) {
        let root = tempfile::tempdir().unwrap();
        write_audio_file(
            root.path(),
            file_key,
            &sine_flac(SAMPLE_RATE, HALF_SECOND_SAMPLES),
        );
        let files = DecodedFiles::new(root.path().join("audio"));
        (root, files)
    }

    #[test]
    fn a_file_is_decoded_on_the_first_request_and_later_requests_share_its_samples() {
        let (_root, mut files) = files_with(FILE);

        let first = files.get(FILE).unwrap();
        let second = files.get(FILE).unwrap();

        assert!(Arc::ptr_eq(&first.frames, &second.frames));
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn a_file_that_is_absent_is_an_error_naming_its_path_and_is_not_kept() {
        let (_root, mut files) = files_with(FILE);

        let error = files.get("sound/fx/fst/absent.flac").unwrap_err();

        assert!(
            matches!(&error, AudioError::SfxFile { path, .. } if path.ends_with("absent.flac")),
            "{error}"
        );
        assert_eq!(files.len(), 0);
    }
}
