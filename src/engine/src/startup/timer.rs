use std::time::{Duration, Instant};

use super::{CancelToken, StartupError};

/// Wall time of each startup step, logged as one `STARTUP_TIMINGS` line.
pub struct StartupTimer {
    started: Instant,
    last_mark: Instant,
    steps: Vec<(&'static str, Duration)>,
    cancel: CancelToken,
}

impl StartupTimer {
    pub fn start(cancel: CancelToken) -> Self {
        let now = Instant::now();
        Self {
            started: now,
            last_mark: now,
            steps: Vec::new(),
            cancel,
        }
    }

    /// Ends the step that has run since the previous mark; fails once startup was cancelled.
    pub fn mark(&mut self, step: &'static str) -> Result<(), StartupError> {
        let now = Instant::now();
        self.steps.push((step, now - self.last_mark));
        self.last_mark = now;
        self.cancel.check()
    }

    pub fn summary(&self) -> String {
        let steps = self
            .steps
            .iter()
            .map(|(step, duration)| format!("{step}={}", duration.as_millis()));
        let total = format!("total={}", self.started.elapsed().as_millis());
        std::iter::once("STARTUP_TIMINGS".to_owned())
            .chain(steps)
            .chain([total])
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn log_summary(&self) {
        tracing::info!("{}", self.summary());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_lists_the_steps_in_order_and_ends_with_the_total() {
        let mut timer = StartupTimer::start(CancelToken::default());
        timer.mark("shaders").unwrap();
        timer.mark("settings").unwrap();
        let summary = timer.summary();
        let words: Vec<_> = summary.split(' ').collect();
        assert_eq!(words[0], "STARTUP_TIMINGS");
        assert!(words[1].starts_with("shaders="));
        assert!(words[2].starts_with("settings="));
        assert!(words[3].starts_with("total="));
        assert_eq!(words.len(), 4);
    }

    #[test]
    fn a_timer_without_steps_still_reports_the_total() {
        assert!(
            StartupTimer::start(CancelToken::default())
                .summary()
                .starts_with("STARTUP_TIMINGS total=")
        );
    }

    #[test]
    fn marking_a_step_after_cancellation_stops_startup() {
        let cancel = CancelToken::default();
        let mut timer = StartupTimer::start(cancel.clone());
        assert!(timer.mark("shaders").is_ok());
        cancel.cancel();
        assert!(matches!(
            timer.mark("settings"),
            Err(StartupError::Cancelled)
        ));
    }
}
