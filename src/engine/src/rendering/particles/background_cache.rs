//! A cache whose values are built on a worker thread, so the frame loop never parses a model
//! file: a lookup answers `Pending` until the worker has produced the value.

use std::collections::HashMap;
use std::sync::{Arc, mpsc};

/// What a lookup found.
#[derive(Debug)]
pub enum Lookup<V> {
    /// The worker has not finished (or started) this key; ask again later.
    Pending,
    /// The loader produced nothing for this key (it reported why itself).
    Unavailable,
    Ready(Arc<V>),
}

enum Slot<V> {
    Loading,
    Done(Option<Arc<V>>),
}

pub struct BackgroundCache<V> {
    requests: Option<mpsc::Sender<String>>,
    results: mpsc::Receiver<(String, Option<V>)>,
    slots: HashMap<String, Slot<V>>,
    pending: usize,
}

impl<V: Send + 'static> BackgroundCache<V> {
    /// `load` runs on a thread named `name`; `None` means "no value" and is cached.
    pub fn new(name: &str, load: impl Fn(&str) -> Option<V> + Send + 'static) -> Self {
        let (request_sender, request_receiver) = mpsc::channel::<String>();
        let (result_sender, results) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                for key in request_receiver {
                    let value = load(&key);
                    if result_sender.send((key, value)).is_err() {
                        break;
                    }
                }
            });
        let requests = match spawned {
            Ok(_) => Some(request_sender),
            Err(error) => {
                tracing::error!("cannot start the {name} thread: {error}; nothing loads");
                None
            }
        };
        Self {
            requests,
            results,
            slots: HashMap::new(),
            pending: 0,
        }
    }

    /// The value for `key`, starting its load on first use.
    pub fn get(&mut self, key: &str) -> Lookup<V> {
        if self.pending > 0 {
            self.drain();
        }
        match self.slots.get(key) {
            Some(Slot::Done(Some(value))) => Lookup::Ready(Arc::clone(value)),
            Some(Slot::Done(None)) => Lookup::Unavailable,
            Some(Slot::Loading) => Lookup::Pending,
            None => self.request(key),
        }
    }

    fn request(&mut self, key: &str) -> Lookup<V> {
        let sent = self
            .requests
            .as_ref()
            .is_some_and(|requests| requests.send(key.to_owned()).is_ok());
        if sent {
            self.pending += 1;
            self.slots.insert(key.to_owned(), Slot::Loading);
            Lookup::Pending
        } else {
            self.slots.insert(key.to_owned(), Slot::Done(None));
            Lookup::Unavailable
        }
    }

    fn accept(&mut self, key: String, value: Option<V>) {
        self.pending = self.pending.saturating_sub(1);
        self.slots.insert(key, Slot::Done(value.map(Arc::new)));
    }

    fn drain(&mut self) {
        while let Ok((key, value)) = self.results.try_recv() {
            self.accept(key, value);
        }
    }

    /// Blocks until every requested key has finished loading.
    #[cfg(test)]
    pub fn wait_idle(&mut self) {
        while self.pending > 0 {
            let (key, value) = self
                .results
                .recv()
                .expect("the loader thread outlives its cache");
            self.accept(key, value);
        }
    }

    /// Stores a finished value without loading it.
    #[cfg(test)]
    pub fn insert_ready(&mut self, key: &str, value: Option<V>) {
        self.slots
            .insert(key.to_owned(), Slot::Done(value.map(Arc::new)));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn a_lookup_is_pending_until_the_worker_finishes_and_loads_once() {
        let loads = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&loads);
        let mut cache = BackgroundCache::new("test-loader", move |key| {
            counter.fetch_add(1, Ordering::SeqCst);
            (key != "missing").then_some(key.len())
        });

        assert!(matches!(cache.get("abc"), Lookup::Pending));
        assert!(matches!(cache.get("missing"), Lookup::Pending));
        cache.wait_idle();

        assert!(matches!(cache.get("abc"), Lookup::Ready(value) if *value == 3));
        assert!(matches!(cache.get("missing"), Lookup::Unavailable));
        assert_eq!(loads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_loader_never_runs_on_the_calling_thread() {
        let loader_thread = Arc::new(Mutex::new(None));
        let record = Arc::clone(&loader_thread);
        let mut cache = BackgroundCache::new("test-loader", move |_| {
            *record.lock().unwrap() = Some(std::thread::current().id());
            Some(())
        });

        let _ = cache.get("key");
        cache.wait_idle();

        let loader = loader_thread.lock().unwrap().expect("the loader ran");
        assert_ne!(loader, std::thread::current().id());
    }
}
