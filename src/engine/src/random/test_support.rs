//! A random source that replays scripted answers, shared by the tests of everything that rolls.

use std::collections::VecDeque;

use super::RandomSource;

/// Replays fixed answers, so a test steers every roll and strike. An answer that is missing or
/// outside the asked range fails the test.
#[derive(Debug, Default)]
pub struct ScriptedRandom {
    below: VecDeque<usize>,
    units: VecDeque<f32>,
    asked_below: Vec<usize>,
}

impl ScriptedRandom {
    pub fn new(below: &[usize], units: &[f32]) -> Self {
        Self {
            below: below.iter().copied().collect(),
            units: units.iter().copied().collect(),
            asked_below: Vec::new(),
        }
    }

    /// A script that fails the test on any draw.
    pub fn silent() -> Self {
        Self::default()
    }

    pub fn is_spent(&self) -> bool {
        self.below.is_empty() && self.units.is_empty()
    }

    /// The bound of every `below` draw so far.
    pub fn asked_below(&self) -> &[usize] {
        &self.asked_below
    }
}

impl RandomSource for ScriptedRandom {
    fn below(&mut self, n: usize) -> usize {
        self.asked_below.push(n);
        let answer = self
            .below
            .pop_front()
            .expect("the script has no `below` answer left");
        assert!(answer < n, "scripted {answer} is out of range 0..{n}");
        answer
    }

    fn unit(&mut self) -> f32 {
        self.units
            .pop_front()
            .expect("the script has no `unit` answer left")
    }
}
