use std::collections::HashMap;
use std::hash::Hash;

/// Dense set of ids with O(1) insert and remove, so per-frame work scales with
/// what is playing, not with what exists.
#[derive(Clone, Debug)]
pub struct ActiveSet<Id: Copy + Eq + Hash> {
    dense: Vec<Id>,
    slot_by_id: HashMap<Id, usize>,
}

impl<Id: Copy + Eq + Hash> Default for ActiveSet<Id> {
    fn default() -> Self {
        Self {
            dense: Vec::new(),
            slot_by_id: HashMap::new(),
        }
    }
}

impl<Id: Copy + Eq + Hash> ActiveSet<Id> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns false if the id was already present.
    pub fn insert(&mut self, id: Id) -> bool {
        if self.slot_by_id.contains_key(&id) {
            return false;
        }
        self.slot_by_id.insert(id, self.dense.len());
        self.dense.push(id);
        true
    }

    /// Returns false if the id was not present.
    pub fn remove(&mut self, id: Id) -> bool {
        let Some(slot) = self.slot_by_id.remove(&id) else {
            return false;
        };
        self.dense.swap_remove(slot);
        if let Some(&moved) = self.dense.get(slot) {
            self.slot_by_id.insert(moved, slot);
        }
        true
    }

    pub fn contains(&self, id: Id) -> bool {
        self.slot_by_id.contains_key(&id)
    }

    pub fn len(&self) -> usize {
        self.dense.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dense.is_empty()
    }

    pub fn as_slice(&self) -> &[Id] {
        &self.dense
    }

    pub fn iter(&self) -> impl Iterator<Item = Id> + '_ {
        self.dense.iter().copied()
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use std::collections::HashSet;

    use proptest::prelude::*;

    use super::*;

    #[test]
    fn insert_reports_whether_the_id_was_new() {
        let mut set = ActiveSet::new();
        assert!(set.insert(7_u32));
        assert!(!set.insert(7));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn remove_reports_whether_the_id_was_present() {
        let mut set = ActiveSet::new();
        set.insert(1_u32);
        assert!(set.remove(1));
        assert!(!set.remove(1));
        assert!(set.is_empty());
    }

    #[test]
    fn removing_a_middle_id_keeps_the_moved_id_removable() {
        let mut set = ActiveSet::new();
        for id in [10_u32, 20, 30, 40] {
            set.insert(id);
        }
        set.remove(20);
        assert!(
            set.remove(40),
            "id moved into the freed slot must stay addressable"
        );
        assert!(set.remove(10));
        assert_eq!(set.as_slice(), &[30]);
    }

    #[test]
    fn removing_the_last_id_leaves_the_others_untouched() {
        let mut set = ActiveSet::new();
        for id in [1_u32, 2, 3] {
            set.insert(id);
        }
        set.remove(3);
        assert_eq!(set.as_slice(), &[1, 2]);
    }

    proptest! {
        #[test]
        fn behaves_like_a_hash_set(ops in prop::collection::vec((any::<bool>(), 0_u8..16), 0..200)) {
            let mut set = ActiveSet::new();
            let mut model = HashSet::new();
            for (insert, id) in ops {
                if insert {
                    prop_assert_eq!(set.insert(id), model.insert(id));
                } else {
                    prop_assert_eq!(set.remove(id), model.remove(&id));
                }
                prop_assert_eq!(set.len(), model.len());
            }
            let members: HashSet<u8> = set.iter().collect();
            prop_assert_eq!(members, model.clone());
            for id in 0..16_u8 {
                prop_assert_eq!(set.contains(id), model.contains(&id));
            }
        }
    }
}
