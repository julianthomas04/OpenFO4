//! Whether a load is the session's first or a transition between places, which decides how the
//! origin of a transition is known and what the load's report is called.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::rendering) enum LoadOrigin {
    /// The load that brings the player into the world after the menu or a direct start.
    Initial,
    /// Every later switch of cell or worldspace.
    Transition,
}

/// The loads of a session so far.
#[derive(Debug)]
pub(in crate::rendering) struct LoadHistory {
    began: bool,
    current: LoadOrigin,
}

impl Default for LoadHistory {
    fn default() -> Self {
        Self {
            began: false,
            current: LoadOrigin::Initial,
        }
    }
}

impl LoadHistory {
    /// Whether the next load is the first of the session.
    pub fn next_is_initial(&self) -> bool {
        !self.began
    }

    /// Records that a load begins and says which kind it is.
    pub fn begin(&mut self) -> LoadOrigin {
        self.current = if self.began {
            LoadOrigin::Transition
        } else {
            LoadOrigin::Initial
        };
        self.began = true;
        self.current
    }

    /// The kind of the load that began last.
    pub fn current(&self) -> LoadOrigin {
        self.current
    }
}

/// The toast's title for a load that finished.
pub(in crate::rendering) fn spawn_report_title(origin: LoadOrigin) -> &'static str {
    match origin {
        LoadOrigin::Initial => "Initial spawn",
        LoadOrigin::Transition => "Cell load",
    }
}

/// Why a switch of place may not start now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::rendering) enum Refusal {
    LoadRunning,
    PreviewOpen,
}

impl Refusal {
    fn reason(self) -> &'static str {
        match self {
            Self::LoadRunning => "a load is already running",
            Self::PreviewOpen => "the loading screen preview is open",
        }
    }
}

/// A request that arrives while a switch of place may not start is dropped, with a warning naming
/// why, rather than kept for later, when it would be stale.
pub(in crate::rendering) fn take_if_accepted<T>(
    slot: &mut Option<T>,
    refusal: Option<Refusal>,
    request: &str,
) -> Option<T> {
    let taken = slot.take()?;
    match refusal {
        None => Some(taken),
        Some(refusal) => {
            tracing::warn!("{request} dropped: {}", refusal.reason());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_load_is_initial_and_every_later_one_a_transition() {
        let mut history = LoadHistory::default();
        assert!(history.next_is_initial());
        assert_eq!(history.begin(), LoadOrigin::Initial);
        assert_eq!(history.current(), LoadOrigin::Initial);
        assert!(!history.next_is_initial());
        assert_eq!(history.begin(), LoadOrigin::Transition);
        assert_eq!(history.begin(), LoadOrigin::Transition);
        assert_eq!(history.current(), LoadOrigin::Transition);
    }

    #[test]
    fn the_report_names_the_kind_of_load() {
        assert_eq!(spawn_report_title(LoadOrigin::Initial), "Initial spawn");
        assert_eq!(spawn_report_title(LoadOrigin::Transition), "Cell load");
    }

    #[test]
    fn an_accepted_request_is_taken() {
        let mut slot = Some(3);
        assert_eq!(take_if_accepted(&mut slot, None, "test request"), Some(3));
        assert_eq!(slot, None);
    }

    #[test]
    fn a_request_during_a_load_is_dropped_and_not_kept() {
        let mut slot = Some(3);
        assert_eq!(take_if_accepted(&mut slot, Some(Refusal::LoadRunning), "test request"),
            None
        );
        assert_eq!(slot, None, "it must not fire after the reveal");
    }

    #[test]
    fn each_refusal_names_its_own_reason() {
        assert_eq!(Refusal::LoadRunning.reason(), "a load is already running");
        assert_eq!(
            Refusal::PreviewOpen.reason(),
            "the loading screen preview is open"
        );
    }

    #[test]
    fn no_request_stays_no_request() {
        let mut slot: Option<i32> = None;
        assert_eq!(take_if_accepted(&mut slot, None, "test request"), None);
        assert_eq!(
            take_if_accepted(&mut slot, Some(Refusal::PreviewOpen), "test request"),
            None
        );
    }
}
