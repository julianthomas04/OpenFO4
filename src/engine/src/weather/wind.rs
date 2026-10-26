use std::f32::consts::TAU;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindParams {
    /// 0..1.
    pub speed: f32,
    /// 0..1; `None` when the weather's data predates the field.
    pub turbulence: Option<f32>,
    /// The record's unscaled byte.
    pub direction_raw: u8,
    /// The record's unscaled byte.
    pub direction_range_raw: u8,
}

impl WindParams {
    /// The direction of the wind as an angle in radians, see [`heading_of`].
    pub fn heading(&self) -> f32 {
        heading_of(self.direction_raw)
    }
}

/// A weather record's direction byte as an angle in radians: the byte spans a full turn.
pub fn heading_of(direction_raw: u8) -> f32 {
    f32::from(direction_raw) / f32::from(u8::MAX) * TAU
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-5,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn the_direction_byte_spans_a_full_turn() {
        close(heading_of(0), 0.0);
        close(heading_of(u8::MAX), TAU);
        close(heading_of(64), 64.0 / 255.0 * TAU);
    }

    #[test]
    fn a_larger_byte_is_a_larger_angle() {
        let angles: Vec<f32> = (0..=u8::MAX).map(heading_of).collect();
        assert!(angles.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn the_wind_of_a_weather_heads_where_its_direction_byte_points() {
        let wind = WindParams {
            speed: 0.4,
            turbulence: Some(0.2),
            direction_raw: 15,
            direction_range_raw: 200,
        };
        close(wind.heading(), 15.0 / 255.0 * TAU);
    }
}
