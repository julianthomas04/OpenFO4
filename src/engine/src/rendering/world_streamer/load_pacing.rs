//! Frame pacing of the cell loader: how long one frame may spend integrating
//! cells in each [`LoadMode`], and the worst-frame meter that the
//! `SPAWN_COMPLETE` line reports.

use std::time::{Duration, Instant};

use super::LoadMode;

/// Streaming-mode wall-clock cap on integrating further cells within one frame.
const STREAMING_INTEGRATE_TIME_BUDGET: Duration = Duration::from_millis(4);

/// Loading-mode wall-clock cap on integrating further cells within one frame.
/// Wider than streaming's because behind a loading screen throughput matters,
/// but still bounded so the window keeps pumping events on a cold cache where
/// every cell brings fresh textures and meshes.
const LOADING_INTEGRATE_TIME_BUDGET: Duration = Duration::from_millis(12);

/// What one frame has integrated so far.
pub(super) struct IntegrationProgress {
    pub(super) integrated: usize,
    pub(super) cell_cap: usize,
    pub(super) bytes_exhausted: bool,
    pub(super) elapsed: Duration,
}

/// Whether the frame must not start another cell: the cell cap, the upload
/// byte budget or the mode's time budget is spent. The first cell of a frame
/// is always integrated, so a budget slows the load but cannot stall it.
pub(super) fn integration_must_stop(mode: LoadMode, progress: &IntegrationProgress) -> bool {
    progress.integrated >= 1
        && (progress.integrated >= progress.cell_cap
            || progress.bytes_exhausted
            || integrate_time_spent(mode, progress.elapsed))
}

fn integrate_time_spent(mode: LoadMode, elapsed: Duration) -> bool {
    let budget = match mode {
        LoadMode::Streaming => STREAMING_INTEGRATE_TIME_BUDGET,
        LoadMode::Loading => LOADING_INTEGRATE_TIME_BUDGET,
    };
    elapsed >= budget
}

/// Longest frame-to-frame gap seen while loading, since the last
/// [`reset`](Self::reset). A gap is the time between two consecutive streamer
/// updates, so it covers the whole frame, not just the streamer's share. The
/// frame that completes the load is not part of the figure: its gap is only
/// closed by the next update, by which time the mode has left Loading.
pub(super) struct LoadFrameMeter {
    previous_update: Option<Instant>,
    worst: Duration,
}

impl LoadFrameMeter {
    pub(super) fn new() -> Self {
        Self {
            previous_update: None,
            worst: Duration::ZERO,
        }
    }

    /// Forgets the worst gap of the previous load; the last update time stays,
    /// so the frame that starts the load is measured too.
    pub(super) fn reset(&mut self) {
        self.worst = Duration::ZERO;
    }

    /// Forgets the last update time, for frames on which the streamer update
    /// is skipped (menus): the next observation then has no gap to measure,
    /// so the idle time is not mistaken for a slow loading frame.
    pub(super) fn suspend(&mut self) {
        self.previous_update = None;
    }

    pub(super) fn observe(&mut self, now: Instant, mode: LoadMode) {
        if mode == LoadMode::Loading
            && let Some(previous) = self.previous_update
        {
            self.worst = self.worst.max(now.saturating_duration_since(previous));
        }
        self.previous_update = Some(now);
    }

    pub(super) fn worst_ms(&self) -> f32 {
        self.worst.as_secs_f32() * 1000.0
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the meter reports exactly zero before any gap is seen"
)]
mod tests {
    use super::*;

    const FRAME: Duration = Duration::from_millis(16);

    #[test]
    fn streaming_stops_integrating_at_its_own_budget() {
        assert!(!integrate_time_spent(
            LoadMode::Streaming,
            STREAMING_INTEGRATE_TIME_BUDGET.saturating_sub(Duration::from_micros(1))
        ));
        assert!(integrate_time_spent(
            LoadMode::Streaming,
            STREAMING_INTEGRATE_TIME_BUDGET
        ));
    }

