//! Bounded concurrent load-cache shared by the streaming worker pool.
//!
//! One structure replaces the four near-identical worker-local LRU caches
//! (mesh / collider sidecar / DDS / rapier shape bake): a weight-bounded map
//! with tick-LRU eviction, safe to call from any number of worker threads.
//! The loader runs OUTSIDE the lock (it's the expensive part — GLB parse,
//! DDS read, trimesh bake), and an in-flight table dedups concurrent misses:
//! when two workers want the same uncached key, one loads it and the other
//! blocks on the same `OnceLock` instead of loading it twice.
//!
//! Loaders must be infallible by construction — failures are encoded in the
//! value type (`None` / empty vec), exactly like the caches this replaces —
//! so a claimed in-flight cell is always set and waiters never hang.

use std::collections::HashMap;
use std::collections::hash_map::Entry as MapEntry;
use std::hash::Hash;
use std::sync::{Arc, Mutex, OnceLock};

use rayon::prelude::*;

struct CacheEntry<V> {
    value: V,
    weight: u64,
    last_use: u64,
}

struct State<K, V> {
    map: HashMap<K, CacheEntry<V>>,
    /// Keys some thread is currently loading. The owner (the thread that
    /// inserted the cell) sets it and removes the key; waiters block on
    /// `OnceLock::wait`.
    inflight: HashMap<K, Arc<OnceLock<V>>>,
    total_weight: u64,
    tick: u64,
}

/// Miss-batches at least this big are loaded via the dedicated loader pool;
/// smaller ones load inline on the calling thread (see the call site).
/// Inline-everything was measured twice (2026-06-12) and rejected twice:
/// 16-31 threads decoding concurrently tax the main thread through SMT and
/// memory bandwidth — below-normal thread priority does not help with
/// either — and integration throughput is what the spawn actually waits on.
const PAR_LOAD_THRESHOLD: usize = 8;

/// Dedicated pool for big miss-batches, NOT the global rayon pool. During a
/// loading-mode spawn nearly every batch is big (everything is fresh), and
/// many workers fanning batches into the 32-thread global pool kept it
/// constantly waking/spinning — 105+ CPU-seconds of overhead and slower
/// wall time (measured 2026-06-12, see `logs/bench_cpf`). The pool runs at
/// below-normal thread priority: that is what lets it be wide — without the
/// priority drop, any loader configuration wide enough to keep up with the
/// workers halved the main thread's frame rate and integration throughput
/// collapsed ~5x; with it, the main thread always wins the scheduler.
/// `OPENFO4_LOADER_THREADS` overrides the width for benchmarking.
fn loader_pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        let threads = crate::utils::env_or_valid("OPENFO4_LOADER_THREADS", 4, |&n: &usize| n >= 1);
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("cache_loader_{i}"))
            .start_handler(|_| crate::utils::lower_thread_priority())
            .build()
            .expect("loader thread pool")
    })
}

pub struct SharedCache<K, V> {
    state: Mutex<State<K, V>>,
    /// Maximum total weight before LRU eviction. With a weigher of `|_| 1`
    /// this is an entry cap; with a byte weigher it's a byte cap.
    max_weight: u64,
}

