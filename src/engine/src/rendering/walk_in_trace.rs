//! `OPENFO4_WALK_IN_TRACE=1`: a diagnostic trace of how close the player is to the nearest
//! walk-in trigger, to find out why a door does or does not fire. What to log and when lives
//! here; the renderer owns the logging and the blocker rays.

use super::walk_in::TriggerProbe;

/// Seconds between periodic trace lines.
const TRACE_INTERVAL_SECONDS: f32 = 0.5;

/// Gap, in Bethesda units, below which the renderer also names what stands between the player
/// and the trigger.
pub(super) const BLOCKER_RAY_GAP_UNITS: f32 = 20.0;

#[derive(Debug)]
pub(super) struct WalkInTrace {
    since_last_line: f32,
    last_touching: Option<(String, bool)>,
}

impl WalkInTrace {
    /// Enabled by `OPENFO4_WALK_IN_TRACE=1`; anything else leaves the trace off.
    pub(super) fn from_env() -> Option<Self> {
        Self::from_setting(std::env::var("OPENFO4_WALK_IN_TRACE").ok().as_deref())
    }

    fn from_setting(setting: Option<&str>) -> Option<Self> {
        (setting == Some("1")).then_some(Self {
            since_last_line: 0.0,
            last_touching: None,
        })
    }

    /// Whether this frame's probe deserves a line: every interval, when the capsule starts
    /// touching the trigger, and whenever the detector fired.
    pub(super) fn should_log(&mut self, dt: f32, probe: &TriggerProbe, fired: bool) -> bool {
        self.since_last_line += dt;
        let touching = probe.gap_units <= 0.0;
        let began_touching = match &self.last_touching {
            Some((source_ref, was_touching)) => {
                *source_ref == probe.source_ref && !was_touching && touching
            }
            None => false,
        };
        self.last_touching = Some((probe.source_ref.clone(), touching));
        let due = self.since_last_line >= TRACE_INTERVAL_SECONDS;
        if due || began_touching || fired {
            self.since_last_line = 0.0;
            return true;
        }
        false
    }
}

pub(super) fn trace_line(probe: &TriggerProbe, fired: bool) -> String {
    let point = |v: glam::Vec3| format!("({:.1},{:.1},{:.1})", v.x, v.y, v.z);
    format!(
        "WALK_IN_TRACE ref={} feet_local={} head_local={} box_min={} box_max={} gap={:.1} swept_prev={} armed={} fired={fired}",
        probe.source_ref,
        point(probe.feet_local),
        point(probe.head_local),
        point(probe.box_min),
        point(probe.box_max),
        probe.gap_units,
        probe.swept_prev,
        probe.armed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn probe(source_ref: &str, gap_units: f32) -> TriggerProbe {
        TriggerProbe {
            source_ref: source_ref.to_owned(),
            feet_local: Vec3::new(1.0, 2.0, 3.0),
            head_local: Vec3::new(1.0, 2.0, 130.0),
            box_min: Vec3::new(-64.0, -8.0, 0.0),
            box_max: Vec3::new(64.0, 8.0, 192.0),
            gap_units,
            swept_prev: false,
            armed: true,
            horizontal_to_center: Vec3::X,
            horizontal_distance: 5.0,
            ray_origins: [Vec3::ZERO; 3],
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
        let far = probe("A", 50.0);
        assert!(!trace.should_log(0.2, &far, false));
        assert!(!trace.should_log(0.2, &far, false));
        assert!(trace.should_log(0.2, &far, false));
        assert!(!trace.should_log(0.2, &far, false));
    }

    #[test]
    fn the_gap_reaching_zero_logs_at_once() {
        let mut trace = trace();
        assert!(!trace.should_log(0.01, &probe("A", 5.0), false));
        assert!(trace.should_log(0.01, &probe("A", 0.0), false));
    }

    #[test]
    fn staying_in_contact_does_not_log_every_frame() {
        let mut trace = trace();
        trace.should_log(0.01, &probe("A", 5.0), false);
        assert!(trace.should_log(0.01, &probe("A", 0.0), false));
        assert!(!trace.should_log(0.01, &probe("A", 0.0), false));
    }

    #[test]
    fn touching_a_different_nearest_door_is_not_a_transition() {
        let mut trace = trace();
        trace.should_log(0.01, &probe("A", 5.0), false);
        assert!(!trace.should_log(0.01, &probe("B", 0.0), false));
    }

    #[test]
    fn a_fire_always_logs() {
        let mut trace = trace();
        assert!(trace.should_log(0.0, &probe("A", 50.0), true));
    }

    #[test]
    fn the_line_names_every_diagnostic_field() {
        let line = trace_line(&probe("12F822:Fallout4.esm", 0.0), true);
        for part in [
            "WALK_IN_TRACE ref=12F822:Fallout4.esm",
            "feet_local=(1.0,2.0,3.0)",
            "head_local=(1.0,2.0,130.0)",
            "box_min=(-64.0,-8.0,0.0)",
            "box_max=(64.0,8.0,192.0)",
            "gap=0.0",
            "swept_prev=false",
            "armed=true",
            "fired=true",
        ] {
            assert!(line.contains(part), "{part} missing in {line}");
        }
    }
}
