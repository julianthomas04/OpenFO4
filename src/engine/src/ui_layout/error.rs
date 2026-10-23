use std::fmt;
use std::path::PathBuf;

/// Why a UI layout lookup or a layout document failed. Every message names the movie (or
/// document) and the key, so a failure at boot says which extracted value is absent.
#[derive(Debug, thiserror::Error)]
pub enum UiLayoutError {
    #[error("UI layout movie '{movie}' is not installed (expected {path}); re-run the installer")]
    MissingMovie { movie: String, path: PathBuf },
    #[error("UI layout movie '{movie}' cannot be read from {path}: {source}")]
    Unreadable {
        movie: String,
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("UI layout movie '{movie}' has no {kind} '{key}'{}", Nearby(.nearby))]
    MissingKey {
        movie: String,
        key: String,
        kind: &'static str,
        nearby: Vec<String>,
    },
    #[error("UI layout movie '{movie}': '{key}' cannot be resolved ({reason})")]
    Unresolved {
        movie: String,
        key: String,
        reason: String,
    },
    #[error(
        "UI layout movie '{movie}': '{key}' has {actual} values, the engine expects {expected}"
    )]
    CountMismatch {
        movie: String,
        key: String,
        expected: usize,
        actual: usize,
    },
    #[error("UI layout movie '{movie}': '{key}' is {actual}, the engine expects {expected}")]
    WrongKind {
        movie: String,
        key: String,
        expected: &'static str,
        actual: String,
    },
    #[error("UI layout document '{movie}' is not usable: {reason}")]
    BadFormat { movie: String, reason: String },
    #[error("{}", Failures(.failures))]
    Invalid { failures: Vec<UiLayoutError> },
}

impl UiLayoutError {
    /// A value of `key` in `movie` that exists but cannot be used, for `reason`.
    pub fn unresolved(
        movie: impl Into<String>,
        key: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::Unresolved {
            movie: movie.into(),
            key: key.into(),
            reason: reason.into(),
        }
    }
}

struct Nearby<'a>(&'a [String]);

impl fmt::Display for Nearby<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return Ok(());
        }
        write!(f, "; similar keys: {}", self.0.join(", "))
    }
}

struct Failures<'a>(&'a [UiLayoutError]);

impl fmt::Display for Failures<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the installed UI layout does not satisfy this engine ({} problems; re-run the installer's interface step if the extractor changed):",
            self.0.len()
        )?;
        for failure in self.0 {
            write!(f, "\n  - {failure}")?;
        }
        Ok(())
    }
}
