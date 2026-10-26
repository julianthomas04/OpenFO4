//! The state of the weather cycle and how it moves with the world clock.

use super::lightning::{LightningStrike, StrikeClock, StrikeSource, strike_at, strike_source};
use super::roll::{pick_weighted, roll_interval_hours, transition_hours};
use crate::random::{RandomSource, SplitMix64};
use crate::weather::{TransitionSide, WeatherId, WeatherRollTable, WeatherRollTraits};

mod edit;

pub use edit::WeatherEditError;

/// What one frame tells the cycle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeatherStep {
    /// The world clock's `total_hours` after the frame.
    pub clock_hours: f64,
    /// The seconds of world time of the frame, not scaled by the time scale of the clock.
    pub world_seconds: f32,
}

/// What the renderer, audio and the dev panel read each frame.
#[derive(Debug, Clone, Copy)]
pub struct WeatherSnapshot<'a> {
    pub current: WeatherId,
    /// The weather being left; `None` while settled.
    pub previous: Option<WeatherId>,
    /// How far the transition from `previous` to `current` has come, `0..=1`; 1 while settled.
    pub progress: f32,
    /// The strikes of the last step.
    pub strikes: &'a [LightningStrike],
    /// Game hours until the next roll; 0 while one waits for a transition to end.
    pub hours_to_next_roll: f32,
}

impl WeatherSnapshot<'_> {
    /// The weathers that show and the part each plays in the transition, the incoming one first.
    pub fn sides(&self) -> impl Iterator<Item = (WeatherId, TransitionSide)> + use<> {
        sides(self.current, self.previous, self.progress)
    }
}

/// The weather of a climate over time. It rolls for a new weather now and then, moves to it in a
/// transition and lets storms strike. `R` supplies the chance.
#[derive(Debug)]
pub struct WeatherCycle<R = SplitMix64> {
    table: WeatherRollTable,
    rng: R,
    current: WeatherId,
    previous: Option<WeatherId>,
    progress: f32,
    hours_until_roll: f32,
    clock_hours: f64,
    strike_clock: StrikeClock,
    requested_strikes: u32,
    strikes: Vec<LightningStrike>,
}

impl<R: RandomSource> WeatherCycle<R> {
    /// Starts on the initial weather of the climate, settled, with the first roll one interval
    /// away. `clock_hours` is the `total_hours` of the world clock now.
    pub fn new(table: WeatherRollTable, rng: R, clock_hours: f64) -> Self {
        let current = table.initial();
        let mut cycle = Self {
            table,
            rng,
            current,
            previous: None,
            progress: 1.0,
            hours_until_roll: 0.0,
            clock_hours,
            strike_clock: StrikeClock::default(),
            requested_strikes: 0,
            strikes: Vec::new(),
        };
        cycle.restart_roll_clock();
        cycle
    }

    /// Moves the cycle on to the clock hours and the world seconds of one frame. The hours that
    /// have passed drive the transition and the rolls, so the cycle follows the time scale, the
    /// pause and the Pip-Boy as the clock does; a clock that went back moves nothing and the cycle
    /// carries on from there. The world seconds drive the lightning.
    pub fn advance(&mut self, step: WeatherStep) {
        self.strikes.clear();
        let hours = self.elapsed_hours(step.clock_hours);
        self.progress_transition(hours);
        self.hours_until_roll -= hours;
        if self.previous.is_none() && self.hours_until_roll <= 0.0 {
            self.roll();
        }
        self.strike(step.world_seconds);
    }

