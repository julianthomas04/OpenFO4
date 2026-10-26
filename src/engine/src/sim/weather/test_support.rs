//! Fixtures of the weather cycle tests.

use std::collections::VecDeque;

use crate::random::RandomSource;
use crate::weather::{
    ClimateEntry, FadeRange, LightningParams, WeatherId, WeatherRollTable, WeatherRollTraits,
};

pub(crate) const A: WeatherId = WeatherId::new(0);
pub(crate) const B: WeatherId = WeatherId::new(1);
pub(crate) const C: WeatherId = WeatherId::new(2);

/// Replays fixed answers, so a test steers every roll and strike. An answer that is missing or
/// outside the asked range fails the test.
#[derive(Debug, Default)]
pub(super) struct ScriptedRandom {
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

/// A weather that takes 1.5625 h to arrive (transition byte 25), has no volatility multiplier and
/// no thunder.
pub(crate) fn calm() -> WeatherRollTraits {
    WeatherRollTraits {
        trans_delta_raw: 25,
        volatility_mult: 1.0,
        lightning: LightningParams {
            fade: FadeRange {
                begin_fade_in: 0.0,
                end_fade_out: 1.0,
            },
            frequency: 1.0,
            color_srgb: [255, 255, 255],
        },
        has_thunder_sound: false,
    }
}

/// Like [`calm`], with thunder and the most frequent lightning: a strike every 5 s on average.
pub(crate) fn stormy() -> WeatherRollTraits {
    let calm = calm();
    WeatherRollTraits {
        lightning: LightningParams {
            frequency: 15.0 / 255.0,
            ..calm.lightning
        },
        has_thunder_sound: true,
        ..calm
    }
}

/// A table of `weathers` in which weather `i` has the chance `chances[i]` and the cycle starts on
/// the first weather.
pub(crate) fn table(
    climate_volatility: u8,
    weathers: Vec<WeatherRollTraits>,
    chances: &[u32],
) -> WeatherRollTable {
    let entries = chances
        .iter()
        .enumerate()
        .map(|(index, &chance)| ClimateEntry {
            weather: WeatherId::new(index),
            chance,
        })
        .collect();
    WeatherRollTable::new(climate_volatility, entries, weathers, A)
        .expect("the fixture table is valid")
}
