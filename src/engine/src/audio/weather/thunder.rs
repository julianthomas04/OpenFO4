//! Thunder that is on its way: it follows its flash after the time the sound needs to arrive.
//! Pure: the clock is an input.

use std::time::Duration;

use super::input::{ThunderStrike, unit};
use crate::weather::WeatherId;

/// How much quieter than its sound's own level the thunder of the faintest strike is, in dB. The
/// loudest strike plays at the sound's own level, and the dB in between follow the strike's
/// intensity.
pub const FAINTEST_THUNDER_DB: f32 = -24.0;

/// Thunder that has come due.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DueThunder {
    pub weather: WeatherId,
    /// Added to the gain of the thunder sound's layers.
    pub gain_db: f32,
}

#[derive(Debug)]
struct Pending {
    due: Duration,
    thunder: DueThunder,
}

#[derive(Debug, Default)]
pub struct ThunderQueue {
    pending: Vec<Pending>,
}

impl ThunderQueue {
    /// Queues the thunder of `strike`, which sounds `strike.delay_s` after `now`. A delay that is
    /// negative or not a number counts as none, one beyond what a duration holds as never.
    pub fn push(&mut self, strike: ThunderStrike, now: Duration) {
        self.pending.push(Pending {
            due: now.saturating_add(delay(strike.delay_s)),
            thunder: DueThunder {
                weather: strike.weather,
                gain_db: thunder_gain_db(strike.intensity),
            },
        });
    }

    /// The thunder due at `now` or before, the earliest first; it leaves the queue.
    pub fn take_due(&mut self, now: Duration) -> Vec<DueThunder> {
        let (mut due, waiting): (Vec<Pending>, Vec<Pending>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|pending| pending.due <= now);
        self.pending = waiting;
        due.sort_by_key(|pending| pending.due);
        due.into_iter().map(|pending| pending.thunder).collect()
    }

    /// Drops the thunder that is on its way.
    pub fn clear(&mut self) {
        self.pending.clear();
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.pending.len()
    }
}

/// The gain of the thunder of a strike of `intensity`: 0 dB at 1 and [`FAINTEST_THUNDER_DB`] at 0,
/// linear in dB between. An intensity outside `0..=1` is held to it and NaN counts as 0.
pub fn thunder_gain_db(intensity: f32) -> f32 {
    FAINTEST_THUNDER_DB * (1.0 - unit(intensity))
}

fn delay(seconds: f32) -> Duration {
    if seconds.is_nan() || seconds <= 0.0 {
        return Duration::ZERO;
    }
    Duration::try_from_secs_f32(seconds).unwrap_or(Duration::MAX)
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the expected gains are exactly representable"
)]
mod tests {
    use super::*;

    const STORM: WeatherId = WeatherId::new(1);
    const RAIN: WeatherId = WeatherId::new(2);
    const START: Duration = Duration::from_secs(10);

    fn strike(weather: WeatherId, delay_s: f32, intensity: f32) -> ThunderStrike {
        ThunderStrike {
            weather,
            delay_s,
            intensity,
        }
    }

    fn weathers(due: &[DueThunder]) -> Vec<WeatherId> {
        due.iter().map(|thunder| thunder.weather).collect()
    }

    #[test]
    fn a_strike_sounds_once_its_delay_has_passed_and_only_once() {
        let mut queue = ThunderQueue::default();
        queue.push(strike(STORM, 2.5, 1.0), START);

        assert_eq!(queue.take_due(START), []);
        assert_eq!(queue.take_due(START + Duration::from_millis(2499)), []);
        assert_eq!(
            weathers(&queue.take_due(START + Duration::from_millis(2500))),
            [STORM]
        );
        assert_eq!(queue.take_due(START + Duration::from_secs(60)), []);
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn a_strike_that_is_late_in_being_asked_for_still_sounds() {
        let mut queue = ThunderQueue::default();
        queue.push(strike(STORM, 1.0, 1.0), START);

        assert_eq!(
            weathers(&queue.take_due(START + Duration::from_secs(30))),
            [STORM]
        );
    }

    #[test]
    fn strikes_come_due_in_the_order_of_their_delays_and_the_others_wait() {
        let mut queue = ThunderQueue::default();
        queue.push(strike(STORM, 3.0, 1.0), START);
        queue.push(strike(RAIN, 1.0, 1.0), START);
        queue.push(strike(STORM, 9.0, 1.0), START);

        let first = queue.take_due(START + Duration::from_secs(4));

        assert_eq!(weathers(&first), [RAIN, STORM]);
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn the_delay_counts_from_the_moment_the_strike_was_queued() {
        let mut queue = ThunderQueue::default();
        queue.push(strike(STORM, 2.0, 1.0), START);
        let later = START + Duration::from_secs(5);
        queue.push(strike(RAIN, 2.0, 1.0), later);

        assert_eq!(
            weathers(&queue.take_due(START + Duration::from_secs(2))),
            [STORM]
        );
        assert_eq!(queue.take_due(later + Duration::from_secs(1)), []);
        assert_eq!(
            weathers(&queue.take_due(later + Duration::from_secs(2))),
            [RAIN]
        );
    }

    #[test]
    fn a_strike_without_a_usable_delay_sounds_at_once() {
        for delay_s in [0.0, -3.0, f32::NAN, f32::NEG_INFINITY] {
            let mut queue = ThunderQueue::default();
            queue.push(strike(STORM, delay_s, 1.0), START);

            assert_eq!(weathers(&queue.take_due(START)), [STORM], "{delay_s}");
        }
    }

    #[test]
    fn a_strike_with_a_delay_no_duration_holds_never_sounds() {
        let mut queue = ThunderQueue::default();
        queue.push(strike(STORM, f32::INFINITY, 1.0), START);
        queue.push(strike(RAIN, f32::MAX, 1.0), START);

        assert_eq!(queue.take_due(Duration::from_secs(10_000_000)), []);
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn clearing_drops_the_thunder_that_is_on_its_way() {
        let mut queue = ThunderQueue::default();
        queue.push(strike(STORM, 1.0, 1.0), START);

        queue.clear();

        assert_eq!(queue.take_due(START + Duration::from_secs(60)), []);
    }

    #[test]
    fn the_gain_runs_from_the_faintest_to_the_sounds_own_level_with_the_intensity() {
        assert_eq!(thunder_gain_db(1.0), 0.0);
        assert_eq!(thunder_gain_db(0.0), FAINTEST_THUNDER_DB);
        assert_eq!(thunder_gain_db(0.5), FAINTEST_THUNDER_DB / 2.0);
    }

    #[test]
    fn a_stronger_strike_is_never_quieter() {
        let gains: Vec<f32> = (0..=10)
            .map(|step| thunder_gain_db(step as f32 / 10.0))
            .collect();

        assert!(gains.windows(2).all(|pair| pair[0] < pair[1]), "{gains:?}");
    }

    #[test]
    fn an_intensity_outside_the_range_or_not_a_number_is_held_to_it() {
        assert_eq!(thunder_gain_db(7.0), 0.0);
        assert_eq!(thunder_gain_db(-2.0), FAINTEST_THUNDER_DB);
        assert_eq!(thunder_gain_db(f32::NAN), FAINTEST_THUNDER_DB);
    }

    #[test]
    fn the_queued_thunder_carries_the_gain_of_its_strike() {
        let mut queue = ThunderQueue::default();
        queue.push(strike(STORM, 0.0, 0.25), START);

        assert_eq!(
            queue.take_due(START),
            [DueThunder {
                weather: STORM,
                gain_db: thunder_gain_db(0.25),
            }]
        );
    }
}
