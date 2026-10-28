//! Fixtures of the weather cycle tests.

use std::collections::VecDeque;

use super::{WeatherCycle, WeatherSnapshot, WeatherStep};
use crate::random::RandomSource;
use crate::sim::commands::WeatherEdit;
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

/// A weather that takes 1 h to arrive (transition byte 100), has no volatility multiplier and
/// no thunder.
pub(crate) fn calm() -> WeatherRollTraits {
    WeatherRollTraits {
        trans_delta_raw: 100,
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

/// A climate that changes its weather often, with two storms and a calm weather of equal chances:
/// a roll every 2 h, and 1 h to arrive at any of the weathers.
pub(crate) fn busy_climate() -> WeatherRollTable {
    table(255, vec![stormy(), calm(), stormy()], &[1, 1, 1])
}

/// A weather cycle on a clock of its own, which a test advances by game hours and world seconds.
/// The rolls and the lightning draw from scripts.
pub(super) struct Run {
    pub cycle: WeatherCycle<ScriptedRandom>,
    hours: f64,
}

/// Where a cycle stands, apart from its strikes: the current and the previous weather, the
/// progress and the game hours to the next roll.
pub(super) type Place = (WeatherId, Option<WeatherId>, f32, f32);

impl Run {
    /// `rolls` are the tickets the rolls draw and `strikes` the unit draws of the lightning.
    pub fn new(table: WeatherRollTable, rolls: &[usize], strikes: &[f32]) -> Self {
        let rolls = ScriptedRandom::new(rolls, &[]);
        let strikes = ScriptedRandom::new(&[], strikes);
        Self {
            cycle: WeatherCycle::new(table, rolls, strikes, 0.0),
            hours: 0.0,
        }
    }

    pub fn hours(&mut self, hours: f64) {
        self.jump_to(self.hours + hours);
    }

    pub fn jump_to(&mut self, clock_hours: f64) {
        self.hours = clock_hours;
        self.cycle.advance(WeatherStep {
            clock_hours,
            world_seconds: 0.0,
        });
    }

    pub fn seconds(&mut self, world_seconds: f32) {
        self.cycle.advance(WeatherStep {
            clock_hours: self.hours,
            world_seconds,
        });
    }

    pub fn snapshot(&self) -> WeatherSnapshot<'_> {
        self.cycle.snapshot()
    }

    pub fn place(&self) -> Place {
        let snapshot = self.snapshot();
        (
            snapshot.current,
            snapshot.previous,
            snapshot.progress,
            snapshot.hours_to_next_roll,
        )
    }

    pub fn edit(&mut self, edit: WeatherEdit) {
        self.cycle.apply(edit).unwrap();
    }
}

/// Asserts that two places hold the same weathers and agree on the numbers to within float error.
pub(super) fn assert_same_place(actual: Place, expected: Place) {
    assert_eq!((actual.0, actual.1), (expected.0, expected.1));
    assert!(
        (actual.2 - expected.2).abs() < 1e-4,
        "progress {} is not {}",
        actual.2,
        expected.2
    );
    assert!(
        (actual.3 - expected.3).abs() < 1e-4,
        "hours to the next roll {} are not {}",
        actual.3,
        expected.3
    );
}

/// Three calm weathers of the chances 50, 30 and 20 in a climate without volatility: a roll after a
/// weather has stayed for 8 h, and the tickets 0..50, 50..80 and 80..100 pick A, B and C. Each takes
/// 1 h to arrive.
pub(super) fn calm_run(rolls: &[usize]) -> Run {
    Run::new(table(0, vec![calm(); 3], &[50, 30, 20]), rolls, &[])
}

/// A stormy and a calm weather in a climate without volatility; the cycle starts on the storm,
/// which strikes every 5 s on average. The strikes draw the given units.
pub(super) fn storm_run(strikes: &[f32]) -> Run {
    Run::new(table(0, vec![stormy(), calm()], &[1, 1]), &[], strikes)
}
