//! Bookkeeping for state keyed by a shared material's identity that must not outlive it.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Weak};

/// Frames between two sweeps of the caches of materials that are gone.
const SWEEP_INTERVAL_FRAMES: u32 = 120;

/// Counts frames and says when the caches are due for a sweep.
#[derive(Default)]
pub struct SweepClock {
    frames: u32,
}

impl SweepClock {
    /// Counts one frame; true once every [`SWEEP_INTERVAL_FRAMES`].
    pub fn tick(&mut self) -> bool {
        self.frames += 1;
        let due = self.frames >= SWEEP_INTERVAL_FRAMES;
        if due {
            self.frames = 0;
        }
        due
    }
}

/// A set of `(material, key)` pairs. A pair whose material was dropped no longer counts as
/// present and is removed by [`WeakSet::prune`].
pub struct WeakSet<M, K = ()> {
    entries: HashMap<(usize, K), Weak<M>>,
}

impl<M, K> Default for WeakSet<M, K> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

pub fn material_id<M>(material: &Arc<M>) -> usize {
    Arc::as_ptr(material) as usize
}

impl<M, K: Copy + Eq + Hash> WeakSet<M, K> {
    pub fn contains(&self, material: &Arc<M>, key: K) -> bool {
        self.entries
            .get(&(material_id(material), key))
            .is_some_and(|weak| weak.strong_count() > 0)
    }

    /// Adds the pair; false when it was already present.
    pub fn insert(&mut self, material: &Arc<M>, key: K) -> bool {
        if self.contains(material, key) {
            return false;
        }
        self.entries
            .insert((material_id(material), key), Arc::downgrade(material));
        true
    }

    pub fn remove(&mut self, material: &Arc<M>, key: K) {
        self.entries.remove(&(material_id(material), key));
    }

    pub fn prune(&mut self) {
        self.entries.retain(|_, weak| weak.strong_count() > 0);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pair_is_present_once_inserted_and_a_second_insert_reports_it() {
        let material = Arc::new(1);
        let mut set = WeakSet::<i32>::default();

        assert!(set.insert(&material, ()));
        assert!(!set.insert(&material, ()));
        assert!(set.contains(&material, ()));
    }

    #[test]
    fn keys_of_one_material_are_independent() {
        let material = Arc::new(1);
        let mut set = WeakSet::<i32, &str>::default();

        set.insert(&material, "a");

        assert!(!set.contains(&material, "b"));
    }

    #[test]
    fn a_dropped_material_is_absent_at_once_and_forgotten_by_the_next_prune() {
        let kept = Arc::new(1);
        let dropped = Arc::new(2);
        let mut set = WeakSet::<i32>::default();
        set.insert(&kept, ());
        set.insert(&dropped, ());

        let dropped_id = Arc::downgrade(&dropped);
        drop(dropped);

        assert_eq!(dropped_id.strong_count(), 0);
        assert_eq!(set.len(), 2);
        set.prune();
        assert_eq!(set.len(), 1);
        assert!(set.contains(&kept, ()));
    }

    #[test]
    fn the_sweep_clock_is_due_once_per_interval() {
        let mut clock = SweepClock::default();
        let due = (0..SWEEP_INTERVAL_FRAMES * 2)
            .filter(|_| clock.tick())
            .count();
        assert_eq!(due, 2);
    }
}
