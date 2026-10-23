//! Engine startup off the event-loop thread: the main thread keeps the window alive with black
//! frames while a worker builds the renderer state.

mod cancel;
mod error;
mod handoff;
mod timer;
mod worker;

pub use cancel::CancelToken;
pub use error::StartupError;
pub use handoff::{BootHandle, BootPoll, BootState, spawn_worker};
pub use timer::StartupTimer;
pub use worker::run_startup;
