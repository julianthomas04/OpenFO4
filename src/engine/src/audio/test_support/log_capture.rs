//! A tracing writer that keeps what is logged, so a test can read the lines back.

use std::io::Write;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

impl SharedBuffer {
    /// Everything logged since the last call.
    pub fn drain(&self) -> String {
        String::from_utf8_lossy(&std::mem::take(&mut *self.0.lock().unwrap())).into_owned()
    }

    /// Writes the events matching the `filter` directives plain into this buffer on this thread
    /// until the guard is dropped.
    pub fn capture(&self, filter: &str) -> tracing::subscriber::DefaultGuard {
        tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
                .with_writer(self.clone())
                .with_ansi(false)
                .without_time()
                .with_target(false)
                .finish(),
        )
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

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedBuffer {
    type Writer = SharedBuffer;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
