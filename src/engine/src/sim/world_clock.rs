//! The world's game time: hour, calendar date, days passed and the speed they run at. The
//! simulation owns the clock and the frame loop advances it; everything else reads it.

use thiserror::Error;

/// The game's year value counts from this year: a `GameYear` global of 287 is the year 2287.
pub const GAME_YEAR_BASE: u16 = 2000;
/// Upper bound of the time scale, in game minutes per real minute. Far above any useful speed; it
/// keeps one advance from spanning an absurd number of days.
pub const MAX_TIME_SCALE: f32 = 1_000_000.0;

const MINUTES_PER_HOUR: f64 = 60.0;
const HOURS_PER_DAY: f32 = 24.0;
const MINUTES_PER_DAY: f64 = 24.0 * MINUTES_PER_HOUR;
const SECONDS_PER_MINUTE: f64 = 60.0;
const MONTHS_PER_YEAR: u8 = 12;
/// Real month lengths; the game has no leap years.
const DAYS_IN_MONTH: [u8; MONTHS_PER_YEAR as usize] =
    [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameClock {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub minute_of_day: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Error)]
pub enum ClockError {
    #[error("hour {0} is outside 0..=24")]
    HourOutOfRange(f32),
    #[error("time scale {0} is not a finite number in (0, {MAX_TIME_SCALE}]")]
    InvalidTimeScale(f32),
    #[error("{year}-{month}-{day} is not a calendar date")]
    InvalidDate { year: u16, month: u8, day: u8 },
}

/// Where a world clock begins, read from the game's globals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClockStart {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: f32,
    pub days_passed: u32,
    pub time_scale: f32,
}

