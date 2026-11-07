//! Why reading a texture failed: the file is absent, which its stand-in covers; the read hit a
//! condition that may pass, so it is tried again; or the file is broken, which ends the game.

use crate::assets::types::texture::TextureError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadFailure {
    Missing,
    Transient(String),
    Broken(String),
}

impl From<&TextureError> for ReadFailure {
    fn from(error: &TextureError) -> Self {
        if error.is_missing_file() {
            Self::Missing
        } else if error.is_transient() {
            Self::Transient(error.to_string())
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
    fn a_read_that_may_pass_is_transient() {
        for kind in [
            std::io::ErrorKind::Interrupted,
            std::io::ErrorKind::TimedOut,
            std::io::ErrorKind::WouldBlock,
            std::io::ErrorKind::ResourceBusy,
        ] {
            assert!(matches!(
                ReadFailure::from(&io_error(kind)),
                ReadFailure::Transient(_)
            ));
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_file_another_process_holds_is_transient() {
        for code in [32, 33] {
            let error = TextureError::Io {
                path: "textures/a.dds".to_owned(),
                source: std::io::Error::from_raw_os_error(code),
            };
            assert!(matches!(
                ReadFailure::from(&error),
                ReadFailure::Transient(_)
            ));
        }
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
