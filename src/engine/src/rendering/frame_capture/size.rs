//! The window size the screenshot modes render and write their images at.

use winit::dpi::PhysicalSize;

pub const SIZE_ENV: &str = "OPENFO4_CAPTURE_SIZE";
pub const DEFAULT_CAPTURE_SIZE: PhysicalSize<u32> = PhysicalSize::new(1920, 1080);

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("{SIZE_ENV}={0:?} is not <width>x<height> with both at least 1")]
pub struct InvalidCaptureSize(pub String);

/// `OPENFO4_CAPTURE_SIZE` as given, `None` while it is unset.
pub fn size_from_env() -> Option<String> {
    crate::utils::optional_env_var(SIZE_ENV)
}

/// The size `text` names, or the default without one.
pub fn capture_size(text: Option<&str>) -> Result<PhysicalSize<u32>, InvalidCaptureSize> {
    text.map_or(Ok(DEFAULT_CAPTURE_SIZE), parse_size)
}

fn parse_size(text: &str) -> Result<PhysicalSize<u32>, InvalidCaptureSize> {
    let invalid = || InvalidCaptureSize(text.to_owned());
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

    #[test]
    fn size_defaults_to_full_hd() {
        assert_eq!(capture_size(None), Ok(PhysicalSize::new(1920, 1080)));
    }

    #[test]
    fn size_is_width_x_height() {
        assert_eq!(
            capture_size(Some(" 1280X720 ")),
            Ok(PhysicalSize::new(1280, 720))
        );
    }

    #[test]
    fn malformed_sizes_are_rejected() {
        for text in [
            "", "1920", "1920x", "x1080", "0x1080", "1920x0", "ax1", "-1x5", "1x2x3",
        ] {
            assert_eq!(
                capture_size(Some(text)),
                Err(InvalidCaptureSize(text.to_owned())),
                "{text:?}"
            );
        }
    }
}