#[cfg(test)]
impl ClockStart {
    /// A valid start for tests: 11:00 on 23 October 2287, one day passed, time scale 20.
    pub fn fixture() -> Self {
        Self {
            year: GAME_YEAR_BASE + 287,
            month: 10,
            day: 23,
            hour: 11.0,
            days_passed: 1,
            time_scale: 20.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorldClock {
    year: u16,
    month: u8,
    day: u8,
    minute_of_day: f64,
    days_passed: u32,
    time_scale: f32,
    start_time_scale: f32,
    paused: bool,
}

impl WorldClock {
    #[cfg(test)]
    pub fn fixture() -> Self {
        Self::new(ClockStart::fixture()).expect("the fixture start is valid")
    }

    pub fn new(start: ClockStart) -> Result<Self, ClockError> {
        validate_date(start.year, start.month, start.day)?;
        validate_hour(start.hour)?;
        validate_time_scale(start.time_scale)?;
        Ok(Self {
            year: start.year,
            month: start.month,
            day: start.day,
            minute_of_day: minute_of_day(start.hour),
            days_passed: start.days_passed,
            time_scale: start.time_scale,
            start_time_scale: start.time_scale,
            paused: false,
        })
    }

    /// Runs the clock for `world_dt_seconds` of world time at the current time scale, rolling the
    /// date over at midnight. Does nothing while paused or for a non-finite or non-positive step.
    /// The caller bounds the step (see `clock_dt`), so a stalled frame cannot skip a long stretch.
    pub fn advance(&mut self, world_dt_seconds: f32) {
        if self.paused || !world_dt_seconds.is_finite() || world_dt_seconds <= 0.0 {
            return;
        }
        let game_minutes =
            f64::from(world_dt_seconds) * f64::from(self.time_scale) / SECONDS_PER_MINUTE;
        let total = self.minute_of_day + game_minutes;
        let remainder = total % MINUTES_PER_DAY;
        let midnights = ((total - remainder) / MINUTES_PER_DAY).round() as u32;
        self.minute_of_day = remainder;
        for _ in 0..midnights {
            self.start_next_day();
        }
    }

    /// Sets the hour of the day (24 is midnight). The date and the days passed stay as they are.
    pub fn set_hour(&mut self, hour: f32) -> Result<(), ClockError> {
        validate_hour(hour)?;
        self.minute_of_day = minute_of_day(hour);
        Ok(())
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn set_time_scale(&mut self, time_scale: f32) -> Result<(), ClockError> {
        validate_time_scale(time_scale)?;
        self.time_scale = time_scale;
        Ok(())
    }

    /// The hour of the day in `[0, 24)` with the minutes as a fraction: the game's `GameHour`.
    pub fn hour(&self) -> f32 {
        ((self.minute_of_day / MINUTES_PER_HOUR) as f32).min(HOURS_PER_DAY.next_down())
    }

    pub fn minute_of_day(&self) -> f64 {
        self.minute_of_day
    }

    /// The game hours since the day count began: the days passed times 24 plus the hour of the
    /// day. It rises with the clock and without a break at midnight, but moves by the difference
    /// when the hour is set.
    pub fn total_hours(&self) -> f64 {
        f64::from(self.days_passed) * f64::from(HOURS_PER_DAY)
            + self.minute_of_day / MINUTES_PER_HOUR
    }

    /// The calendar date and the whole minute of the day, as the Pip-Boy shows them.
    pub fn date(&self) -> GameClock {
        GameClock {
            year: self.year,
            month: self.month,
            day: self.day,
            minute_of_day: self.minute_of_day as u16,
        }
    }

    pub fn days_passed(&self) -> u32 {
        self.days_passed
    }

    /// Game minutes per real minute.
    pub fn time_scale(&self) -> f32 {
        self.time_scale
    }

    /// The time scale the clock started with: the game's `TimeScale` as loaded.
    pub fn start_time_scale(&self) -> f32 {
        self.start_time_scale
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    fn start_next_day(&mut self) {
        self.days_passed = self.days_passed.saturating_add(1);
        if self.day < days_in_month(self.month) {
            self.day += 1;
            return;
        }
        self.day = 1;
        if self.month < MONTHS_PER_YEAR {
            self.month += 1;
        } else {
            self.month = 1;
            self.year = self.year.saturating_add(1);
        }
    }
}

fn days_in_month(month: u8) -> u8 {
    DAYS_IN_MONTH[usize::from(month) - 1]
}

fn minute_of_day(hour: f32) -> f64 {
    (f64::from(hour) * MINUTES_PER_HOUR) % MINUTES_PER_DAY
}

fn validate_date(year: u16, month: u8, day: u8) -> Result<(), ClockError> {
    let in_calendar =
        (1..=MONTHS_PER_YEAR).contains(&month) && (1..=days_in_month(month)).contains(&day);
    if in_calendar {
        Ok(())
    } else {
        Err(ClockError::InvalidDate { year, month, day })
    }
}

fn validate_hour(hour: f32) -> Result<(), ClockError> {
    if (0.0..=HOURS_PER_DAY).contains(&hour) {
        Ok(())
    } else {
        Err(ClockError::HourOutOfRange(hour))
    }
}

fn validate_time_scale(time_scale: f32) -> Result<(), ClockError> {
    if time_scale > 0.0 && time_scale <= MAX_TIME_SCALE {
        Ok(())
    } else {
        Err(ClockError::InvalidTimeScale(time_scale))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL_SECONDS_PER_GAME_DAY_AT_SCALE_20: f32 = 72.0 * 60.0;

    fn clock_at(year: u16, month: u8, day: u8, hour: f32) -> WorldClock {
        WorldClock::new(ClockStart {
            year,
            month,
            day,
            hour,
            ..ClockStart::fixture()
        })
        .unwrap()
    }

    fn ymd(clock: &WorldClock) -> (u16, u8, u8) {
        let date = clock.date();
        (date.year, date.month, date.day)
    }

    #[test]
    fn a_new_clock_runs_from_its_start() {
        let clock = WorldClock::fixture();
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
        assert!(!clock.paused());
    }

    #[test]
    fn the_start_time_scale_survives_later_time_scale_changes() {
        let mut clock = WorldClock::new(ClockStart {
            time_scale: 6.0,
            ..ClockStart::fixture()
        })
        .unwrap();
        clock.set_time_scale(500.0).unwrap();
        assert_eq!(clock.time_scale(), 500.0);
        assert_eq!(clock.start_time_scale(), 6.0);
    }

    #[test]
    fn a_start_hour_with_minutes_keeps_its_fraction() {
        let clock = clock_at(2287, 10, 23, 9.5);
        assert_eq!(clock.date().minute_of_day, 9 * 60 + 30);
        assert!((clock.hour() - 9.5).abs() < 1e-6);
    }

    #[test]
    fn a_start_with_an_impossible_date_hour_or_time_scale_is_rejected() {
        let rejected = |change: fn(&mut ClockStart)| {
            let mut start = ClockStart::fixture();
            change(&mut start);
            WorldClock::new(start).unwrap_err()
        };
        assert!(matches!(
            rejected(|s| s.month = 13),
            ClockError::InvalidDate { month: 13, .. }
        ));
        assert!(matches!(
            rejected(|s| s.month = 0),
            ClockError::InvalidDate { month: 0, .. }
        ));
        assert!(matches!(
            rejected(|s| s.day = 0),
            ClockError::InvalidDate { day: 0, .. }
        ));
        assert!(matches!(
            rejected(|s| {
                s.month = 2;
                s.day = 29;
            }),
            ClockError::InvalidDate {
                month: 2,
                day: 29,
                ..
            }
        ));
        assert_eq!(
            rejected(|s| s.hour = 24.5),
            ClockError::HourOutOfRange(24.5)
        );
        assert_eq!(
            rejected(|s| s.time_scale = 0.0),
            ClockError::InvalidTimeScale(0.0)
        );
    }

    #[test]
    fn at_scale_20_one_real_second_is_a_third_of_a_game_minute() {
        let mut clock = clock_at(2287, 10, 23, 11.0);
        clock.advance(3.0);
        assert_eq!(clock.date().minute_of_day, 11 * 60 + 1);
        clock.advance(57.0);
        assert_eq!(clock.date().minute_of_day, 11 * 60 + 20);
    }

    #[test]
    fn at_scale_20_a_game_day_lasts_72_real_minutes() {
        let mut clock = clock_at(2287, 10, 23, 11.0);
        clock.advance(REAL_SECONDS_PER_GAME_DAY_AT_SCALE_20);
        assert_eq!(ymd(&clock), (2287, 10, 24));
        assert!((clock.hour() - 11.0).abs() < 1e-4);
        assert_eq!(clock.days_passed(), 2);
    }

    #[test]
    fn crossing_midnight_starts_the_next_date_at_the_leftover_time() {
        let mut clock = clock_at(2287, 10, 23, 23.5);
        clock.advance(90.0);
        assert_eq!(ymd(&clock), (2287, 10, 24));
        assert_eq!(clock.date().minute_of_day, 0);
        assert_eq!(clock.days_passed(), 2);
        clock.advance(90.0);
        assert_eq!(clock.date().minute_of_day, 30);
    }

    #[test]
    fn a_long_step_counts_every_midnight_it_crosses() {
        let mut clock = clock_at(2287, 10, 23, 11.0);
        clock.advance(3.0 * REAL_SECONDS_PER_GAME_DAY_AT_SCALE_20);
        assert_eq!(ymd(&clock), (2287, 10, 26));
        assert_eq!(clock.days_passed(), 4);
    }

    #[test]
    fn the_last_day_of_a_month_rolls_into_the_first_of_the_next() {
        for (index, last_day) in DAYS_IN_MONTH.iter().copied().enumerate().take(11) {
            let month = index as u8 + 1;
            let mut clock = clock_at(2287, month, last_day, 23.5);
            clock.advance(90.0);
            assert_eq!(ymd(&clock), (2287, month + 1, 1), "after month {month}");
        }
    }

    #[test]
    fn the_last_day_of_the_year_rolls_into_january_of_the_next_year() {
        let mut clock = clock_at(2287, 12, 31, 23.5);
        clock.advance(90.0);
        assert_eq!(ymd(&clock), (2288, 1, 1));
    }

    #[test]
    fn february_has_28_days_even_in_a_gregorian_leap_year() {
        let mut clock = clock_at(2288, 2, 28, 23.5);
        clock.advance(90.0);
        assert_eq!(ymd(&clock), (2288, 3, 1));
    }

    #[test]
    fn setting_the_hour_keeps_the_date_and_the_days_passed() {
        let mut clock = clock_at(2287, 10, 23, 11.0);
        clock.set_hour(3.25).unwrap();
        assert_eq!(ymd(&clock), (2287, 10, 23));
        assert_eq!(clock.days_passed(), 1);
        assert_eq!(clock.date().minute_of_day, 3 * 60 + 15);
        clock.set_hour(23.0).unwrap();
        assert_eq!(ymd(&clock), (2287, 10, 23));
        assert_eq!(clock.days_passed(), 1);
    }

    #[test]
    fn setting_the_hour_to_24_is_midnight_of_the_same_date() {
        let mut clock = clock_at(2287, 10, 23, 11.0);
        clock.set_hour(24.0).unwrap();
        assert_eq!(ymd(&clock), (2287, 10, 23));
        assert_eq!(clock.date().minute_of_day, 0);
        assert_eq!(clock.hour(), 0.0);
    }

    #[test]
    fn a_rejected_hour_leaves_the_clock_unchanged() {
        let mut clock = clock_at(2287, 10, 23, 11.0);
        let before = clock.clone();
        for bad in [-0.1, 24.1, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let error = clock.set_hour(bad).unwrap_err();
            assert!(matches!(error, ClockError::HourOutOfRange(_)), "{bad}");
            assert_eq!(clock, before, "{bad}");
        }
    }

    #[test]
    fn a_paused_clock_does_not_advance_until_resumed() {
        let mut clock = clock_at(2287, 10, 23, 11.0);
        clock.set_paused(true);
        clock.advance(REAL_SECONDS_PER_GAME_DAY_AT_SCALE_20);
        assert_eq!(clock.date().minute_of_day, 11 * 60);
        assert_eq!(clock.days_passed(), 1);
        clock.set_paused(false);
        clock.advance(3.0);
        assert_eq!(clock.date().minute_of_day, 11 * 60 + 1);
    }

    #[test]
    fn a_pause_does_not_block_setting_the_hour() {
        let mut clock = clock_at(2287, 10, 23, 11.0);
        clock.set_paused(true);
        clock.set_hour(5.0).unwrap();
        assert_eq!(clock.date().minute_of_day, 5 * 60);
    }

    #[test]
    fn a_slower_time_scale_slows_the_clock_and_a_faster_one_speeds_it_up() {
        let mut slow = clock_at(2287, 10, 23, 11.0);
        slow.set_time_scale(1.0).unwrap();
        slow.advance(60.0);
        assert_eq!(slow.date().minute_of_day, 11 * 60 + 1);

        let mut fast = clock_at(2287, 10, 23, 11.0);
        fast.set_time_scale(2000.0).unwrap();
        fast.advance(3.0);
        assert_eq!(fast.date().minute_of_day, 11 * 60 + 100);
    }

    #[test]
    fn the_time_scale_accepts_the_range_up_to_the_maximum_and_nothing_else() {
        let mut clock = WorldClock::fixture();
        for good in [0.001, 1.0, 20.0, 500.0, MAX_TIME_SCALE] {
            clock.set_time_scale(good).unwrap();
            assert_eq!(clock.time_scale(), good);
        }
        for bad in [0.0, -1.0, MAX_TIME_SCALE * 2.0, f32::NAN, f32::INFINITY] {
            clock.set_time_scale(1.5).unwrap();
            let error = clock.set_time_scale(bad).unwrap_err();
            assert!(matches!(error, ClockError::InvalidTimeScale(_)), "{bad}");
            assert_eq!(clock.time_scale(), 1.5, "{bad}");
        }
    }

    #[test]
    fn a_non_positive_or_non_finite_step_does_not_move_the_clock() {
        let mut clock = clock_at(2287, 10, 23, 11.0);
        let before = clock.clone();
        for bad in [0.0, -5.0, f32::NAN, f32::INFINITY] {
            clock.advance(bad);
            assert_eq!(clock, before, "{bad}");
        }
    }

    #[test]
    fn the_total_hours_are_the_days_passed_and_the_hour_of_the_day() {
        let clock = clock_at(2287, 10, 23, 11.5);
        assert_eq!(clock.total_hours(), 24.0 + 11.5);
    }

    #[test]
    fn the_total_hours_rise_without_a_break_across_midnight() {
        let mut clock = clock_at(2287, 10, 23, 23.5);
        let before = clock.total_hours();
        clock.advance(90.0);
        assert!((clock.total_hours() - (before + 0.5)).abs() < 1e-9);
    }

    #[test]
    fn setting_the_hour_moves_the_total_hours_by_the_difference() {
        let mut clock = clock_at(2287, 10, 23, 11.0);
        let before = clock.total_hours();
        clock.set_hour(3.0).unwrap();
        assert_eq!(clock.total_hours(), before - 8.0);
        clock.set_hour(20.0).unwrap();
        assert_eq!(clock.total_hours(), before + 9.0);
    }

    #[test]
    fn the_hour_stays_below_24_just_before_midnight() {
        let clock = WorldClock {
            minute_of_day: MINUTES_PER_DAY - 1e-9,
            ..WorldClock::fixture()
        };
        assert!(clock.hour() < 24.0);
        assert!(clock.date().minute_of_day < 24 * 60);
    }
}
