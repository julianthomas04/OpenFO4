use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::StartupError;

/// Raised by the main thread when the app exits during startup; the worker polls it between steps.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    pub fn check(&self) -> Result<(), StartupError> {
        if self.is_cancelled() {
            Err(StartupError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_token_lets_startup_continue() {
        assert!(CancelToken::default().check().is_ok());
    }

    #[test]
    fn cancelling_one_clone_cancels_them_all() {
        let token = CancelToken::default();
        let worker_copy = token.clone();
        token.cancel();
        assert!(matches!(worker_copy.check(), Err(StartupError::Cancelled)));
        assert!(worker_copy.is_cancelled());
    }
}
