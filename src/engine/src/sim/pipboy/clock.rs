//! In-game calendar time shown in the DATA and MAP bottom bar.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameClock {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub minute_of_day: u16,
}

const MINUTES_PER_HOUR: f32 = 60.0;

impl GameClock {
    /// The hour of the day with the minutes as a fraction, as the game's `GameHour` reads.
    pub fn hour(&self) -> f32 {
        f32::from(self.minute_of_day) / MINUTES_PER_HOUR
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hour_carries_the_minutes_as_a_fraction() {
        let clock = GameClock {
            year: 2287,
            month: 10,
            day: 23,
            minute_of_day: 9 * 60 + 47,
        };
        assert!((clock.hour() - 9.783_333).abs() < 1e-5);
    }
}
