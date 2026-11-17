//! `OPENFO4_WALK_IN_TRACE=1`: a diagnostic trace of how far the player is from the walk-in
//! entrance that decides the HUD, else the nearest one, to find out why it does or does not show
//! or start. What to log and when lives here; the renderer owns the logging.

use super::walk_in::EntranceProbe;

/// Seconds between periodic trace lines.
const TRACE_INTERVAL_SECONDS: f32 = 0.5;

#[derive(Debug)]
pub(super) struct WalkInTrace {
    since_last_line: f32,
    last_inside: Option<(String, bool)>,
}

impl WalkInTrace {
    /// Enabled by `OPENFO4_WALK_IN_TRACE=1`; anything else leaves the trace off.
    pub(super) fn from_env() -> Option<Self> {
        Self::from_setting(std::env::var("OPENFO4_WALK_IN_TRACE").ok().as_deref())
    }

    fn from_setting(setting: Option<&str>) -> Option<Self> {
        (setting == Some("1")).then_some(Self {
            since_last_line: 0.0,
            last_inside: None,
        })
    }

    /// Whether this frame's probe deserves a line: every interval, when the player steps into
    /// the start zone, and whenever the entrance started.
    pub(super) fn should_log(&mut self, dt: f32, probe: &EntranceProbe, fired: bool) -> bool {
        self.since_last_line += dt;
        let stepped_in = match &self.last_inside {
            Some((source_ref, was_inside)) => {
                *source_ref == probe.source_ref && !was_inside && probe.inside_start
            }
            None => false,
        };
        self.last_inside = Some((probe.source_ref.clone(), probe.inside_start));
        let due = self.since_last_line >= TRACE_INTERVAL_SECONDS;
        if due || stepped_in || fired {
            self.since_last_line = 0.0;
            return true;
        }
        false
    }
}

pub(super) fn trace_line(probe: &EntranceProbe, fired: bool, live: bool) -> String {
    format!(
        "WALK_IN_TRACE ref={} distance={:.1} display_limit={:.1} start_limit={:.1} shown={} inside_start={} armed={} fired={fired} live={live}",
        probe.source_ref,
        probe.distance_units,
        probe.limits.display_units,
        probe.limits.start_units,
        probe.shown,
        probe.inside_start,
        probe.armed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::walk_in::EntranceDistances;

    fn probe(source_ref: &str, inside_start: bool) -> EntranceProbe {
        EntranceProbe {
            source_ref: source_ref.to_owned(),
            distance_units: 340.0,
            limits: EntranceDistances {
                display_units: 511.5,
                start_units: 311.5,
            },
            shown: true,
            inside_start,
            armed: true,
        }
    }

    fn trace() -> WalkInTrace {
        WalkInTrace::from_setting(Some("1")).unwrap()
    }

    #[test]
    fn only_the_value_one_enables_the_trace() {
        assert!(WalkInTrace::from_setting(None).is_none());
        assert!(WalkInTrace::from_setting(Some("0")).is_none());
        assert!(WalkInTrace::from_setting(Some("true")).is_none());
        assert!(WalkInTrace::from_setting(Some("1")).is_some());
    }

    #[test]
    fn lines_are_throttled_to_the_interval() {
        let mut trace = trace();
        let outside = probe("A", false);
        assert!(!trace.should_log(0.2, &outside, false));
        assert!(!trace.should_log(0.2, &outside, false));
        assert!(trace.should_log(0.2, &outside, false));
        assert!(!trace.should_log(0.2, &outside, false));
    }

    #[test]
    fn stepping_into_the_start_zone_logs_at_once() {
        let mut trace = trace();
        assert!(!trace.should_log(0.01, &probe("A", false), false));
        assert!(trace.should_log(0.01, &probe("A", true), false));
    }

    #[test]
    fn staying_inside_does_not_log_every_frame() {
        let mut trace = trace();
        trace.should_log(0.01, &probe("A", false), false);
        assert!(trace.should_log(0.01, &probe("A", true), false));
        assert!(!trace.should_log(0.01, &probe("A", true), false));
    }

    #[test]
    fn being_inside_a_different_nearest_entrance_is_not_a_transition() {
        let mut trace = trace();
        trace.should_log(0.01, &probe("A", false), false);
        assert!(!trace.should_log(0.01, &probe("B", true), false));
    }

    #[test]
    fn a_start_always_logs() {
        let mut trace = trace();
        assert!(trace.should_log(0.0, &probe("A", false), true));
    }

    #[test]
    fn the_line_names_every_diagnostic_field() {
        let line = trace_line(&probe("12F822:Fallout4.esm", true), true, false);
        for part in [
            "WALK_IN_TRACE ref=12F822:Fallout4.esm",
            "distance=340.0",
            "display_limit=511.5",
            "start_limit=311.5",
            "shown=true",
            "inside_start=true",
            "armed=true",
            "fired=true",
        ] {
            assert!(line.contains(part), "{part} missing in {line}");
        }
        assert!(line.ends_with("fired=true live=false"), "{line}");
    }
}
