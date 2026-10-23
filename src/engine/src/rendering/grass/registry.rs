//! Reference-counted store of shared grass resources (models, grass types): a
//! resource is built on its first acquisition and handed back for teardown on
//! its last release.

use std::collections::HashMap;
use std::hash::Hash;

struct Entry<V> {
    value: V,
    refs: usize,
}

pub struct RefRegistry<K, V> {
    entries: HashMap<K, Entry<V>>,
}

impl<K, V> Default for RefRegistry<K, V> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl<K: Eq + Hash, V> RefRegistry<K, V> {
    /// Takes a reference to `key`, building the value with `build` when the key
    /// is not held yet. `None` from `build` means the value cannot exist and
    /// takes no reference.
    pub fn acquire(&mut self, key: K, build: impl FnOnce() -> Option<V>) -> Option<&V> {
        use std::collections::hash_map::Entry::{Occupied, Vacant};
        let entry = match self.entries.entry(key) {
            Occupied(held) => held.into_mut(),
            Vacant(slot) => slot.insert(Entry {
                value: build()?,
                refs: 0,
            }),
        };
        entry.refs += 1;
        Some(&entry.value)
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.entries.get(key).map(|entry| &entry.value)
    }

    /// Drops one reference to `key`; returns the value when that was the last one.
    pub fn release(&mut self, key: &K) -> Option<V> {
        let entry = self.entries.get_mut(key)?;
        entry.refs -= 1;
        if entry.refs > 0 {
            return None;
        }
        self.entries.remove(key).map(|entry| entry.value)
    }

    pub fn drain(&mut self) -> impl Iterator<Item = V> + '_ {
        self.entries.drain().map(|(_, entry)| entry.value)
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.entries.values().map(|entry| &entry.value)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_value_is_built_once_and_shared() {
        let mut registry = RefRegistry::default();
        let mut builds = 0;
        for _ in 0..3 {
            registry.acquire("a", || {
                builds += 1;
                Some(7)
            });
        }
        assert_eq!(builds, 1);
        assert_eq!(registry.get(&"a"), Some(&7));
    }

    #[test]
    fn the_value_is_handed_back_on_the_last_release_only() {
        let mut registry = RefRegistry::default();
        registry.acquire("a", || Some(7));
        registry.acquire("a", || Some(7));
        assert_eq!(registry.release(&"a"), None);
        assert_eq!(registry.get(&"a"), Some(&7));
        assert_eq!(registry.release(&"a"), Some(7));
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn a_value_that_cannot_be_built_takes_no_reference() {
        let mut registry: RefRegistry<&str, i32> = RefRegistry::default();
        assert!(registry.acquire("a", || None).is_none());
        assert_eq!(registry.len(), 0);
        assert_eq!(registry.release(&"a"), None);
    }

    #[test]
    fn a_released_key_is_rebuilt_on_the_next_acquisition() {
        let mut registry = RefRegistry::default();
        registry.acquire("a", || Some(1));
        registry.release(&"a");
        registry.acquire("a", || Some(2));
        assert_eq!(registry.get(&"a"), Some(&2));
    }

    #[test]
    fn drain_hands_back_every_value_regardless_of_references() {
        let mut registry = RefRegistry::default();
        registry.acquire("a", || Some(1));
        registry.acquire("a", || Some(1));
        registry.acquire("b", || Some(2));
        let mut drained: Vec<i32> = registry.drain().collect();
        drained.sort_unstable();
        assert_eq!(drained, vec![1, 2]);
        assert_eq!(registry.len(), 0);
    }
}