    pub fn snapshot(&self) -> WeatherSnapshot<'_> {
        WeatherSnapshot {
            current: self.current,
            previous: self.previous,
            progress: self.progress,
            strikes: &self.strikes,
            hours_to_next_roll: self.hours_until_roll.max(0.0),
        }
    }

    fn traits(&self, weather: WeatherId) -> &WeatherRollTraits {
        self.table
            .traits(weather)
            .expect("the cycle only holds weathers of its table")
    }

    /// The game hours since the last step; none when the clock went back or is not a number.
    fn elapsed_hours(&mut self, clock_hours: f64) -> f32 {
        if !clock_hours.is_finite() {
            return 0.0;
        }
        let elapsed = clock_hours - self.clock_hours;
        self.clock_hours = clock_hours;
        if elapsed > 0.0 { elapsed as f32 } else { 0.0 }
    }

    fn progress_transition(&mut self, hours: f32) {
        if self.previous.is_none() {
            return;
        }
        let duration = transition_hours(self.traits(self.current).trans_delta_raw);
        self.progress += hours / duration;
        if self.progress >= 1.0 {
            self.settle();
        }
    }

    fn settle(&mut self) {
        self.previous = None;
        self.progress = 1.0;
    }

    /// Leaves the current weather for `to` and schedules the next roll by the new weather.
    fn begin_transition(&mut self, to: WeatherId) {
        self.previous = Some(self.current);
        self.current = to;
        self.progress = 0.0;
        self.restart_roll_clock();
    }

    /// A roll that picks another weather starts the transition to it. Either way the next roll
    /// is a whole interval away, whatever time the clock overshot this one by.
    fn roll(&mut self) {
        let picked = pick_weighted(&self.table, &mut self.rng);
        if picked == self.current {
            self.restart_roll_clock();
        } else {
            self.begin_transition(picked);
        }
    }

    fn restart_roll_clock(&mut self) {
        let volatility_mult = self.traits(self.current).volatility_mult;
        self.hours_until_roll =
            roll_interval_hours(self.table.climate_volatility(), volatility_mult);
    }

    fn strike(&mut self, world_seconds: f32) {
        for _ in 0..std::mem::take(&mut self.requested_strikes) {
            let strike = strike_at(self.current, 1.0, &mut self.rng);
            self.strikes.push(strike);
        }
        let sources: Vec<StrikeSource> = sides(self.current, self.previous, self.progress)
            .filter_map(|(weather, side)| strike_source(weather, self.traits(weather), side))
            .collect();
        if let Some(strike) = self
            .strike_clock
            .advance(&sources, world_seconds, &mut self.rng)
        {
            self.strikes.push(strike);
        }
    }
}

