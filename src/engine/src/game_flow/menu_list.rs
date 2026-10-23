//! A vertical list with one selected row and a window of visible rows, shared by the main
//! menu and the spawn picker.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuList {
    labels: Vec<&'static str>,
    visible_rows: usize,
    selected: usize,
    first_visible: usize,
}

/// What a renderer needs of a list: the labels, the selected row and the first row in view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuView<'a> {
    pub labels: &'a [&'static str],
    pub selected: usize,
    pub first_visible: usize,
}

impl MenuList {
    pub fn new(labels: Vec<&'static str>, visible_rows: usize) -> Self {
        assert!(!labels.is_empty(), "a menu list needs at least one row");
        assert!(visible_rows > 0, "a menu list shows at least one row");
        Self {
            labels,
            visible_rows,
            selected: 0,
            first_visible: 0,
        }
    }

    fn len(&self) -> usize {
        self.labels.len()
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn view(&self) -> MenuView<'_> {
        MenuView {
            labels: &self.labels,
            selected: self.selected,
            first_visible: self.first_visible,
        }
    }

    pub fn reset(&mut self) {
        self.selected = 0;
        self.first_visible = 0;
    }

    /// Moves the selection one row up, wrapping from the first row to the last.
    pub fn select_previous(&mut self) {
        self.selected = self.selected.checked_sub(1).unwrap_or(self.len() - 1);
        self.scroll_to_selection();
    }

    /// Moves the selection one row down, wrapping from the last row to the first.
    pub fn select_next(&mut self) {
        self.selected = (self.selected + 1) % self.len();
        self.scroll_to_selection();
    }

    /// Selects `row` when it exists; returns whether it did.
    pub fn select(&mut self, row: usize) -> bool {
        let exists = row < self.len();
        if exists {
            self.selected = row;
            self.scroll_to_selection();
        }
        exists
    }

    /// Scrolls the window by `rows` (negative scrolls towards the first row), clamped to the
    /// list, and keeps the selection inside the window.
    pub fn scroll(&mut self, rows: i32) {
        let max_first = self.len().saturating_sub(self.visible_rows);
        let target = i64::try_from(self.first_visible).unwrap_or(i64::MAX) + i64::from(rows);
        self.first_visible = usize::try_from(target.max(0)).unwrap_or(0).min(max_first);
        let last_visible = self.first_visible + self.visible_rows - 1;
        self.selected = self.selected.clamp(self.first_visible, last_visible);
    }

    fn scroll_to_selection(&mut self) {
        if self.selected < self.first_visible {
            self.first_visible = self.selected;
        } else if self.selected >= self.first_visible + self.visible_rows {
            self.first_visible = self.selected + 1 - self.visible_rows;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(rows: usize, visible: usize) -> MenuList {
        MenuList::new(vec!["ROW"; rows], visible)
    }

    #[test]
    fn selecting_next_from_the_last_row_wraps_to_the_first() {
        let mut list = list(3, 9);
        list.select(2);
        list.select_next();
        assert_eq!(list.selected(), 0);
    }

    #[test]
    fn selecting_previous_from_the_first_row_wraps_to_the_last() {
        let mut list = list(3, 9);
        list.select_previous();
        assert_eq!(list.selected(), 2);
    }

    #[test]
    fn the_window_follows_the_selection_down_and_up() {
        let mut list = list(14, 9);
        for _ in 0..9 {
            list.select_next();
        }
        assert_eq!((list.selected(), list.view().first_visible), (9, 1));
        for _ in 0..9 {
            list.select_previous();
        }
        assert_eq!((list.selected(), list.view().first_visible), (0, 0));
    }

    #[test]
    fn wrapping_up_from_the_first_row_scrolls_to_the_end_of_the_list() {
        let mut list = list(14, 9);
        list.select_previous();
        assert_eq!((list.selected(), list.view().first_visible), (13, 5));
    }

    #[test]
    fn a_list_shorter_than_the_window_never_scrolls() {
        let mut list = list(2, 9);
        list.scroll(5);
        list.select_next();
        assert_eq!((list.selected(), list.view().first_visible), (1, 0));
    }

    #[test]
    fn scrolling_is_clamped_at_both_ends() {
        let mut list = list(14, 9);
        list.scroll(100);
        assert_eq!(list.view().first_visible, 5);
        list.scroll(-100);
        assert_eq!(list.view().first_visible, 0);
    }

    #[test]
    fn scrolling_keeps_the_selection_inside_the_window() {
        let mut list = list(14, 9);
        list.scroll(3);
        assert_eq!((list.selected(), list.view().first_visible), (3, 3));
        list.select(11);
        list.scroll(-5);
        assert_eq!((list.selected(), list.view().first_visible), (8, 0));
    }

    #[test]
    fn selecting_a_missing_row_changes_nothing() {
        let mut list = list(3, 9);
        list.select(1);
        assert!(!list.select(3));
        assert_eq!(list.selected(), 1);
    }

    #[test]
    fn reset_returns_to_the_first_row_and_window() {
        let mut list = list(14, 9);
        list.select(12);
        list.reset();
        assert_eq!((list.selected(), list.view().first_visible), (0, 0));
    }
}
