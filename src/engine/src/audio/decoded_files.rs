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
