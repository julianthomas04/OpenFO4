use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use super::StartupError;

/// The receiving end of a startup worker. Dropping it detaches the worker.
pub struct BootHandle<S> {
    result: Receiver<Result<S, StartupError>>,
}

pub enum BootPoll<S> {
    Pending,
    Ready(S),
    Failed(StartupError),
}

/// Runs `work` on a named thread. A panic in `work` arrives as `WorkerPanicked`.
pub fn spawn_worker<S: Send + 'static>(
    name: &str,
    work: impl FnOnce() -> Result<S, StartupError> + Send + 'static,
) -> Result<BootHandle<S>, StartupError> {
    let (sender, result) = mpsc::channel();
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(work))
                .unwrap_or_else(|payload| Err(panic_error(payload.as_ref())));
            // The receiver is gone when the window closed during boot; nobody wants the result.
            let _ = sender.send(outcome);
        })
        .map_err(StartupError::WorkerSpawn)?;
    Ok(BootHandle { result })
}

fn panic_error(payload: &(dyn Any + Send)) -> StartupError {
    let message = payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no panic message".to_owned());
    StartupError::WorkerPanicked(message)
}

impl<S> BootHandle<S> {
    pub fn poll(&self) -> BootPoll<S> {
        match self.result.try_recv() {
            Ok(Ok(value)) => BootPoll::Ready(value),
            Ok(Err(error)) => BootPoll::Failed(error),
            Err(TryRecvError::Empty) => BootPoll::Pending,
            Err(TryRecvError::Disconnected) => BootPoll::Failed(StartupError::WorkerDisconnected),
        }
    }
}

/// What the app is doing: starting up behind `B` (the main-thread boot resources), running `S`,
/// or stopped because startup failed.
pub enum BootState<B, S> {
    Booting { boot: B, handle: BootHandle<S> },
    Running(S),
    Failed(StartupError),
}

impl<B, S> BootState<B, S> {
    pub fn booting(boot: B, handle: BootHandle<S>) -> Self {
        Self::Booting { boot, handle }
    }

    /// A finished boot becomes `Running` or `Failed`; every other state is returned unchanged.
    #[must_use]
    pub fn advance(self) -> Self {
        match self {
            Self::Booting { boot, handle } => match handle.poll() {
                BootPoll::Pending => Self::Booting { boot, handle },
                BootPoll::Ready(state) => Self::Running(state),
                BootPoll::Failed(error) => Self::Failed(error),
            },
            settled => settled,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn advance_until_settled(mut state: BootState<(), u32>) -> BootState<(), u32> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while matches!(state, BootState::Booting { .. }) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
            state = state.advance();
        }
        state
    }

    fn booting_with(
        work: impl FnOnce() -> Result<u32, StartupError> + Send + 'static,
    ) -> BootState<(), u32> {
        BootState::booting(
            (),
            spawn_worker("test-startup", work).expect("thread spawns"),
        )
    }

    fn failure_of(state: BootState<(), u32>) -> StartupError {
        match advance_until_settled(state) {
            BootState::Failed(error) => error,
            BootState::Running(value) => panic!("expected a failure, got Running({value})"),
            BootState::Booting { .. } => panic!("the worker never settled"),
        }
    }

    #[test]
    fn a_finished_worker_moves_the_state_to_running_with_its_value() {
        let state = advance_until_settled(booting_with(|| Ok(7)));
        assert!(matches!(state, BootState::Running(7)));
    }

    #[test]
    fn a_worker_error_fails_the_state_and_keeps_the_error() {
        let error = failure_of(booting_with(|| {
            Err(StartupError::Streamer(anyhow::anyhow!("no database")))
        }));
        assert!(matches!(error, StartupError::Streamer(_)), "{error:?}");
        assert!(error.to_string().contains("no database"));
    }

    #[test]
    fn a_worker_panic_is_reported_with_its_message() {
        let error = failure_of(booting_with(|| panic!("shader table corrupt")));
        assert!(
            matches!(&error, StartupError::WorkerPanicked(message) if message.contains("shader table corrupt")),
            "{error:?}"
        );
    }

    #[test]
    fn a_formatted_panic_message_is_kept() {
        let error = failure_of(booting_with(|| panic!("step {} failed", 3)));
        assert!(
            matches!(&error, StartupError::WorkerPanicked(message) if message == "step 3 failed")
        );
    }

    #[test]
    fn a_cancelled_worker_reports_a_cancellation() {
        let error = failure_of(booting_with(|| Err(StartupError::Cancelled)));
        assert!(error.is_cancellation());
    }

    #[test]
    fn a_dropped_sender_is_reported_as_a_disconnect() {
        let (sender, result) = mpsc::channel::<Result<u32, StartupError>>();
        drop(sender);
        let error = failure_of(BootState::booting((), BootHandle { result }));
        assert!(matches!(error, StartupError::WorkerDisconnected));
    }

    #[test]
    fn a_worker_that_has_not_finished_leaves_the_state_booting() {
        let (_sender, result) = mpsc::channel::<Result<u32, StartupError>>();
        let state = BootState::booting((), BootHandle { result }).advance();
        assert!(matches!(state, BootState::Booting { .. }));
    }

    #[test]
    fn advancing_a_settled_state_changes_nothing() {
        assert!(matches!(
            BootState::<(), u32>::Running(1).advance(),
            BootState::Running(1)
        ));
        assert!(matches!(
            BootState::<(), u32>::Failed(StartupError::Cancelled).advance(),
            BootState::Failed(StartupError::Cancelled)
        ));
    }
}
