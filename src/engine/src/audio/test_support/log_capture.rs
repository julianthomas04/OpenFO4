//! A tracing writer that keeps what is logged, so a test can read the lines back.
//!
//! Captures go through one global subscriber that is interested in every callsite and hands each
//! event to the capture active on the emitting thread. A subscriber set per thread instead leaves
//! tracing's process-wide callsite cache to whichever test registers a callsite first, so parallel
//! tests could lose each other's events.

use std::cell::RefCell;
use std::io::Write;
use std::sync::{Arc, Mutex, Once};

use tracing::subscriber::Interest;
use tracing::{Metadata, Subscriber};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::{Context, Filter, Layer, SubscriberExt};

#[derive(Clone, Default)]
pub struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

struct ActiveCapture {
    buffer: SharedBuffer,
    filter: EnvFilter,
}

thread_local! {
    static ACTIVE: RefCell<Option<ActiveCapture>> = const { RefCell::new(None) };
}

/// Ends the capture of this thread when dropped.
#[must_use = "the capture ends when the guard is dropped"]
pub struct CaptureGuard(());

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        ACTIVE.with(|active| active.borrow_mut().take());
    }
}

impl SharedBuffer {
    /// Everything logged since the last call.
    pub fn drain(&self) -> String {
        String::from_utf8_lossy(&std::mem::take(&mut *self.0.lock().unwrap())).into_owned()
    }

    /// Writes the events matching the `filter` directives plain into this buffer on this thread
    /// until the guard is dropped.
    ///
    /// # Panics
    /// When this thread already captures, or another global subscriber is installed.
    pub fn capture(&self, filter: &str) -> CaptureGuard {
        install_capture_subscriber();
        ACTIVE.with(|active| {
            let mut active = active.borrow_mut();
            assert!(active.is_none(), "this thread already captures its logs");
            *active = Some(ActiveCapture {
                buffer: self.clone(),
                filter: EnvFilter::new(filter),
            });
        });
        CaptureGuard(())
    }
}

impl Write for SharedBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for SharedBuffer {
    type Writer = SharedBuffer;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn install_capture_subscriber() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_writer(ActiveWriter)
                .with_ansi(false)
                .without_time()
                .with_target(false)
                .with_filter(ActiveFilter),
        );
        tracing::subscriber::set_global_default(subscriber)
            .expect("log capture needs to own the global subscriber of the test process");
    });
}

/// Passes an event to the fmt layer when this thread's capture asks for it.
struct ActiveFilter;

impl<S: Subscriber> Filter<S> for ActiveFilter {
    fn enabled(&self, metadata: &Metadata<'_>, context: &Context<'_, S>) -> bool {
        // A thread tearing down its locals captures nothing.
        ACTIVE
            .try_with(|active| {
                active
                    .borrow()
                    .as_ref()
                    .is_some_and(|capture| Filter::<S>::enabled(&capture.filter, metadata, context))
            })
            .unwrap_or(false)
    }

    fn callsite_enabled(&self, _metadata: &'static Metadata<'static>) -> Interest {
        Interest::sometimes()
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(LevelFilter::TRACE)
    }
}

/// Writes into this thread's capture; events only reach it while one is active.
struct ActiveWriter;

impl<'a> MakeWriter<'a> for ActiveWriter {
    type Writer = SharedBuffer;

    fn make_writer(&'a self) -> Self::Writer {
        ACTIVE.with(|active| {
            active
                .borrow()
                .as_ref()
                .map(|capture| capture.buffer.clone())
                .expect("an event passed the capture filter on a thread without a capture")
        })
    }
}
