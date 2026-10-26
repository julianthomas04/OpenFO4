//! The edits a developer makes to a running weather cycle.

use thiserror::Error;

use super::WeatherCycle;
use crate::random::RandomSource;
use crate::sim::commands::WeatherEdit;
use crate::weather::WeatherId;

#[derive(Clone, Copy, Debug, PartialEq, Error)]
pub enum WeatherEditError {
    #[error("{0:?} is not a weather of the library")]
    UnknownWeather(WeatherId),
    #[error("the weather is {0:?} already")]
    AlreadyCurrent(WeatherId),
    #[error("progress {0} is not between 0 and 1")]
    ProgressOutOfRange(f32),
    #[error("no transition is running")]
    NotTransitioning,
}

impl<R: RandomSource> WeatherCycle<R> {
    /// Applies the edit, or rejects an invalid one and leaves the cycle as it was. A change of
    /// the weather reschedules the next roll.
    pub fn apply(&mut self, edit: WeatherEdit) -> Result<(), WeatherEditError> {
        match edit {
            WeatherEdit::Force(weather) => self.force(weather),
            WeatherEdit::Transition(weather) => self.start_transition(weather),
            WeatherEdit::SetProgress(progress) => self.set_progress(progress),
            WeatherEdit::TriggerLightning => {
                self.requested_strikes = self.requested_strikes.saturating_add(1);
                Ok(())
            }
        }
    }

    fn require_known(&self, weather: WeatherId) -> Result<(), WeatherEditError> {
        self.table
            .traits(weather)
            .map(|_| ())
            .ok_or(WeatherEditError::UnknownWeather(weather))
    }

    fn force(&mut self, weather: WeatherId) -> Result<(), WeatherEditError> {
        self.require_known(weather)?;
        self.settle();
        self.current = weather;
        self.restart_roll_clock();
        Ok(())
    }

    /// Leaves the current weather for `weather`, also while another transition is running; the
    /// weather that was on its way out is dropped.
    fn start_transition(&mut self, weather: WeatherId) -> Result<(), WeatherEditError> {
        self.require_known(weather)?;
        if weather == self.current {
            return Err(WeatherEditError::AlreadyCurrent(weather));
        }
        self.begin_transition(weather);
        Ok(())
    }