fn sides(
    current: WeatherId,
    previous: Option<WeatherId>,
    progress: f32,
) -> impl Iterator<Item = (WeatherId, TransitionSide)> + use<> {
    let sides = match previous {
        Some(previous) => [
            Some((current, TransitionSide::Incoming { progress })),
            Some((previous, TransitionSide::Outgoing { progress })),
        ],
        None => [Some((current, TransitionSide::Settled)), None],
    };
    sides.into_iter().flatten()
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{A, B, C, ScriptedRandom, calm, stormy, table};
    use super::*;
    use crate::sim::commands::WeatherEdit;
    use crate::weather::FadeRange;

    /// A cycle with a clock of its own, advanced by game hours and world seconds.
    pub(super) struct Run {
        pub cycle: WeatherCycle<ScriptedRandom>,
        hours: f64,
    }

    /// Where a cycle stands, without the strikes: the current and the previous weather, the
    /// progress and the hours to the next roll.
    pub(super) type Place = (WeatherId, Option<WeatherId>, f32, f32);

    impl Run {
        pub fn new(table: WeatherRollTable, rng: ScriptedRandom) -> Self {
            Self {
                cycle: WeatherCycle::new(table, rng, 0.0),
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

    /// Three calm weathers of the chances 50, 30 and 20 in a climate without volatility: a roll
    /// every 8 h, and the tickets 0..50, 50..80 and 80..100 pick A, B and C. Each takes 1.5625 h
    /// to arrive.
    pub(super) fn calm_run(below: &[usize]) -> Run {
        Run::new(
            table(0, vec![calm(); 3], &[50, 30, 20]),
            ScriptedRandom::new(below, &[]),
        )
    }

    /// A stormy and a calm weather in a climate without volatility; the cycle starts on the storm,
    /// which strikes every 5 s on average.
    pub(super) fn storm_run(units: &[f32]) -> Run {
        Run::new(
            table(0, vec![stormy(), calm()], &[1, 1]),
            ScriptedRandom::new(&[], units),
        )
    }

    #[test]
    fn a_new_cycle_starts_settled_on_the_initial_weather() {
        let run = calm_run(&[]);
        assert_eq!(run.place(), (A, None, 1.0, 8.0));
        assert_eq!(run.snapshot().strikes, []);
    }

    #[test]
    fn the_next_roll_counts_down_with_the_game_hours_of_the_clock() {
        let mut run = calm_run(&[]);
        run.hours(3.0);
        assert_eq!(run.snapshot().hours_to_next_roll, 5.0);
        run.hours(0.5);
        assert_eq!(run.snapshot().hours_to_next_roll, 4.5);
    }

    #[test]
    fn a_roll_that_picks_another_weather_starts_the_transition_to_it_at_zero_progress() {
        let mut run = calm_run(&[50]);
        run.hours(8.0);
        assert_eq!(run.place(), (B, Some(A), 0.0, 8.0));
    }

    #[test]
    fn a_roll_drops_the_time_the_clock_overshot_it_by() {
        let mut run = calm_run(&[50]);
        run.hours(10.0);
        assert_eq!(run.place(), (B, Some(A), 0.0, 8.0));
    }

    #[test]
    fn a_roll_that_picks_the_current_weather_only_schedules_the_next_roll() {
        let mut run = calm_run(&[10, 49]);
        run.hours(8.0);
        assert_eq!(run.place(), (A, None, 1.0, 8.0));
        run.hours(5.0);
        assert_eq!(run.snapshot().hours_to_next_roll, 3.0);
        run.hours(3.0);
        assert_eq!(run.place(), (A, None, 1.0, 8.0));
    }

    #[test]
    fn the_next_roll_is_scheduled_by_the_weather_that_was_rolled() {
        let mut weathers = vec![calm(); 2];
        weathers[1].volatility_mult = 2.0;
        let mut run = Run::new(table(0, weathers, &[1, 1]), ScriptedRandom::new(&[1], &[]));
        run.hours(8.0);
        assert_eq!(run.place(), (B, Some(A), 0.0, 4.0));
    }

    #[test]
    fn a_transition_advances_with_the_hours_and_settles_at_its_end() {
        let mut run = calm_run(&[50]);
        run.hours(8.0);
        run.hours(0.78125);
        assert_eq!(run.snapshot().progress, 0.5);
        assert_eq!(run.snapshot().previous, Some(A));
        run.hours(0.78125);
        assert_eq!(run.place(), (B, None, 1.0, 6.4375));
    }

    #[test]
    fn a_long_step_settles_the_transition_in_one_go() {
        let mut run = calm_run(&[50]);
        run.hours(8.0);
        run.hours(5.0);
        assert_eq!(run.place(), (B, None, 1.0, 3.0));
    }

    #[test]
    fn the_incoming_weather_sets_the_length_of_the_transition() {
        let mut weathers = vec![calm(); 3];
        weathers[1].trans_delta_raw = 125;
        weathers[2].trans_delta_raw = 0;
        let mut run = Run::new(
            table(0, weathers, &[1, 1, 1]),
            ScriptedRandom::new(&[1, 2], &[]),
        );
        run.hours(8.0);
        run.hours(1.5);
        assert_eq!((run.snapshot().current, run.snapshot().progress), (B, 0.5));
        run.hours(1.5);
        assert_eq!(run.snapshot().previous, None);
        run.hours(8.0);
        assert_eq!(run.snapshot().current, C);
        assert_eq!(run.snapshot().previous, Some(B));
        run.hours(0.25);
        assert_eq!(run.snapshot().previous, None);
    }

    #[test]
    fn a_roll_that_comes_due_during_a_transition_waits_until_it_ends() {
        // A roll every 0.5 h and a transition of 3 h, the longest there is.
        let mut weathers = vec![calm(); 3];
        for traits in &mut weathers {
            traits.volatility_mult = 4.0;
            traits.trans_delta_raw = 125;
        }
        let mut run = Run::new(
            table(255, weathers, &[1, 1, 1]),
            ScriptedRandom::new(&[1, 2], &[]),
        );
        run.hours(0.5);
        assert_eq!(run.place(), (B, Some(A), 0.0, 0.5));
        run.hours(1.0);
        run.hours(1.9);
        let waiting = run.place();
        assert_eq!((waiting.0, waiting.1), (B, Some(A)));
        assert!(waiting.2 < 1.0);
        assert_eq!(waiting.3, 0.0);
        run.hours(0.2);
        assert_eq!(run.place(), (C, Some(B), 0.0, 0.5));
    }

    #[test]
    fn a_clock_that_went_back_moves_nothing_and_the_cycle_carries_on_from_there() {
        let mut run = calm_run(&[]);
        run.jump_to(5.0);
        let before = run.place();
        run.jump_to(2.0);
        assert_eq!(run.place(), before);
        run.hours(1.0);
        assert_eq!(run.snapshot().hours_to_next_roll, 2.0);
    }

    #[test]
    fn a_clock_that_jumped_ahead_counts_as_elapsed_time() {
        let mut run = calm_run(&[]);
        run.jump_to(5.0);
        assert_eq!(run.snapshot().hours_to_next_roll, 3.0);
    }

    #[test]
    fn a_clock_that_is_not_a_number_is_ignored_and_the_cycle_keeps_its_place() {
        let mut run = calm_run(&[]);
        run.hours(2.0);
        let before = run.place();
        for clock_hours in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            run.jump_to(clock_hours);
            assert_eq!(run.place(), before);
        }
        run.jump_to(3.0);
        assert_eq!(run.snapshot().hours_to_next_roll, 5.0);
    }

    #[test]
    fn a_step_without_time_changes_nothing() {
        let mut run = calm_run(&[50]);
        run.hours(8.0);
        run.hours(0.5);
        let before = run.place();
        run.seconds(0.0);
        assert_eq!(run.place(), before);
    }

    #[test]
    fn a_weather_without_a_thunder_sound_never_strikes() {
        let mut run = calm_run(&[]);
        run.seconds(1000.0);
        assert_eq!(run.snapshot().strikes, []);
        assert!(run.cycle.rng.is_spent());
    }

    #[test]
    fn a_storm_strikes_when_its_wait_is_over_and_the_strike_lasts_one_step() {
        // A wait of ln 2 expected strikes at 0.2 strikes per second is 3.47 s.
        let mut run = storm_run(&[0.5, 0.0]);
        run.seconds(3.0);
        assert_eq!(run.snapshot().strikes, []);
        run.seconds(0.5);
        assert_eq!(
            run.snapshot().strikes,
            [LightningStrike {
                weather: A,
                distance_m: 300.0,
                thunder_delay_s: 300.0 / 343.0,
                flash_intensity: 1.0,
            }]
        );
        run.seconds(0.0);
        assert_eq!(run.snapshot().strikes, []);
    }

    #[test]
    fn strikes_run_on_world_seconds_and_not_on_the_game_hours_of_the_clock() {
        let mut run = storm_run(&[]);
        run.hours(5.0);
        assert_eq!(run.snapshot().strikes, []);
        assert!(run.cycle.rng.is_spent());
    }

    #[test]
    fn a_fading_storm_strikes_less_often() {
        let mut run = storm_run(&[0.5, 0.0]);
        run.edit(WeatherEdit::Transition(B));
        run.edit(WeatherEdit::SetProgress(0.5));
        // Half the thunder is left, so the wait of 3.47 s at full strength is twice as long.
        run.seconds(6.9);
        assert_eq!(run.snapshot().strikes, []);
        run.seconds(0.1);
        let strikes = run.snapshot().strikes;
        assert_eq!(strikes.len(), 1);
        assert_eq!(strikes[0].weather, A);
        assert_eq!(strikes[0].flash_intensity, 0.5);
    }

    #[test]
    fn during_a_transition_between_storms_a_strike_comes_from_either_side() {
        let strike_weather = |units: &[f32]| {
            let mut run = Run::new(
                table(0, vec![stormy(), stormy()], &[1, 1]),
                ScriptedRandom::new(&[], units),
            );
            run.edit(WeatherEdit::Transition(B));
            run.edit(WeatherEdit::SetProgress(0.5));
            run.seconds(100.0);
            run.snapshot().strikes[0].weather
        };
        // The draws are the wait, the pick between the two sides and the distance.
        assert_eq!(strike_weather(&[0.5, 0.1, 0.0]), B);
        assert_eq!(strike_weather(&[0.5, 0.9, 0.0]), A);
    }

    #[test]
    fn a_storm_whose_thunder_has_not_faded_in_does_not_strike() {
        let mut late = stormy();
        late.lightning.fade = FadeRange {
            begin_fade_in: 0.5,
            end_fade_out: 1.0,
        };
        let mut run = Run::new(
            table(0, vec![calm(), late], &[1, 1]),
            ScriptedRandom::silent(),
        );
        run.edit(WeatherEdit::Transition(B));
        run.edit(WeatherEdit::SetProgress(0.25));
        run.seconds(100.0);
        assert_eq!(run.snapshot().strikes, []);
    }

    #[test]
    fn the_sides_of_a_transition_are_the_incoming_and_the_outgoing_weather() {
        let mut run = calm_run(&[50]);
        run.hours(8.0);
        run.hours(0.78125);
        let sides: Vec<_> = run.snapshot().sides().collect();
        assert_eq!(
            sides,
            [
                (B, TransitionSide::Incoming { progress: 0.5 }),
                (A, TransitionSide::Outgoing { progress: 0.5 }),
            ]
        );
    }

    #[test]
    fn a_settled_cycle_has_one_settled_side() {
        let run = calm_run(&[]);
        let sides: Vec<_> = run.snapshot().sides().collect();
        assert_eq!(sides, [(A, TransitionSide::Settled)]);
    }
}
