//! The state of the weather cycle and how it moves with the world clock.

use super::lightning::{LightningStrike, StrikeClock, StrikeSource, strike_at, strike_source};
use super::roll::{pick_weighted, roll_interval_hours, transition_hours};
use crate::random::{RandomSource, SplitMix64};
use crate::weather::{TableWeather, TransitionSide, WeatherId, WeatherRollTable};

mod edit;

pub use edit::WeatherEditError;

/// Mixed into the seed of the lightning stream, so that the two streams of a seed differ.
const LIGHTNING_STREAM_SALT: u64 = 0xD6E8_FEB8_6659_FD93;

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
    /// Game hours until the next roll, which counts from the moment the weather settles; 0 when a
    /// roll is due.
    pub hours_to_next_roll: f32,
}

impl WeatherSnapshot<'_> {
    /// The weathers that show and the part each plays in the transition, the incoming one first.
    pub fn sides(&self) -> impl Iterator<Item = (WeatherId, TransitionSide)> + use<> {
        sides(self.current, self.previous, self.progress)
    }
}

/// The way from the weather that was current to the one that is, in game hours on the clock.
#[derive(Debug, Clone, Copy)]
struct Transition {
    from: TableWeather,
    start: f64,
    /// Above 0.
    duration: f64,
}

impl Transition {
    fn end(&self) -> f64 {
        self.start + self.duration
    }

    /// How far the transition has come at `clock_hours`, `0..=1`.
    fn progress(&self, clock_hours: f64) -> f32 {
        ((clock_hours - self.start) / self.duration).clamp(0.0, 1.0) as f32
    }
}

/// The weather of a climate over time. A weather stays settled for an interval that the climate
/// and the weather set. Then the cycle rolls for a weather by the chances of the climate, and a
/// roll that picks another weather starts the transition to it. Storms strike while their thunder
/// shows.
///
/// Every time is in game hours on the world clock and absolute: the end of the transition and the
/// next roll are deadlines, so the state at a clock reading does not depend on the steps that led
/// there. `R` supplies the chance. The rolls and the lightning draw from streams of their own, so
/// the lightning cannot change which weathers come.
#[derive(Debug)]
pub struct WeatherCycle<R = SplitMix64> {
    table: WeatherRollTable,
    roll_rng: R,
    strike_rng: R,
    current: TableWeather,
    transition: Option<Transition>,
    /// Counts from the end of the transition, so it is never due while one runs.
    next_roll_at: f64,
    clock_hours: f64,
    strike_clock: StrikeClock,
    requested_strikes: u32,
    strikes: Vec<LightningStrike>,
    strike_sources: Vec<StrikeSource>,
}

impl WeatherCycle {
    /// A cycle whose two streams of chance are derived from `seed`: the same seed brings the same
    /// weathers in the same order, however the clock is stepped and whether or not it storms.
    pub fn seeded(table: WeatherRollTable, seed: u64, clock_hours: f64) -> Self {
        let roll_rng = SplitMix64::new(seed);
        let strike_rng = SplitMix64::new(seed ^ LIGHTNING_STREAM_SALT);
        Self::new(table, roll_rng, strike_rng, clock_hours)
    }
}

impl<R: RandomSource> WeatherCycle<R> {
    /// Starts on the initial weather of the climate, settled, with the first roll one interval
    /// away. `clock_hours` is the `total_hours` of the world clock now, a finite number. The rolls
    /// draw from `roll_rng` and the lightning from `strike_rng`.
    pub fn new(table: WeatherRollTable, roll_rng: R, strike_rng: R, clock_hours: f64) -> Self {
        let current = table.initial_weather();
        let mut cycle = Self {
            table,
            roll_rng,
            strike_rng,
            current,
            transition: None,
            next_roll_at: clock_hours,
            clock_hours,
            strike_clock: StrikeClock::default(),
            requested_strikes: 0,
            strikes: Vec::new(),
            strike_sources: Vec::new(),
        };
        cycle.schedule_roll_after(clock_hours);
        cycle
    }