    /// A progress of 1 ends the transition.
    fn set_progress(&mut self, progress: f32) -> Result<(), WeatherEditError> {
        if !(0.0..=1.0).contains(&progress) {
            return Err(WeatherEditError::ProgressOutOfRange(progress));
        }
        if self.previous.is_none() {
            return Err(WeatherEditError::NotTransitioning);
        }
        self.progress = progress;
        if progress >= 1.0 {
            self.settle();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{Run, calm_run, storm_run};
    use super::*;
    use crate::sim::weather::test_support::{A, B, C, ScriptedRandom, calm, table};

    fn rejected(run: &mut Run, edit: WeatherEdit) -> WeatherEditError {
        let before = run.place();
        let error = run.cycle.apply(edit).unwrap_err();
        assert_eq!(run.place(), before, "{edit:?} changed the cycle");
        error
    }

    fn unknown() -> WeatherId {
        WeatherId::new(99)
    }

    /// Two calm weathers and a script of the given unit draws.
    fn calm_run_drawing(units: &[f32]) -> Run {
        Run::new(
            table(0, vec![calm(); 2], &[1, 1]),
            ScriptedRandom::new(&[], units),
        )
    }

    #[test]
    fn forcing_a_weather_settles_on_it_at_once() {
        let mut run = calm_run(&[]);
        run.edit(WeatherEdit::Force(C));
        assert_eq!(run.place(), (C, None, 1.0, 8.0));
    }

    #[test]
    fn forcing_a_weather_ends_a_transition() {
        let mut run = calm_run(&[50]);
        run.hours(8.0);
        run.hours(1.0);
        run.edit(WeatherEdit::Force(C));
        assert_eq!(run.place(), (C, None, 1.0, 8.0));
    }

    #[test]
    fn forcing_a_weather_cancels_a_roll_that_waits_for_the_transition_to_end() {
        let mut weathers = vec![calm(); 3];
        for traits in &mut weathers {
            traits.volatility_mult = 4.0;
            traits.trans_delta_raw = 125;
        }
        let mut run = Run::new(
            table(255, weathers, &[1, 1, 1]),
            ScriptedRandom::new(&[1], &[]),
        );
        run.hours(0.5);
        run.hours(1.0);
        assert_eq!(run.snapshot().hours_to_next_roll, 0.0);
        run.edit(WeatherEdit::Force(C));
        run.seconds(0.0);
        assert_eq!(run.place(), (C, None, 1.0, 0.5));
    }

    #[test]
    fn forcing_the_current_weather_is_fine_and_reschedules_the_roll() {
        let mut run = calm_run(&[]);
        run.hours(3.0);
        run.edit(WeatherEdit::Force(A));
        assert_eq!(run.place(), (A, None, 1.0, 8.0));
    }

    #[test]
    fn forcing_a_weather_that_is_not_in_the_library_is_rejected() {
        let mut run = calm_run(&[]);
        run.hours(3.0);
        assert_eq!(
            rejected(&mut run, WeatherEdit::Force(unknown())),
            WeatherEditError::UnknownWeather(unknown())
        );
    }

    #[test]
    fn a_weather_that_is_not_in_the_climate_can_be_forced() {
        let mut run = Run::new(table(0, vec![calm(); 3], &[1, 1]), ScriptedRandom::silent());
        run.edit(WeatherEdit::Force(C));
        assert_eq!(run.snapshot().current, C);
    }

    #[test]
    fn a_transition_leaves_the_current_weather_at_zero_progress() {
        let mut run = calm_run(&[]);
        run.hours(3.0);
        run.edit(WeatherEdit::Transition(C));
        assert_eq!(run.place(), (C, Some(A), 0.0, 8.0));
    }

    #[test]
    fn a_transition_during_a_transition_leaves_the_weather_it_had_reached() {
        let mut run = calm_run(&[50]);
        run.hours(8.0);
        run.hours(0.78125);
        run.edit(WeatherEdit::Transition(C));
        assert_eq!(run.place(), (C, Some(B), 0.0, 8.0));
    }

    #[test]
    fn a_transition_to_the_current_weather_is_rejected() {
        let mut run = calm_run(&[]);
        assert_eq!(
            rejected(&mut run, WeatherEdit::Transition(A)),
            WeatherEditError::AlreadyCurrent(A)
        );
    }

    #[test]
    fn a_transition_to_a_weather_that_is_not_in_the_library_is_rejected() {
        let mut run = calm_run(&[]);
        assert_eq!(
            rejected(&mut run, WeatherEdit::Transition(unknown())),
            WeatherEditError::UnknownWeather(unknown())
        );
    }

    #[test]
    fn the_progress_of_a_running_transition_can_be_set() {
        let mut run = calm_run(&[]);
        run.edit(WeatherEdit::Transition(B));
        for progress in [0.0, 0.25, 0.999] {
            run.edit(WeatherEdit::SetProgress(progress));
            assert_eq!(run.snapshot().progress, progress);
            assert_eq!(run.snapshot().previous, Some(A));
        }
    }

    #[test]
    fn a_progress_of_one_ends_the_transition() {
        let mut run = calm_run(&[]);
        run.edit(WeatherEdit::Transition(B));
        run.edit(WeatherEdit::SetProgress(1.0));
        assert_eq!(run.snapshot().previous, None);
        assert_eq!(run.snapshot().progress, 1.0);
    }

    #[test]
    fn a_progress_that_is_not_between_zero_and_one_is_rejected() {
        let mut run = calm_run(&[]);
        run.edit(WeatherEdit::Transition(B));
        for progress in [-0.1, 1.1, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let error = rejected(&mut run, WeatherEdit::SetProgress(progress));
            assert!(
                matches!(error, WeatherEditError::ProgressOutOfRange(_)),
                "{progress}"
            );
        }
    }

    #[test]
    fn a_progress_cannot_be_set_while_settled() {
        let mut run = calm_run(&[]);
        assert_eq!(
            rejected(&mut run, WeatherEdit::SetProgress(0.5)),
            WeatherEditError::NotTransitioning
        );
    }

    #[test]
    fn a_progress_out_of_range_is_reported_before_the_missing_transition() {
        let mut run = calm_run(&[]);
        assert!(matches!(
            rejected(&mut run, WeatherEdit::SetProgress(2.0)),
            WeatherEditError::ProgressOutOfRange(_)
        ));
    }

    #[test]
    fn a_triggered_lightning_strikes_on_the_next_step_even_without_thunder() {
        let mut run = calm_run_drawing(&[0.5]);
        run.edit(WeatherEdit::TriggerLightning);
        assert_eq!(run.snapshot().strikes, []);
        run.seconds(0.0);
        let strikes = run.snapshot().strikes;
        assert_eq!(strikes.len(), 1);
        assert_eq!(strikes[0].weather, A);
        assert_eq!(strikes[0].distance_m, 1650.0);
        run.seconds(0.0);
        assert_eq!(run.snapshot().strikes, []);
    }

    #[test]
    fn every_trigger_of_a_frame_strikes() {
        let mut run = calm_run_drawing(&[0.0, 0.5]);
        run.edit(WeatherEdit::TriggerLightning);
        run.edit(WeatherEdit::TriggerLightning);
        run.seconds(0.0);
        let distances: Vec<f32> = run
            .snapshot()
            .strikes
            .iter()
            .map(|strike| strike.distance_m)
            .collect();
        assert_eq!(distances, [300.0, 1650.0]);
    }

    #[test]
    fn a_triggered_lightning_leaves_the_wait_of_the_storm_alone() {
        // The storm waits for 3.47 s; the draws are that wait, the triggered strike's distance and
        // the storm's own strike distance.
        let mut run = storm_run(&[0.5, 0.0, 0.25]);
        run.seconds(1.0);
        run.edit(WeatherEdit::TriggerLightning);
        run.seconds(0.5);
        assert_eq!(run.snapshot().strikes.len(), 1);
        assert_eq!(run.snapshot().strikes[0].distance_m, 300.0);
        run.seconds(2.0);
        assert_eq!(run.snapshot().strikes.len(), 1);
        assert_eq!(run.snapshot().strikes[0].distance_m, 975.0);
    }
}
