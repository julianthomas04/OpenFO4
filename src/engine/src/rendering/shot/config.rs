//! The environment contract of the single-screenshot mode.

use std::path::PathBuf;

use winit::dpi::PhysicalSize;

use crate::rendering::frame_capture::{InvalidCaptureSize, capture_size, size_from_env};
use crate::rendering::pipboy_capture::CAPTURE_DIR_ENV;

pub const SHOT_ENV: &str = "OPENFO4_SHOT";
const SETTLE_FRAMES_ENV: &str = "OPENFO4_SHOT_SETTLE_FRAMES";
/// Consecutive ready frames before the shot: a second at 60 fps, long enough for rain and other
/// particles to fill the view after the world settled.
const DEFAULT_SETTLE_FRAMES: u32 = 60;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ShotConfigError {
    #[error("{SHOT_ENV} is empty; expected the path of the .png to write")]
    EmptyPath,
    #[error("{SHOT_ENV}={0:?} does not end in .png")]
    NotPng(String),
    #[error(transparent)]
    InvalidSize(#[from] InvalidCaptureSize),
    #[error("{SETTLE_FRAMES_ENV}={0:?} is not a whole number of at least 1")]
    InvalidSettleFrames(String),
    #[error("{SHOT_ENV} and {CAPTURE_DIR_ENV} cannot be set together")]
    PipBoyCaptureAlsoSet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShotConfig {
    pub path: PathBuf,
    pub window_size: PhysicalSize<u32>,
    /// Consecutive frames the world must be ready before it is photographed.
    pub settle_frames: u32,
}

/// The environment variables the shot mode reads, as given.
#[derive(Debug, Default)]
struct ShotEnv {
    path: Option<String>,
    size: Option<String>,
    settle_frames: Option<String>,
    pipboy_capture_set: bool,
}

/// Whether `OPENFO4_SHOT` is set, valid or not.
pub fn shot_requested() -> bool {
    std::env::var_os(SHOT_ENV).is_some()
}

impl ShotConfig {
    /// `None` when `OPENFO4_SHOT` is unset.
    pub fn from_env() -> Result<Option<Self>, ShotConfigError> {
        Self::parse(ShotEnv {
            path: crate::utils::optional_env_var(SHOT_ENV),
            size: size_from_env(),
            settle_frames: crate::utils::optional_env_var(SETTLE_FRAMES_ENV),
            pipboy_capture_set: std::env::var_os(CAPTURE_DIR_ENV).is_some(),
        })
    }

    fn parse(env: ShotEnv) -> Result<Option<Self>, ShotConfigError> {
        let Some(path) = env.path else {
            return Ok(None);
        };
        if env.pipboy_capture_set {
            return Err(ShotConfigError::PipBoyCaptureAlsoSet);
        }
        if path.trim().is_empty() {
            return Err(ShotConfigError::EmptyPath);
        }
        let path = PathBuf::from(path);
        let is_png = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("png"));
        if !is_png {
            return Err(ShotConfigError::NotPng(path.display().to_string()));
        }
        Ok(Some(Self {
            path,
            window_size: capture_size(env.size.as_deref())?,
            settle_frames: env
                .settle_frames
                .map_or(Ok(DEFAULT_SETTLE_FRAMES), |text| parse_settle_frames(&text))?,
        }))
    }
}

fn parse_settle_frames(text: &str) -> Result<u32, ShotConfigError> {
    text.trim()
        .parse()
        .ok()
        .filter(|frames| *frames >= 1)
        .ok_or_else(|| ShotConfigError::InvalidSettleFrames(text.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot(path: &str) -> ShotEnv {
        ShotEnv {
            path: Some(path.to_owned()),
            ..ShotEnv::default()
        }
    }

    #[test]
    fn an_unset_path_disables_the_shot() {
        let env = ShotEnv {
            size: Some("800x600".to_owned()),
            pipboy_capture_set: true,
            ..ShotEnv::default()
        };
        assert_eq!(ShotConfig::parse(env), Ok(None));
    }

    #[test]
    fn defaults_are_full_hd_and_sixty_settle_frames() {
        let config = ShotConfig::parse(shot("out/view.png")).unwrap().unwrap();
        assert_eq!(config.path, PathBuf::from("out/view.png"));
        assert_eq!(config.window_size, PhysicalSize::new(1920, 1080));
        assert_eq!(config.settle_frames, 60);
    }

    #[test]
    fn size_and_settle_frames_are_taken_as_given() {
        let env = ShotEnv {
            size: Some("1280x720".to_owned()),
            settle_frames: Some(" 5 ".to_owned()),
            ..shot("view.PNG")
        };
        let config = ShotConfig::parse(env).unwrap().unwrap();
        assert_eq!(config.window_size, PhysicalSize::new(1280, 720));
        assert_eq!(config.settle_frames, 5);
    }

    #[test]
    fn the_path_must_name_a_png() {
        assert_eq!(
            ShotConfig::parse(shot(" ")),
            Err(ShotConfigError::EmptyPath)
        );
        for path in ["view.jpg", "view", "png"] {
            assert_eq!(
                ShotConfig::parse(shot(path)),
                Err(ShotConfigError::NotPng(path.to_owned())),
                "{path:?}"
            );
        }
    }

    #[test]
    fn invalid_settle_frames_are_rejected() {
        for text in ["0", "-1", "1.5", "many", ""] {
            let env = ShotEnv {
                settle_frames: Some(text.to_owned()),
                ..shot("view.png")
            };
            assert_eq!(
                ShotConfig::parse(env),
                Err(ShotConfigError::InvalidSettleFrames(text.to_owned())),
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_malformed_size_is_rejected() {
        let env = ShotEnv {
            size: Some("big".to_owned()),
            ..shot("view.png")
        };
        assert_eq!(
            ShotConfig::parse(env),
            Err(ShotConfigError::InvalidSize(InvalidCaptureSize(
                "big".to_owned()
            )))
        );
    }

    #[test]
    fn the_pipboy_capture_cannot_run_alongside() {
        let env = ShotEnv {
            pipboy_capture_set: true,
            ..shot("view.png")
        };
        assert_eq!(
            ShotConfig::parse(env),
            Err(ShotConfigError::PipBoyCaptureAlsoSet)
        );
    }
}