    /// Moves the cycle on to the clock hours and the world seconds of one frame. The clock hours
    /// drive the transition and the rolls, so the cycle follows the time scale, the pause and the
    /// Pip-Boy as the clock does; a clock that went back moves nothing and the cycle carries on
    /// from there. The world seconds drive the lightning.
    ///
    /// A roll takes place at its deadline, however far the step overshoots it. A step rolls at most
    /// once: when it passes several deadlines, the next step rolls again.
    pub fn advance(&mut self, step: WeatherStep) {
        self.strikes.clear();
        self.follow_clock(step.clock_hours);
        if self.next_roll_at <= self.clock_hours {
            self.roll();
        }
        self.settle_arrived_transition();
        self.strike(step.world_seconds);
    }

    pub fn snapshot(&self) -> WeatherSnapshot<'_> {
        WeatherSnapshot {
            current: self.current.id(),
            previous: self.transition.map(|transition| transition.from.id()),
            progress: self.progress(),
            strikes: &self.strikes,
            hours_to_next_roll: (self.next_roll_at - self.clock_hours).max(0.0) as f32,
        }
    }

    fn progress(&self) -> f32 {
        self.transition
            .map_or(1.0, |transition| transition.progress(self.clock_hours))
    }

    /// Takes the clock reading of a step. A reading that is not a number is ignored. A clock that
    /// went back takes the deadlines back with it, so the cycle keeps its place instead of
    /// rewinding.
    fn follow_clock(&mut self, clock_hours: f64) {
        if !clock_hours.is_finite() {
            return;
        }
        let shift = clock_hours - self.clock_hours;
        if shift < 0.0 {
            self.next_roll_at += shift;
            if let Some(transition) = &mut self.transition {
                transition.start += shift;
            }
        }
        self.clock_hours = clock_hours;
    }

    fn settle_arrived_transition(&mut self) {
        if self
            .transition
            .is_some_and(|transition| self.clock_hours >= transition.end())
        {
            self.transition = None;
        }
    }

    /// Rolls at the deadline. A roll that picks another weather starts the transition to it, one
    /// that keeps the weather only schedules the next roll.
    fn roll(&mut self) {
        let deadline = self.next_roll_at;
        let picked = pick_weighted(&self.table, &mut self.roll_rng);
        if picked.id() == self.current.id() {
            self.schedule_roll_after(deadline);
        } else {
            self.begin_transition(picked, deadline);
        }
    }

    /// Leaves the current weather for `to` at `start` hours on the clock; a weather whose
    /// transition takes no time settles at once.
    fn begin_transition(&mut self, to: TableWeather, start: f64) {
        let duration = transition_hours(to.traits().trans_delta_raw);
        let from = std::mem::replace(&mut self.current, to);
        if duration > 0.0 {
            let transition = Transition {
                from,
                start,
                duration,
            };
            self.transition = Some(transition);
            self.schedule_roll_after(transition.end());
        } else {
            self.transition = None;
            self.schedule_roll_after(start);
        }
    }

    /// Schedules the next roll one interval of the current weather after `settled_at`, the hours on
    /// the clock at which it has settled.
    fn schedule_roll_after(&mut self, settled_at: f64) {
        let interval = roll_interval_hours(
            self.table.climate_volatility(),
            self.current.traits().volatility_mult,
        );
        self.next_roll_at = settled_at + interval;
    }

    fn strike(&mut self, world_seconds: f32) {
        let previous = self.transition.map(|transition| transition.from);
        let requested_weather = requested_strike_weather(self.current, previous);
        for _ in 0..std::mem::take(&mut self.requested_strikes) {
            let strike = strike_at(requested_weather, 1.0, &mut self.strike_rng);
            self.strikes.push(strike);
        }
        let sides = sides(self.current, previous, self.progress());
        self.strike_sources.clear();
        self.strike_sources.extend(
            sides.filter_map(|(weather, side)| strike_source(weather.id(), weather.traits(), side)),
        );
        let strike =
            self.strike_clock
                .advance(&self.strike_sources, world_seconds, &mut self.strike_rng);
        self.strikes.extend(strike);
    }
}

