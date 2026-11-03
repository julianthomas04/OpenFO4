//! What the grass stage holds per streamed cell, keyed by the cell's residency
//! id: unique per integration, so a cell reloaded on the same key while its
//! predecessor is still queued for release never collides with it. A cell's
//! entries can be held hidden, while the streamer waits for the cell's
//! textures, and revealed later.

use std::collections::{HashMap, HashSet};

pub type CellKey = u64;

pub struct CellBook<T> {
    cells: HashMap<CellKey, Vec<T>>,
    hidden: HashSet<CellKey>,
}

impl<T> Default for CellBook<T> {
    fn default() -> Self {
        Self {
            cells: HashMap::new(),
            hidden: HashSet::new(),
        }
    }
}

impl<T> CellBook<T> {
    /// Records `entries` for `key`, hidden or not, returning what the key held
    /// before so the caller can release it. Empty `entries` record nothing.
    pub fn insert(&mut self, key: CellKey, entries: Vec<T>, hidden: bool) -> Option<Vec<T>> {
        if entries.is_empty() {
            return self.remove(key);
        }
        if hidden {
            self.hidden.insert(key);
        } else {
            self.hidden.remove(&key);
        }
        self.cells.insert(key, entries)
    }

    pub fn remove(&mut self, key: CellKey) -> Option<Vec<T>> {
        self.hidden.remove(&key);
        self.cells.remove(&key)
    }

    /// Shows the entries of a cell inserted hidden.
    pub fn reveal(&mut self, key: CellKey) {
        self.hidden.remove(&key);
    }

    pub fn clear(&mut self) {
        self.cells.clear();
        self.hidden.clear();
    }

    /// Every entry, hidden or not.
    pub fn entries(&self) -> impl Iterator<Item = &T> {
        self.cells.values().flatten()
    }

    /// The entries of the cells that are not hidden.
    pub fn visible_entries(&self) -> impl Iterator<Item = &T> {
        self.cells
            .iter()
            .filter(|(key, _)| !self.hidden.contains(key))
            .flat_map(|(_, entries)| entries)
    }

    /// The entries recorded for `key`.
    pub fn cell(&self, key: CellKey) -> &[T] {
        self.cells.get(&key).map_or(&[], Vec::as_slice)
    }

    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    #[cfg(test)]
    pub fn entry_count(&self) -> usize {
        self.cells.values().map(Vec::len).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(book: &CellBook<u32>) -> Vec<u32> {
        let mut all: Vec<u32> = book.entries().copied().collect();
        all.sort_unstable();
        all
    }

    fn sorted_visible(book: &CellBook<u32>) -> Vec<u32> {
        let mut all: Vec<u32> = book.visible_entries().copied().collect();
        all.sort_unstable();
        all
    }

    #[test]
    fn a_hidden_cell_is_held_but_not_visible_until_revealed() {
        let mut book = CellBook::default();
        book.insert(0, vec![1], false);
        book.insert(1, vec![2, 3], true);

        assert_eq!(sorted(&book), vec![1, 2, 3]);
        assert_eq!(sorted_visible(&book), vec![1]);
        assert_eq!(book.cell(1), &[2, 3]);

        book.reveal(1);

        assert_eq!(sorted_visible(&book), vec![1, 2, 3]);
    }

    #[test]
    fn a_removed_hidden_key_reused_later_is_visible() {
        let mut book = CellBook::default();
        book.insert(4, vec![1], true);
        book.remove(4);
        book.insert(4, vec![2], false);

        assert_eq!(sorted_visible(&book), vec![2]);
        assert_eq!(book.cell(9), &[] as &[u32]);
    }

    #[test]
    fn inserted_entries_are_listed_and_counted() {
        let mut book = CellBook::default();
        assert!(book.insert(0, vec![1, 2], false).is_none());
        assert!(book.insert(1, vec![3], false).is_none());
        assert_eq!(sorted(&book), vec![1, 2, 3]);
        assert_eq!(book.cell_count(), 2);
        assert_eq!(book.entry_count(), 3);
    }

    #[test]
    fn removing_a_key_returns_its_entries_and_leaves_the_others() {
        let mut book = CellBook::default();
        book.insert(0, vec![1, 2], false);
        book.insert(1, vec![3], false);
        assert_eq!(book.remove(0), Some(vec![1, 2]));
        assert_eq!(sorted(&book), vec![3]);
        assert_eq!(book.remove(0), None);
    }

    #[test]
    fn reinserting_a_key_returns_the_replaced_entries() {
        let mut book = CellBook::default();
        book.insert(2, vec![7], false);
        assert_eq!(book.insert(2, vec![8, 9], false), Some(vec![7]));
        assert_eq!(sorted(&book), vec![8, 9]);
    }

    #[test]
    fn inserting_nothing_records_no_cell_and_drops_a_stale_one() {
        let mut book = CellBook::default();
        assert!(book.insert(0, Vec::new(), false).is_none());
        assert_eq!(book.cell_count(), 0);
        book.insert(0, vec![5], false);
        assert_eq!(book.insert(0, Vec::new(), false), Some(vec![5]));
        assert_eq!(book.cell_count(), 0);
    }

    #[test]
    fn removing_a_key_that_holds_nothing_reports_nothing() {
        let mut book: CellBook<u32> = CellBook::default();
        book.insert(4, Vec::new(), false);
        assert_eq!(book.remove(4), None);
        assert_eq!(book.remove(99), None);
    }

    #[test]
    fn a_cell_reloaded_under_a_new_key_survives_its_predecessors_release() {
        let (retiring, reloaded) = (10, 11);
        let mut book = CellBook::default();
        book.insert(retiring, vec![1, 2], false);
        book.insert(reloaded, vec![3], false);
        assert_eq!(book.remove(retiring), Some(vec![1, 2]));
        assert_eq!(sorted(&book), vec![3]);
        assert_eq!(book.cell_count(), 1);
    }

    #[test]
    fn a_reload_that_places_no_grass_leaves_the_retiring_cells_entries_alone() {
        let (retiring, reloaded) = (10, 11);
        let mut book = CellBook::default();
        book.insert(retiring, vec![1], false);
        assert!(book.insert(reloaded, Vec::new(), false).is_none());
        assert_eq!(sorted(&book), vec![1]);
        assert_eq!(book.remove(retiring), Some(vec![1]));
        assert_eq!(book.remove(reloaded), None);
    }

    #[test]
    fn clear_drops_every_cell() {
        let mut book = CellBook::default();
        book.insert(0, vec![1], false);
        book.insert(1, vec![2], false);
        book.clear();
        assert_eq!(book.entry_count(), 0);
    }
}