    #[test]
    fn loading_keeps_integrating_where_streaming_would_stop_and_stops_at_its_budget() {
        assert!(!integrate_time_spent(
            LoadMode::Loading,
            STREAMING_INTEGRATE_TIME_BUDGET
        ));
        assert!(!integrate_time_spent(
            LoadMode::Loading,
            LOADING_INTEGRATE_TIME_BUDGET.saturating_sub(Duration::from_micros(1))
        ));
        assert!(integrate_time_spent(
            LoadMode::Loading,
            LOADING_INTEGRATE_TIME_BUDGET
        ));
    }

    fn progress(integrated: usize, elapsed: Duration) -> IntegrationProgress {
        IntegrationProgress {
            integrated,
            cell_cap: 32,
            bytes_exhausted: false,
            elapsed,
        }
    }

    #[test]
    fn a_frame_integrates_its_first_cell_whatever_the_budgets_say() {
        let way_over = Duration::from_secs(10);
        for mode in [LoadMode::Streaming, LoadMode::Loading] {
            let exhausted = IntegrationProgress {
                bytes_exhausted: true,
                cell_cap: 0,
                ..progress(0, way_over)
            };
            assert!(!integration_must_stop(mode, &exhausted));
            assert!(integration_must_stop(mode, &progress(1, way_over)));
        }
    }

    #[test]
    fn integration_stops_at_the_cell_cap_or_when_the_upload_bytes_are_spent() {
        let early = Duration::ZERO;
        assert!(!integration_must_stop(
            LoadMode::Loading,
            &progress(31, early)
        ));
        assert!(integration_must_stop(
            LoadMode::Loading,
            &progress(32, early)
        ));
        let bytes_spent = IntegrationProgress {
            bytes_exhausted: true,
            ..progress(1, early)
        };
        assert!(integration_must_stop(LoadMode::Loading, &bytes_spent));
    }

    #[test]
    fn loading_budget_is_wider_than_streaming_budget() {
        assert!(LOADING_INTEGRATE_TIME_BUDGET > STREAMING_INTEGRATE_TIME_BUDGET);
    }

    fn after(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }

    #[test]
    fn the_worst_loading_gap_is_the_largest_distance_between_consecutive_updates() {
        let start = Instant::now();
        let mut meter = LoadFrameMeter::new();
        meter.observe(start, LoadMode::Loading);
        meter.observe(start + FRAME, LoadMode::Loading);
        meter.observe(after(start, 16 + 250), LoadMode::Loading);
        meter.observe(after(start, 16 + 250 + 20), LoadMode::Loading);
        assert!(
            (meter.worst_ms() - 250.0).abs() < 0.01,
            "{}",
            meter.worst_ms()
        );
    }

    #[test]
    fn gaps_that_end_in_streaming_mode_are_not_counted() {
        let start = Instant::now();
        let mut meter = LoadFrameMeter::new();
        meter.observe(start, LoadMode::Loading);
        meter.observe(after(start, 500), LoadMode::Streaming);
        assert_eq!(meter.worst_ms(), 0.0);
    }

    #[test]
    fn the_first_observation_has_no_gap_to_measure() {
        let mut meter = LoadFrameMeter::new();
        meter.observe(Instant::now(), LoadMode::Loading);
        assert_eq!(meter.worst_ms(), 0.0);
    }

    #[test]
    fn a_suspension_keeps_the_idle_time_out_of_the_worst_gap() {
        let start = Instant::now();
        let mut meter = LoadFrameMeter::new();
        meter.observe(start, LoadMode::Loading);
        meter.suspend();
        meter.observe(after(start, 60_000), LoadMode::Loading);
        assert_eq!(meter.worst_ms(), 0.0);
        meter.observe(after(start, 60_000 + 30), LoadMode::Loading);
        assert!(
            (meter.worst_ms() - 30.0).abs() < 0.01,
            "{}",
            meter.worst_ms()
        );
    }

    #[test]
    fn reset_clears_the_worst_gap_but_keeps_measuring_from_the_last_update() {
        let start = Instant::now();
        let mut meter = LoadFrameMeter::new();
        meter.observe(start, LoadMode::Loading);
        meter.observe(after(start, 400), LoadMode::Loading);
        meter.reset();
        assert_eq!(meter.worst_ms(), 0.0);
        meter.observe(after(start, 400 + 90), LoadMode::Loading);
        assert!(
            (meter.worst_ms() - 90.0).abs() < 0.01,
            "{}",
            meter.worst_ms()
        );
    }
}
