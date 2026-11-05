//! Which texture views the material bind group holds. A view put into a slot is only sampled
//! once the bind group has been rebuilt after it; until then the slot shows what the bind group
//! captured before, a stand-in or a slot's previous texture.

#[derive(Debug)]
pub struct ViewBinding {
    /// Counts every view change; a change is identified by the count after it.
    changes: u64,
    /// The change that last put a view into each slot, 0 for a slot never changed.
    slot_changes: Vec<u64>,
    /// The changes up to here are held by the bind group.
    bound: u64,
}

impl ViewBinding {
    pub fn new(slots: usize) -> Self {
        Self {
            changes: 0,
            slot_changes: vec![0; slots],
            bound: 0,
        }
    }

    /// A new view was put into `slot`.
    pub fn note_change(&mut self, slot: u32) {
        self.changes += 1;
        self.slot_changes[slot as usize] = self.changes;
    }

    /// The bind group was rebuilt from the views as they are now.
    pub fn mark_bound(&mut self) {
        self.bound = self.changes;
    }

    /// Whether the bind group samples the view now in `slot`.
    pub fn is_bound(&self, slot: u32) -> bool {
        self.slot_changes[slot as usize] <= self.bound
    }

    /// Whether some slot holds a view the bind group does not sample yet.
    pub fn has_unbound(&self) -> bool {
        self.changes > self.bound
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slot_never_changed_is_bound() {
        let binding = ViewBinding::new(4);
        assert!(binding.is_bound(3));
        assert!(!binding.has_unbound());
    }

    #[test]
    fn a_changed_slot_is_bound_only_after_the_next_rebuild() {
        let mut binding = ViewBinding::new(4);
        binding.note_change(2);
        assert!(!binding.is_bound(2));
        assert!(binding.is_bound(1), "other slots keep their bound view");
        assert!(binding.has_unbound());

        binding.mark_bound();

        assert!(binding.is_bound(2));
        assert!(!binding.has_unbound());
    }

    #[test]
    fn a_change_after_a_rebuild_needs_another_rebuild() {
        let mut binding = ViewBinding::new(4);
        binding.note_change(2);
        binding.mark_bound();
        binding.note_change(2);

        assert!(!binding.is_bound(2));
    }
}
