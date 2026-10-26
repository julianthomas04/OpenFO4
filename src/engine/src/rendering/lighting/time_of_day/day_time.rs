#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum DayTimeError {
    #[error("hour {0} is outside [0, 24)")]
    HourOutOfRange(f32),
    #[error("{0} hours since the start is not a representable game time")]
    TotalHoursNotRepresentable(f64),
}

/// A moment of game time: the completed days and a finite hour in `[0, 24)`.
/// The single time input of the time-of-day maths.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DayTime {
    days_passed: u32,
    hour: f32,
}

impl DayTime {
    pub fn new(days_passed: u32, hour: f32) -> Result<Self, DayTimeError> {
        if !(0.0..24.0).contains(&hour) {
            return Err(DayTimeError::HourOutOfRange(hour));
        }
        Ok(Self { days_passed, hour })
    }

    /// For a clock whose hour is in range by construction: a stray value is pulled into
    /// `[0, 24)` (not-a-number reads as midnight) instead of failing.
    pub fn clamped(days_passed: u32, hour: f32) -> Self {
        let hour = if hour.is_nan() {
            0.0
        } else {
            hour.clamp(0.0, 24.0_f32.next_down())
        };
        Self { days_passed, hour }
    }

    /// From the hours elapsed since day 0, 00:00.
    pub fn from_total_hours(total_hours: f64) -> Result<Self, DayTimeError> {
        let not_representable = DayTimeError::TotalHoursNotRepresentable(total_hours);
        if !total_hours.is_finite() || total_hours < 0.0 {
            return Err(not_representable);
        }
        let days = (total_hours / 24.0).floor();
        if days > f64::from(u32::MAX) {
            return Err(not_representable);
        }
        let whole_days = days as u32;
        let hour = (total_hours - days * 24.0) as f32;
        if hour < 24.0 {
            return Self::new(whole_days, hour);
        }
        let next_day = whole_days.checked_add(1).ok_or(not_representable)?;
        Self::new(next_day, 0.0)
    }

    pub fn days_passed(&self) -> u32 {
        self.days_passed
    }

    pub fn hour(&self) -> f32 {
        self.hour
    }

    pub fn elapsed_days(&self) -> f64 {
        f64::from(self.days_passed) + f64::from(self.hour) / 24.0
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact binary fractions of an hour"
)]
mod tests {
    use super::*;

    #[test]
    fn a_clamped_time_keeps_an_hour_in_range_and_pulls_strays_into_it() {
        assert_eq!(DayTime::clamped(3, 13.5), DayTime::new(3, 13.5).unwrap());
        for (stray, expected) in [
            (-1.0, 0.0),
            (f32::NAN, 0.0),
            (24.0, 24.0_f32.next_down()),
            (f32::INFINITY, 24.0_f32.next_down()),
        ] {
            let time = DayTime::clamped(1, stray);
            assert_eq!((time.days_passed(), time.hour()), (1, expected), "{stray}");
        }
    }

    #[test]
    fn a_time_inside_the_day_is_kept() {
        let time = DayTime::new(7, 13.5).unwrap();
        assert_eq!((time.days_passed(), time.hour()), (7, 13.5));
        assert_eq!(time.elapsed_days(), 7.5625);
    }

    #[test]
    fn the_hour_must_be_finite_and_inside_the_day() {
        for bad in [
            -0.001,
            24.0,
            100.0,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ] {
            assert!(
                matches!(DayTime::new(0, bad), Err(DayTimeError::HourOutOfRange(_))),
                "{bad}"
            );
        }
        assert!(DayTime::new(0, 0.0).is_ok());
        assert!(DayTime::new(0, 23.999_998).is_ok());
    }

    #[test]
    fn total_hours_split_into_days_and_hour() {
        let time = DayTime::from_total_hours(24.0 * 3.0 + 6.25).unwrap();
        assert_eq!((time.days_passed(), time.hour()), (3, 6.25));
        let start = DayTime::from_total_hours(0.0).unwrap();
        assert_eq!((start.days_passed(), start.hour()), (0, 0.0));
    }

    #[test]
    fn a_total_a_hair_below_midnight_never_yields_hour_24() {
        let time = DayTime::from_total_hours(24.0 - 1e-9).unwrap();
        assert_eq!((time.days_passed(), time.hour()), (1, 0.0));
    }

    #[test]
    fn totals_that_are_negative_non_finite_or_too_large_are_rejected() {
        for bad in [-1.0, f64::NAN, f64::INFINITY, 24.0 * 5e9] {
            assert!(
                matches!(
                    DayTime::from_total_hours(bad),
                    Err(DayTimeError::TotalHoursNotRepresentable(_))
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_last_representable_day_is_accepted_and_the_one_after_is_not() {
        let last = DayTime::from_total_hours(24.0 * f64::from(u32::MAX) + 12.0).unwrap();
        assert_eq!(last.days_passed(), u32::MAX);
        assert!(DayTime::from_total_hours(24.0 * (f64::from(u32::MAX) + 1.0)).is_err());
    }
}
