//! The environment contract of the Pip-Boy screenshot mode.

use std::path::PathBuf;

use winit::dpi::PhysicalSize;

use crate::rendering::frame_capture::{InvalidCaptureSize, capture_size, size_from_env};

pub const DIR_ENV: &str = "OPENFO4_PIPBOY_CAPTURE";

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum CaptureConfigError {
    #[error("{DIR_ENV} is empty; expected a directory for the screenshots")]
    EmptyDirectory,
    #[error(transparent)]
    InvalidSize(#[from] InvalidCaptureSize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureConfig {
    pub directory: PathBuf,
    pub window_size: PhysicalSize<u32>,
}

impl CaptureConfig {
    /// `None` when `OPENFO4_PIPBOY_CAPTURE` is unset.
    pub fn from_env() -> Result<Option<Self>, CaptureConfigError> {
        Self::parse(
            crate::utils::optional_env_var(DIR_ENV),
            size_from_env().as_deref(),
        )
    }

    fn parse(
        directory: Option<String>,
        size: Option<&str>,
    ) -> Result<Option<Self>, CaptureConfigError> {
        let Some(directory) = directory else {
            return Ok(None);
        };
        if directory.trim().is_empty() {
            return Err(CaptureConfigError::EmptyDirectory);
        }
        let window_size = capture_size(size)?;
        Ok(Some(Self {
            directory: PathBuf::from(directory),
            window_size,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(
        directory: Option<&str>,
        size: Option<&str>,
    ) -> Result<Option<CaptureConfig>, CaptureConfigError> {
        CaptureConfig::parse(directory.map(str::to_owned), size)
    }

    #[test]
    fn unset_directory_disables_capture() {
        assert_eq!(parse(None, Some("800x600")), Ok(None));
    }

    #[test]
    fn the_directory_and_size_are_taken_as_given() {
        let config = parse(Some("shots"), Some("1280x720")).unwrap().unwrap();
        assert_eq!(config.directory, PathBuf::from("shots"));
        assert_eq!(config.window_size, PhysicalSize::new(1280, 720));
    }

    #[test]
    fn a_malformed_size_is_rejected() {
        assert_eq!(
            parse(Some("shots"), Some("1920")),
            Err(CaptureConfigError::InvalidSize(InvalidCaptureSize(
                "1920".to_owned()
            )))
        );
    }

    #[test]
    fn blank_directory_is_rejected() {
        assert_eq!(
            parse(Some("  "), None),
            Err(CaptureConfigError::EmptyDirectory)
        );
    }
}
