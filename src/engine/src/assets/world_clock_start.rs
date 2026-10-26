//! The world clock's starting point, read from the game's globals (`GameHour`, `GameDay`,
//! `GameMonth`, `GameYear`, `GameDaysPassed`, `TimeScale`).

use anyhow::Context;

use crate::assets::db::DbClient;
use crate::sim::world_clock::{ClockStart, GAME_YEAR_BASE, WorldClock};

const GAME_HOUR: &str = "GameHour";
const GAME_DAY: &str = "GameDay";
const GAME_MONTH: &str = "GameMonth";
const GAME_YEAR: &str = "GameYear";
const GAME_DAYS_PASSED: &str = "GameDaysPassed";
const TIME_SCALE: &str = "TimeScale";

/// A database error, a missing global or a global with an impossible value is an error.
pub fn load_world_clock(db: &DbClient) -> anyhow::Result<WorldClock> {
    let start = clock_start_from(|editor_id| db.get_global(editor_id))?;
    WorldClock::new(start).context("the game's clock globals do not describe a valid start")
}

fn clock_start_from(
    lookup: impl Fn(&str) -> anyhow::Result<Option<f32>>,
) -> anyhow::Result<ClockStart> {
    let value = |editor_id: &str| -> anyhow::Result<f32> {
        lookup(editor_id)
            .with_context(|| format!("reading global {editor_id}"))?
            .with_context(|| format!("global {editor_id} is missing from the database"))
    };
    let count =
        |editor_id: &str| -> anyhow::Result<u32> { whole_number(editor_id, value(editor_id)?) };
    let days_passed = floored_count(GAME_DAYS_PASSED, value(GAME_DAYS_PASSED)?)?;
    let years_since_base = count(GAME_YEAR)?;
    Ok(ClockStart {
        year: GAME_YEAR_BASE
            .checked_add(narrow(GAME_YEAR, years_since_base)?)
            .with_context(|| format!("global {GAME_YEAR} is out of range: {years_since_base}"))?,
        month: narrow(GAME_MONTH, count(GAME_MONTH)?)?,
        day: narrow(GAME_DAY, count(GAME_DAY)?)?,
        hour: value(GAME_HOUR)?,
        days_passed,
        time_scale: value(TIME_SCALE)?,
    })
}

fn whole_number(editor_id: &str, value: f32) -> anyhow::Result<u32> {
    if value.is_finite() && value >= 0.0 && value.fract() == 0.0 && value <= u32::MAX as f32 {
        Ok(value as u32)
    } else {
        anyhow::bail!("global {editor_id} is {value}, not a whole number of zero or more")
    }
}

/// `GameDaysPassed` is a float global that counts whole days plus the fraction of today; the clock
/// keeps the whole days.
fn floored_count(editor_id: &str, value: f32) -> anyhow::Result<u32> {
    if value.is_finite() && value >= 0.0 {
        Ok(value.floor() as u32)
    } else {
        anyhow::bail!("global {editor_id} is {value}, not a number of zero or more")
    }
}

fn narrow<T: TryFrom<u32>>(editor_id: &str, value: u32) -> anyhow::Result<T> {
    T::try_from(value).map_err(|_| anyhow::anyhow!("global {editor_id} is out of range: {value}"))
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "values are copied through unchanged; no arithmetic is involved"
)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::sim::world_clock::GameClock;

    fn vanilla() -> HashMap<&'static str, f32> {
        HashMap::from([
            (GAME_HOUR, 11.0),
            (GAME_DAY, 23.0),
            (GAME_MONTH, 10.0),
            (GAME_YEAR, 287.0),
            (GAME_DAYS_PASSED, 1.0),
            (TIME_SCALE, 20.0),
        ])
    }

    fn start_from(globals: &HashMap<&str, f32>) -> anyhow::Result<ClockStart> {
        clock_start_from(|editor_id| Ok(globals.get(editor_id).copied()))
    }

    #[test]
    fn the_vanilla_globals_start_the_clock_at_11_am_on_23_october_2287() {
        let clock = WorldClock::new(start_from(&vanilla()).unwrap()).unwrap();
        assert_eq!(
            clock.date(),
            GameClock {
                year: 2287,
                month: 10,
                day: 23,
                minute_of_day: 11 * 60
            }
        );
        assert_eq!(clock.days_passed(), 1);
        assert_eq!(clock.time_scale(), 20.0);
    }

    #[test]
    fn modded_globals_move_the_start() {
        let mut globals = vanilla();
        globals.extend([
            (GAME_HOUR, 6.5),
            (GAME_DAY, 1.0),
            (GAME_MONTH, 3.0),
            (GAME_YEAR, 290.0),
            (GAME_DAYS_PASSED, 40.0),
            (TIME_SCALE, 10.0),
        ]);
        assert_eq!(
            start_from(&globals).unwrap(),
            ClockStart {
                year: 2290,
                month: 3,
                day: 1,
                hour: 6.5,
                days_passed: 40,
                time_scale: 10.0
            }
        );
    }

    #[test]
    fn a_fractional_days_passed_global_is_floored_to_whole_days() {
        for (value, expected) in [(1.0, 1), (1.5, 1), (41.99, 41), (0.25, 0)] {
            let mut globals = vanilla();
            globals.insert(GAME_DAYS_PASSED, value);
            let clock = WorldClock::new(start_from(&globals).unwrap()).unwrap();
            assert_eq!(clock.days_passed(), expected, "{value}");
        }
    }

    #[test]
    fn a_missing_global_is_an_error_naming_it() {
        for editor_id in [
            GAME_HOUR,
            GAME_DAY,
            GAME_MONTH,
            GAME_YEAR,
            GAME_DAYS_PASSED,
            TIME_SCALE,
        ] {
            let mut globals = vanilla();
            globals.remove(editor_id);
            let message = format!("{:#}", start_from(&globals).unwrap_err());
            assert!(
                message.contains(editor_id) && message.contains("missing"),
                "{message}"
            );
        }
    }

    #[test]
    fn a_failing_lookup_is_an_error_naming_the_global() {
        let error = clock_start_from(|editor_id| {
            if editor_id == GAME_DAY {
                anyhow::bail!("disk on fire")
            }
            Ok(vanilla().get(editor_id).copied())
        })
        .unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains(GAME_DAY) && message.contains("disk on fire"),
            "{message}"
        );
    }

    #[test]
    fn a_global_that_is_not_a_whole_calendar_number_is_an_error() {
        for (editor_id, bad) in [
            (GAME_DAY, 23.5),
            (GAME_MONTH, -1.0),
            (GAME_YEAR, f32::NAN),
            (GAME_DAYS_PASSED, f32::INFINITY),
            (GAME_DAYS_PASSED, -0.5),
            (GAME_MONTH, 300.0),
            (GAME_YEAR, 70_000.0),
        ] {
            let mut globals = vanilla();
            globals.insert(editor_id, bad);
            let error = start_from(&globals).unwrap_err();
            assert!(
                format!("{error:#}").contains(editor_id),
                "{editor_id}={bad}"
            );
        }
    }

    #[test]
    fn globals_that_do_not_form_a_valid_start_are_rejected_by_the_clock() {
        for (editor_id, bad) in [
            (GAME_MONTH, 13.0),
            (GAME_HOUR, 25.0),
            (TIME_SCALE, 0.0),
            (GAME_DAY, 0.0),
        ] {
            let mut globals = vanilla();
            globals.insert(editor_id, bad);
            assert!(
                WorldClock::new(start_from(&globals).unwrap()).is_err(),
                "{editor_id}={bad}"
            );
        }
    }
}