/// The weather a strike asked for by hand takes its colour and thunder from: the first weather
/// that shows with a thunder sound, the current one before the one being left, as only a weather
/// with a thunder sound has thunder to play. Without one, the current weather's strike only flashes.
fn requested_strike_weather(current: TableWeather, previous: Option<TableWeather>) -> WeatherId {
    [Some(current), previous]
        .into_iter()
        .flatten()
        .find(|weather| weather.traits().has_thunder_sound)
        .unwrap_or(current)
        .id()
}

fn sides<W: Copy>(
    current: W,
    previous: Option<W>,
    progress: f32,
) -> impl Iterator<Item = (W, TransitionSide)> + use<W> {
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
    use super::super::test_support::{
        A, B, C, Place, Run, assert_same_place, busy_climate, calm, calm_run, storm_run, stormy,
        table,
    };
    use super::*;
    use crate::sim::commands::WeatherEdit;
    use crate::weather::FadeRange;

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
        assert_eq!(run.place(), (B, Some(A), 0.0, 9.0));
    }

    #[test]
    fn a_step_that_overshoots_a_roll_counts_the_overshoot_into_the_transition() {
        let mut run = calm_run(&[50]);
        run.hours(8.5);
        assert_same_place(run.place(), (B, Some(A), 0.5, 8.5));
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
    fn a_roll_that_keeps_the_weather_counts_its_interval_from_the_deadline_not_from_the_step() {
        let mut run = calm_run(&[10]);
        run.hours(8.5);
        assert_eq!(run.place(), (A, None, 1.0, 7.5));
    }

    #[test]
    fn the_next_roll_is_scheduled_by_the_weather_that_was_rolled() {
        let mut weathers = vec![calm(); 2];
        weathers[1].volatility_mult = 2.0;
        let mut run = Run::new(table(0, weathers, &[1, 1]), &[1], &[]);
        run.hours(8.0);
        assert_eq!(run.place(), (B, Some(A), 0.0, 5.0));
    }

    #[test]
    fn a_transition_advances_with_the_hours_and_settles_at_its_end() {
        let mut run = calm_run(&[50]);
        run.hours(8.0);
        run.hours(0.5);
        assert_eq!(run.snapshot().progress, 0.5);
        assert_eq!(run.snapshot().previous, Some(A));
        run.hours(0.5);
        assert_eq!(run.place(), (B, None, 1.0, 8.0));
    }

    #[test]
    fn a_long_step_settles_the_transition_in_one_go() {
        let mut run = calm_run(&[50]);
        run.hours(8.0);
        run.hours(5.0);
        assert_eq!(run.place(), (B, None, 1.0, 4.0));
    }

    #[test]
    fn the_incoming_weather_sets_the_length_of_the_transition() {
        let mut weathers = vec![calm(); 3];
        weathers[1].trans_delta_raw = 125;
        let mut run = Run::new(table(0, weathers, &[1, 1, 1]), &[1], &[]);
        run.jump_to(8.0);
        run.jump_to(8.625);
        assert_eq!((run.snapshot().current, run.snapshot().progress), (B, 0.5));
        run.jump_to(9.25);
        assert_eq!(run.place(), (B, None, 1.0, 8.0));
    }

    #[test]
    fn a_weather_whose_transition_byte_is_0_settles_at_once() {
        let mut weathers = vec![calm(); 3];
        weathers[2].trans_delta_raw = 0;
        let mut run = Run::new(table(0, weathers, &[1, 1, 1]), &[2], &[]);
        run.jump_to(8.0);
        assert_eq!(run.place(), (C, None, 1.0, 8.0));
    }

    #[test]
    fn the_roll_interval_counts_from_the_moment_the_weather_settles() {
        // A roll interval of 0.5 h, shorter than the 2 h that a transition takes.
        let mut weathers = vec![calm(); 3];
        for traits in &mut weathers {
            traits.volatility_mult = 4.0;
            traits.trans_delta_raw = 200;
        }
        let mut run = Run::new(table(255, weathers, &[1, 1, 1]), &[1, 2], &[]);
        run.jump_to(0.5);
        assert_eq!(run.place(), (B, Some(A), 0.0, 2.5));
        run.jump_to(2.5);
        assert_eq!(run.place(), (B, None, 1.0, 0.5));
        run.jump_to(2.9);
        assert_eq!(run.snapshot().previous, None);
        assert_eq!(run.snapshot().current, B);
        run.jump_to(3.0);
        assert_eq!(
            (run.snapshot().current, run.snapshot().previous),
            (C, Some(B))
        );
    }

    #[test]
    fn a_step_rolls_at_most_once() {
        let mut run = calm_run(&[50, 80]);
        run.hours(100.0);
        assert_eq!(run.snapshot().current, B);
        assert_eq!(run.snapshot().hours_to_next_roll, 0.0);
        run.seconds(0.0);
        assert_eq!(run.snapshot().current, C);
    }

    #[test]
    fn a_clock_that_went_back_moves_nothing_and_the_cycle_carries_on_from_there() {
        let mut run = calm_run(&[]);
        run.jump_to(5.0);
        let before = run.place();
        run.jump_to(2.0);
        assert_same_place(run.place(), before);
        run.hours(1.0);
        assert_eq!(run.snapshot().hours_to_next_roll, 2.0);
    }

    #[test]
    fn a_clock_that_went_back_during_a_transition_keeps_its_progress() {
        let mut run = calm_run(&[50]);
        run.hours(8.0);
        run.hours(0.5);
        let before = run.place();
        run.jump_to(3.0);
        assert_same_place(run.place(), before);
        run.hours(0.5);
        assert_eq!(run.snapshot().previous, None);
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
    fn the_state_at_a_clock_reading_does_not_depend_on_the_steps_that_led_there() {
        let place_after = |steps: u32, step_hours: f64| -> Place {
            let mut run = calm_run(&[50, 80]);
            for step in 1..=steps {
                run.jump_to(f64::from(step) * step_hours);
            }
            run.place()
        };
        let in_one_step = place_after(1, 16.0);
        assert_eq!(in_one_step.0, B);
        assert_same_place(place_after(16, 1.0), in_one_step);
        assert_same_place(place_after(1600, 0.01), in_one_step);
    }

    #[test]
    fn fine_steps_do_not_make_the_countdown_drift() {
        // A clock at a low time scale moves by a hundred thousandth of an hour per frame.
        let mut run = calm_run(&[50]);
        for step in 1..=850_000_u32 {
            run.jump_to(f64::from(step) * 1e-5);
        }
        let (current, previous, progress, hours_to_next_roll) = run.place();
        assert_eq!((current, previous), (B, Some(A)));
        assert!((progress - 0.5).abs() < 1e-5, "{progress}");
        assert!(
            (hours_to_next_roll - 8.5).abs() < 1e-5,
            "{hours_to_next_roll}"
        );
    }

    #[test]
    fn a_weather_without_a_thunder_sound_never_strikes() {
        let mut run = calm_run(&[]);
        run.seconds(1000.0);
        assert_eq!(run.snapshot().strikes, []);
    }

    #[test]
    fn a_storm_strikes_when_its_wait_is_over_and_the_strike_lasts_one_step() {
        // A wait of ln 2 expected strikes at 0.2 strikes per second is 3.47 s.
        let mut run = storm_run(&[0.5, 0.5]);
        run.seconds(3.0);
        assert_eq!(run.snapshot().strikes, []);
        run.seconds(0.5);
        let strikes = run.snapshot().strikes;
        assert_eq!(strikes.len(), 1);
        let strike = strikes[0];
        assert_eq!(strike.weather, A);
        assert_eq!(strike.distance_m, 1650.0);
        assert_eq!(strike.thunder_delay_s, 1650.0 / 343.0);
        let flash = strike.flash_intensity;
        assert!((flash - 0.675).abs() < 1e-6, "{flash}");
        run.seconds(0.0);
        assert_eq!(run.snapshot().strikes, []);
    }

    #[test]
    fn strikes_run_on_world_seconds_and_not_on_the_game_hours_of_the_clock() {
        let mut run = storm_run(&[]);
        run.hours(5.0);
        assert_eq!(run.snapshot().strikes, []);
    }

    #[test]
    fn a_fading_storm_strikes_less_often() {
        let mut run = storm_run(&[0.5, 0.5]);
        run.edit(WeatherEdit::Transition(B));
        run.edit(WeatherEdit::SetProgress(0.5));
        // Half the thunder is left, so the wait of 3.47 s at full strength is twice as long.
        run.seconds(6.9);
        assert_eq!(run.snapshot().strikes, []);
        run.seconds(0.1);
        let strikes = run.snapshot().strikes;
        assert_eq!(strikes.len(), 1);
        assert_eq!(strikes[0].weather, A);
        let flash = strikes[0].flash_intensity;
        assert!((flash - 0.3375).abs() < 1e-6, "{flash}");
    }

    #[test]
    fn during_a_transition_between_storms_a_strike_comes_from_either_side() {
        let strike_weather = |units: &[f32]| {
            let mut run = Run::new(table(0, vec![stormy(), stormy()], &[1, 1]), &[], units);
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
        let mut run = Run::new(table(0, vec![calm(), late], &[1, 1]), &[], &[]);
        run.edit(WeatherEdit::Transition(B));
        run.edit(WeatherEdit::SetProgress(0.25));
        run.seconds(100.0);
        assert_eq!(run.snapshot().strikes, []);
    }

    #[test]
    fn the_sides_of_a_transition_are_the_incoming_and_the_outgoing_weather() {
        let mut run = calm_run(&[50]);
        run.hours(8.0);
        run.hours(0.5);
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

    /// The weathers that a cycle of the busy climate goes through in 100 game hours, each counted
    /// as it comes in. The clock is stepped by `step_hours` and every step carries `world_seconds`
    /// of lightning time.
    fn weathers_of(seed: u64, step_hours: f64, world_seconds: f32) -> Vec<WeatherId> {
        let mut cycle = WeatherCycle::seeded(busy_climate(), seed, 0.0);
        let mut weathers = vec![cycle.snapshot().current];
        for step in 1..=(100.0 / step_hours).round() as u32 {
            cycle.advance(WeatherStep {
                clock_hours: f64::from(step) * step_hours,
                world_seconds,
            });
            let current = cycle.snapshot().current;
            if weathers.last() != Some(&current) {
                weathers.push(current);
            }
        }
        weathers
    }

    #[test]
    fn a_seed_brings_the_same_weathers_in_the_same_order_every_time() {
        let weathers = weathers_of(7, 0.01, 0.0);
        assert!(weathers.len() > 10, "{weathers:?}");
        assert_eq!(weathers_of(7, 0.01, 0.0), weathers);
    }

    #[test]
    fn another_seed_brings_other_weathers() {
        assert_ne!(weathers_of(7, 0.01, 0.0), weathers_of(8, 0.01, 0.0));
    }

    #[test]
    fn the_lightning_does_not_change_which_weathers_come() {
        assert_eq!(weathers_of(7, 0.01, 5.0), weathers_of(7, 0.01, 0.0));
    }

    #[test]
    fn the_steps_of_the_clock_do_not_change_which_weathers_come() {
        let weathers = weathers_of(7, 0.01, 1.0);
        assert_eq!(weathers_of(7, 0.025, 1.0), weathers);
        assert_eq!(weathers_of(7, 0.5, 1.0), weathers);
    }

    #[test]
    fn the_two_streams_of_a_seed_differ() {
        let mut cycle = WeatherCycle::seeded(busy_climate(), 7, 0.0);
        assert_ne!(
            cycle.roll_rng.below(usize::MAX),
            cycle.strike_rng.below(usize::MAX)
        );
    }
}
