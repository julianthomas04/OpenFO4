//! What the grass stage holds per streamed cell, keyed by the cell's residency
//! id: unique per integration, so a cell reloaded on the same key while its
//! predecessor is still queued for release never collides with it.

use std::collections::HashMap;

pub type CellKey = u64;

pub struct CellBook<T> {
    cells: HashMap<CellKey, Vec<T>>,
}

impl<T> Default for CellBook<T> {
    fn default() -> Self {
        Self {
            cells: HashMap::new(),
        }
    }
}

impl<T> CellBook<T> {
    /// Records `entries` for `key`, returning what the key held before so the
    /// caller can release it. Empty `entries` record nothing.
    pub fn insert(&mut self, key: CellKey, entries: Vec<T>) -> Option<Vec<T>> {
        if entries.is_empty() {
            return self.cells.remove(&key);
        }
        self.cells.insert(key, entries)
    }

    pub fn remove(&mut self, key: CellKey) -> Option<Vec<T>> {
        self.cells.remove(&key)
    }

    pub fn clear(&mut self) {
        self.cells.clear();
    }

    pub fn entries(&self) -> impl Iterator<Item = &T> {
        self.cells.values().flatten()
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

    #[test]
    fn inserted_entries_are_listed_and_counted() {
        let mut book = CellBook::default();
        assert!(book.insert(0, vec![1, 2]).is_none());
        assert!(book.insert(1, vec![3]).is_none());
        assert_eq!(sorted(&book), vec![1, 2, 3]);
        assert_eq!(book.cell_count(), 2);
        assert_eq!(book.entry_count(), 3);
    }

    #[test]
    fn removing_a_key_returns_its_entries_and_leaves_the_others() {
        let mut book = CellBook::default();
        book.insert(0, vec![1, 2]);
        book.insert(1, vec![3]);
        assert_eq!(book.remove(0), Some(vec![1, 2]));
        assert_eq!(sorted(&book), vec![3]);
        assert_eq!(book.remove(0), None);
    }

    #[test]
    fn reinserting_a_key_returns_the_replaced_entries() {
        let mut book = CellBook::default();
        book.insert(2, vec![7]);
        assert_eq!(book.insert(2, vec![8, 9]), Some(vec![7]));
        assert_eq!(sorted(&book), vec![8, 9]);
    }

    #[test]
    fn inserting_nothing_records_no_cell_and_drops_a_stale_one() {
        let mut book = CellBook::default();
        assert!(book.insert(0, Vec::new()).is_none());
        assert_eq!(book.cell_count(), 0);
        book.insert(0, vec![5]);
        assert_eq!(book.insert(0, Vec::new()), Some(vec![5]));
        assert_eq!(book.cell_count(), 0);
    }

    #[test]
    fn removing_a_key_that_holds_nothing_reports_nothing() {
        let mut book: CellBook<u32> = CellBook::default();
        book.insert(4, Vec::new());
        assert_eq!(book.remove(4), None);
        assert_eq!(book.remove(99), None);
    }

    #[test]
    fn a_cell_reloaded_under_a_new_key_survives_its_predecessors_release() {
        let (retiring, reloaded) = (10, 11);
        let mut book = CellBook::default();
        book.insert(retiring, vec![1, 2]);
        book.insert(reloaded, vec![3]);
        assert_eq!(book.remove(retiring), Some(vec![1, 2]));
        assert_eq!(sorted(&book), vec![3]);
        assert_eq!(book.cell_count(), 1);
    }

    #[test]
    fn a_reload_that_places_no_grass_leaves_the_retiring_cells_entries_alone() {
        let (retiring, reloaded) = (10, 11);
        let mut book = CellBook::default();
        book.insert(retiring, vec![1]);
        assert!(book.insert(reloaded, Vec::new()).is_none());
        assert_eq!(sorted(&book), vec![1]);
        assert_eq!(book.remove(retiring), Some(vec![1]));
        assert_eq!(book.remove(reloaded), None);
    }

    #[test]
    fn clear_drops_every_cell() {
        let mut book = CellBook::default();
        book.insert(0, vec![1]);
        book.insert(1, vec![2]);
        book.clear();
        assert_eq!(book.entry_count(), 0);
    }
}
