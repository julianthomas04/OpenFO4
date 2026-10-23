use std::collections::HashMap;

use super::backend::StreamError;

const DISCARDED_KEY: &str = "discarded stream errors";

/// Turns the backend's stream errors into log lines without repeating an error kind every
/// frame: a kind is reported the first time and then each time its count reaches a power of
/// two, so a persistent fault costs a logarithmic number of lines. Counting by kind keeps the
/// map as small as the set of kinds, whatever the message text.
#[derive(Debug, Default)]
pub struct BackendErrorReporter {
    occurrences: HashMap<String, u64>,
    discarded_reported: u64,
}

impl BackendErrorReporter {
    /// The lines to log for the errors popped this frame and the backend's running count of
    /// discarded errors.
    pub fn lines(
        &mut self,
        errors: impl IntoIterator<Item = StreamError>,
        discarded_total: u64,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        for error in errors {
            if let Some(count) = self.count_if_due(&error.kind) {
                lines.push(describe("audio backend error", &error.message, count));
            }
        }
        if discarded_total > self.discarded_reported {
            self.discarded_reported = discarded_total;
            if let Some(count) = self.count_if_due(DISCARDED_KEY) {
                lines.push(describe(
                    "audio backend dropped stream errors that were not polled in time",
                    &format!("{discarded_total} dropped in total"),
                    count,
                ));
            }
        }
        lines
    }

    fn count_if_due(&mut self, key: &str) -> Option<u64> {
        let count = self.occurrences.entry(key.to_string()).or_default();
        *count += 1;
        count.is_power_of_two().then_some(*count)
    }
}

fn describe(what: &str, detail: &str, count: u64) -> String {
    match count {
        1 => format!("{what}: {detail}"),
        _ => format!("{what}: {detail} (occurred {count} times)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(kind: &str, message: &str) -> StreamError {
        StreamError {
            kind: kind.to_string(),
            message: message.to_string(),
        }
    }

    fn errors(messages: &[&str]) -> Vec<StreamError> {
        messages
            .iter()
            .map(|message| error(message, message))
            .collect()
    }

    #[test]
    fn a_new_error_is_reported_once() {
        let mut reporter = BackendErrorReporter::default();
        assert_eq!(
            reporter.lines(errors(&["device unplugged"]), 0),
            ["audio backend error: device unplugged"]
        );
    }

    #[test]
    fn a_repeating_error_is_reported_at_growing_intervals() {
        let mut reporter = BackendErrorReporter::default();
        let reported: Vec<usize> = (1..=9)
            .filter(|_| !reporter.lines(errors(&["x"]), 0).is_empty())
            .collect();
        assert_eq!(reported, [1, 2, 4, 8]);
    }

    #[test]
    fn the_repeat_count_is_part_of_the_line() {
        let mut reporter = BackendErrorReporter::default();
        reporter.lines(errors(&["x"]), 0);
        assert_eq!(
            reporter.lines(errors(&["x"]), 0),
            ["audio backend error: x (occurred 2 times)"]
        );
    }

    #[test]
    fn distinct_errors_are_counted_apart() {
        let mut reporter = BackendErrorReporter::default();
        assert_eq!(reporter.lines(errors(&["a", "b", "a"]), 0).len(), 3);
    }

    #[test]
    fn errors_of_one_kind_with_varying_messages_are_counted_together() {
        let mut reporter = BackendErrorReporter::default();
        reporter.lines([error("busy", "device 1 busy")], 0);
        assert_eq!(
            reporter.lines([error("busy", "device 2 busy")], 0),
            ["audio backend error: device 2 busy (occurred 2 times)"]
        );
        assert_eq!(reporter.occurrences.len(), 1);
    }

    #[test]
    fn discarded_errors_are_reported_when_the_total_grows() {
        let mut reporter = BackendErrorReporter::default();
        assert_eq!(reporter.lines([], 0), [] as [String; 0]);
        let lines = reporter.lines([], 7);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("7 dropped in total"), "{lines:?}");
        assert_eq!(reporter.lines([], 7), [] as [String; 0]);
        assert_ne!(reporter.lines([], 9), [] as [String; 0]);
    }
}
