//! The environment contract of the Pip-Boy screenshot mode.

use std::path::PathBuf;

use winit::dpi::PhysicalSize;

pub const DIR_ENV: &str = "OPENFO4_PIPBOY_CAPTURE";
const SIZE_ENV: &str = "OPENFO4_CAPTURE_SIZE";
pub const DEFAULT_WINDOW_SIZE: PhysicalSize<u32> = PhysicalSize::new(1920, 1080);

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum CaptureConfigError {
    #[error("{DIR_ENV} is empty; expected a directory for the screenshots")]
    EmptyDirectory,
    #[error("{SIZE_ENV}={0:?} is not <width>x<height> with both at least 1")]
    InvalidSize(String),
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
            crate::utils::optional_env_var(SIZE_ENV),
        )
    }

    fn parse(
        directory: Option<String>,
        size: Option<String>,
    ) -> Result<Option<Self>, CaptureConfigError> {
        let Some(directory) = directory else {
            return Ok(None);
        };
        if directory.trim().is_empty() {
            return Err(CaptureConfigError::EmptyDirectory);
        }
        let window_size = size.map_or(Ok(DEFAULT_WINDOW_SIZE), |text| parse_size(&text))?;
        Ok(Some(Self {
            directory: PathBuf::from(directory),
            window_size,
        }))
    }
}

fn parse_size(text: &str) -> Result<PhysicalSize<u32>, CaptureConfigError> {
    let invalid = || CaptureConfigError::InvalidSize(text.to_owned());
    let (width, height) = text.trim().split_once(['x', 'X']).ok_or_else(invalid)?;
    let width: u32 = width.trim().parse().map_err(|_| invalid())?;
    let height: u32 = height.trim().parse().map_err(|_| invalid())?;
    if width == 0 || height == 0 {
        return Err(invalid());
    }
    Ok(PhysicalSize::new(width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(
        directory: Option<&str>,
        size: Option<&str>,
    ) -> Result<Option<CaptureConfig>, CaptureConfigError> {
        CaptureConfig::parse(directory.map(str::to_owned), size.map(str::to_owned))
    }

    #[test]
    fn unset_directory_disables_capture() {
        assert_eq!(parse(None, Some("800x600")), Ok(None));
    }

    #[test]
    fn size_defaults_to_full_hd() {
        let config = parse(Some("shots"), None).unwrap().unwrap();
        assert_eq!(config.directory, PathBuf::from("shots"));
        assert_eq!(config.window_size, PhysicalSize::new(1920, 1080));
    }

    #[test]
    fn size_is_width_x_height() {
        let config = parse(Some("shots"), Some(" 1280X720 ")).unwrap().unwrap();
        assert_eq!(config.window_size, PhysicalSize::new(1280, 720));
    }

    #[test]
    fn malformed_sizes_are_rejected() {
        for text in [
            "", "1920", "1920x", "x1080", "0x1080", "1920x0", "ax1", "-1x5", "1x2x3",
        ] {
            assert_eq!(
                parse(Some("shots"), Some(text)),
                Err(CaptureConfigError::InvalidSize(text.to_owned())),
                "{text:?}"
            );
        }
    }

    #[test]
    fn blank_directory_is_rejected() {
        assert_eq!(
            parse(Some("  "), None),
            Err(CaptureConfigError::EmptyDirectory)
        );
    }
}
