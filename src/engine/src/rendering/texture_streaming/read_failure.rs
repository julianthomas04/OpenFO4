//! Why reading a texture failed: the file is absent, which its stand-in covers, or the file is
//! broken, which ends the game.

use crate::assets::types::texture::TextureError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadFailure {
    Missing,
    Broken(String),
}

impl From<&TextureError> for ReadFailure {
    fn from(error: &TextureError) -> Self {
        if error.is_missing_file() {
            Self::Missing
        } else {
            Self::Broken(error.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn io_error(kind: std::io::ErrorKind) -> TextureError {
        TextureError::Io {
            path: "textures/a.dds".to_owned(),
            source: std::io::Error::from(kind),
        }
    }

    #[test]
    fn a_file_that_is_not_there_is_missing() {
        assert_eq!(
            ReadFailure::from(&io_error(std::io::ErrorKind::NotFound)),
            ReadFailure::Missing
        );
    }

    #[test]
    fn any_other_io_error_is_broken() {
        let failure = ReadFailure::from(&io_error(std::io::ErrorKind::PermissionDenied));
        assert!(
            matches!(failure, ReadFailure::Broken(reason) if reason.contains("textures/a.dds"))
        );
    }

    #[test]
    fn a_file_that_is_no_dds_is_broken() {
        let error = TextureError::BadMagic {
            path: "textures/a.dds".to_owned(),
        };
        assert_eq!(
            ReadFailure::from(&error),
            ReadFailure::Broken("textures/a.dds is not a DDS file".to_owned())
        );
    }
}
