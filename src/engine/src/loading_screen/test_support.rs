//! A random source that replays scripted values and records what it was asked.

use std::collections::VecDeque;

use crate::random::RandomSource;

#[derive(Debug, Default)]
pub struct ScriptedRandom {
    units: VecDeque<f32>,
    picks: VecDeque<usize>,
    /// The `n` of every `below` call, in order.
    pub below_calls: Vec<usize>,
}

impl ScriptedRandom {
    pub fn with_units(units: impl IntoIterator<Item = f32>) -> Self {
        Self {
            units: units.into_iter().collect(),
            ..Self::default()
        }
    }

    pub fn with_picks(picks: impl IntoIterator<Item = usize>) -> Self {
        Self {
            picks: picks.into_iter().collect(),
            ..Self::default()
        }
    }
}

impl RandomSource for ScriptedRandom {
    fn below(&mut self, n: usize) -> usize {
        self.below_calls.push(n);
        let pick = self.picks.pop_front().expect("a scripted pick");
        assert!(pick < n, "scripted pick {pick} is not below {n}");
        pick
    }

    fn unit(&mut self) -> f32 {
        self.units.pop_front().expect("a scripted unit value")
    }
}
