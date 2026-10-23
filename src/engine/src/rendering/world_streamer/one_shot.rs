//! A condition that reports true once per arming, on the first poll after it holds.

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct OneShot {
    armed: bool,
}

impl OneShot {
    pub(super) fn arm(&mut self) {
        self.armed = true;
    }

    /// True once per arming, on the first poll where `condition` holds.
    pub(super) fn fire(&mut self, condition: bool) -> bool {
        let fire = self.armed && condition;
        if fire {
            self.armed = false;
        }
        fire
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_never_fires_before_it_is_armed() {
        let mut signal = OneShot::default();
        assert!(!signal.fire(true));
    }

    #[test]
    fn it_waits_for_the_condition_and_fires_exactly_once() {
        let mut signal = OneShot::default();
        signal.arm();
        assert!(!signal.fire(false));
        assert!(signal.fire(true));
        assert!(!signal.fire(true));
    }

    #[test]
    fn arming_again_re_arms_it_for_the_next_load() {
        let mut signal = OneShot::default();
        signal.arm();
        assert!(signal.fire(true));
        signal.arm();
        assert!(signal.fire(true));
    }
}
