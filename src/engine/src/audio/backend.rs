use kira::backend::Backend;
use kira::backend::cpal::CpalBackend;
#[cfg(any(test, feature = "test-support"))]
use kira::backend::mock::MockBackend;

/// A stream error a backend reports while running. `kind` is a small fixed set of categories,
/// so errors can be counted by it without the count growing with the varying message text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamError {
    pub kind: String,
    pub message: String,
}

/// What the audio system needs of a kira backend besides playing: a description of its setup
/// failure and the stream errors it reports while running.
pub trait AudioBackend: Backend {
    fn describe_setup_error(error: &Self::Error) -> String;
    fn pop_stream_error(&mut self) -> Option<StreamError>;
    /// How many stream errors were dropped because they were not polled in time.
    fn stream_errors_discarded(&self) -> u64;
}

impl AudioBackend for CpalBackend {
    fn describe_setup_error(error: &Self::Error) -> String {
        error.to_string()
    }

    fn pop_stream_error(&mut self) -> Option<StreamError> {
        self.pop_error().map(|error| StreamError {
            kind: error.kind().to_string(),
            message: error.to_string(),
        })
    }

    /// kira reports `None` while the stream is not initialised; no stream means nothing was
    /// dropped, so the count is 0.
    fn stream_errors_discarded(&self) -> u64 {
        self.num_stream_errors_discarded().unwrap_or_default()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl AudioBackend for MockBackend {
    fn describe_setup_error(_error: &Self::Error) -> String {
        "the mock backend failed to set up".to_string()
    }

    fn pop_stream_error(&mut self) -> Option<StreamError> {
        None
    }

    fn stream_errors_discarded(&self) -> u64 {
        0
    }
}