impl<K, V> SharedCache<K, V>
where
    K: Eq + Hash + Clone + Send + Sync,
    V: Clone + Send + Sync,
{
    pub fn new(max_weight: u64) -> Self {
        Self {
            state: Mutex::new(State {
                map: HashMap::new(),
                inflight: HashMap::new(),
                total_weight: 0,
                tick: 0,
            }),
            max_weight,
        }
    }

    /// Resolve every key to its value, loading uncached ones in parallel
    /// (rayon) outside the lock. Clones keep results alive past any eviction.
    /// Zero-weight values are pinned: they never count against the cap and
    /// never evict (used for negative entries — missing files cost nothing
    /// to keep and everything to re-probe).
    pub fn resolve(
        &self,
        keys: &[K],
        loader: impl Fn(&K) -> V + Sync,
        weigher: impl Fn(&V) -> u64,
    ) -> HashMap<K, V> {
        let mut out: HashMap<K, V> = HashMap::with_capacity(keys.len());
        // Misses this call claimed (it must load them and set their cells)
        // vs. misses another thread is already loading (wait on its cell).
        let mut owned: Vec<(K, Arc<OnceLock<V>>)> = Vec::new();
        let mut waiting: Vec<(K, Arc<OnceLock<V>>)> = Vec::new();

        let tick = {
            let mut state = self.state.lock().unwrap();
            state.tick += 1;
            let tick = state.tick;
            for key in keys {
                if let Some(entry) = state.map.get_mut(key) {
                    entry.last_use = tick;
                    out.insert(key.clone(), entry.value.clone());
                } else {
                    match state.inflight.entry(key.clone()) {
                        MapEntry::Occupied(e) => waiting.push((key.clone(), e.get().clone())),
                        MapEntry::Vacant(e) => {
                            let cell = Arc::new(OnceLock::new());
                            e.insert(cell.clone());
                            owned.push((key.clone(), cell));
                        }
                    }
                }
            }
            tick
        };

        if !owned.is_empty() {
            // Small miss-batches load inline on the calling thread —
            // cross-key parallelism comes from the worker pool itself. Big
            // batches (initial loads, ring shifts) fan out on the dedicated
            // loader pool (see loader_pool for why not the global one).
            if owned.len() >= PAR_LOAD_THRESHOLD {
                loader_pool().install(|| {
                    owned.par_iter().for_each(|(key, cell)| {
                        // set() can't fail: inflight claiming makes this
                        // thread the only writer (duplicate keys in one
                        // call wait, see below).
                        let _ = cell.set(loader(key));
                    });
                });
            } else {
                for (key, cell) in &owned {
                    let _ = cell.set(loader(key));
                }
            }

            let mut state = self.state.lock().unwrap();
            for (key, cell) in &owned {
                let value = cell.get().expect("owner just set this cell").clone();
                let weight = weigher(&value);
                state.total_weight += weight;
                state.map.insert(
                    key.clone(),
                    CacheEntry {
                        value: value.clone(),
                        weight,
                        last_use: tick,
                    },
                );
                state.inflight.remove(key);
                out.insert(key.clone(), value);
            }
            while state.total_weight > self.max_weight {
                // O(n) min-scan eviction: fine at these sizes (same policy as
                // the per-worker caches this replaces).
                let oldest = state
                    .map
                    .iter()
                    .filter(|(_, e)| e.weight > 0)
                    .min_by_key(|(_, e)| e.last_use)
                    .map(|(k, _)| k.clone());
                match oldest {
                    Some(key) => {
                        if let Some(entry) = state.map.remove(&key) {
                            state.total_weight -= entry.weight;
                        }
                    }
                    None => break,
                }
            }
        }

        // A duplicate key within `keys` lands here too (second occurrence
        // sees the first's in-flight cell) — set before this loop runs, so
        // it never self-deadlocks.
        for (key, cell) in waiting {
            out.insert(key, cell.wait().clone());
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn counting_loader(calls: &AtomicUsize) -> impl Fn(&&str) -> String + Sync + '_ {
        move |key| {
            calls.fetch_add(1, Ordering::SeqCst);
            key.to_uppercase()
        }
    }

    #[test]
    fn resolve_returns_a_value_for_every_key() {
        let cache = SharedCache::new(10);
        let calls = AtomicUsize::new(0);
        let out = cache.resolve(&["a", "b"], counting_loader(&calls), |_| 1);
        assert_eq!(out["a"], "A");
        assert_eq!(out["b"], "B");
    }

    #[test]
    fn cached_key_is_not_loaded_again() {
        let cache = SharedCache::new(10);
        let calls = AtomicUsize::new(0);
        cache.resolve(&["a"], counting_loader(&calls), |_| 1);
        cache.resolve(&["a"], counting_loader(&calls), |_| 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn duplicate_keys_in_one_call_load_once() {
        let cache = SharedCache::new(10);
        let calls = AtomicUsize::new(0);
        let out = cache.resolve(&["a", "a", "a"], counting_loader(&calls), |_| 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn least_recently_used_entry_is_evicted_when_over_weight() {
        let cache = SharedCache::new(2);
        let calls = AtomicUsize::new(0);
        cache.resolve(&["a"], counting_loader(&calls), |_| 1);
        cache.resolve(&["b"], counting_loader(&calls), |_| 1);
        cache.resolve(&["a"], counting_loader(&calls), |_| 1); // touch a; b is now oldest
        cache.resolve(&["c"], counting_loader(&calls), |_| 1);
        assert_eq!(calls.load(Ordering::SeqCst), 3);

        cache.resolve(&["a"], counting_loader(&calls), |_| 1);
        assert_eq!(calls.load(Ordering::SeqCst), 3, "a must have survived");
        cache.resolve(&["b"], counting_loader(&calls), |_| 1);
        assert_eq!(calls.load(Ordering::SeqCst), 4, "b must have been evicted");
    }

    #[test]
    fn eviction_is_by_weight_not_entry_count() {
        let cache = SharedCache::new(10);
        let calls = AtomicUsize::new(0);
        let weigher = |value: &String| if value == "BIG" { 10 } else { 1 };
        cache.resolve(&["small"], counting_loader(&calls), weigher);
        cache.resolve(&["big"], |_| "BIG".to_string(), weigher);
        cache.resolve(&["small"], counting_loader(&calls), weigher);
        assert_eq!(calls.load(Ordering::SeqCst), 2, "small was evicted by big");
    }

    #[test]
    fn zero_weight_entries_are_pinned_and_never_evicted() {
        let cache = SharedCache::new(1);
        let calls = AtomicUsize::new(0);
        cache.resolve(&["missing"], counting_loader(&calls), |_| 0);
        cache.resolve(&["x"], counting_loader(&calls), |_| 1);
        cache.resolve(&["y"], counting_loader(&calls), |_| 1);
        cache.resolve(&["missing"], counting_loader(&calls), |_| 0);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn evicted_values_returned_by_the_same_call_stay_valid() {
        let cache = SharedCache::new(1);
        let calls = AtomicUsize::new(0);
        let out = cache.resolve(&["a", "b", "c"], counting_loader(&calls), |_| 1);
        assert_eq!(out.len(), 3);
        assert_eq!(out["a"], "A");
    }

    #[test]
    fn concurrent_callers_asking_for_the_same_keys_load_each_key_once() {
        const THREADS: usize = 8;
        let keys: Vec<u32> = (0..16).collect();
        let cache = SharedCache::new(1000);
        let calls = AtomicUsize::new(0);
        let start = std::sync::Barrier::new(THREADS);
        let loader = |key: &u32| {
            calls.fetch_add(1, Ordering::SeqCst);
            std::thread::yield_now();
            key * 2
        };
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..THREADS)
                .map(|_| {
                    scope.spawn(|| {
                        start.wait();
                        cache.resolve(&keys, loader, |_| 1)
                    })
                })
                .collect();
            for handle in handles {
                let out = handle.join().unwrap();
                assert!(keys.iter().all(|k| out[k] == k * 2));
            }
        });
        assert_eq!(calls.load(Ordering::SeqCst), 16);
    }

    #[test]
    fn large_batches_load_every_key_exactly_once() {
        let keys: Vec<u32> = (0..64).collect();
        let cache = SharedCache::new(1000);
        let calls = AtomicUsize::new(0);
        let out = cache.resolve(
            &keys,
            |key| {
                calls.fetch_add(1, Ordering::SeqCst);
                key * 2
            },
            |_| 1,
        );
        assert_eq!(calls.load(Ordering::SeqCst), 64);
        assert!(keys.iter().all(|k| out[k] == k * 2));
    }
}
