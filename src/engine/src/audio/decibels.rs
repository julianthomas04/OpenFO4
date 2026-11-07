//! Amplitude ratios as decibels, the unit every gain of the audio is written in.

const DB_PER_DECADE_OF_AMPLITUDE: f32 = 20.0;

/// The gain of the amplitude ratio `amplitude` in dB: 0 at 1 and minus infinity at 0.
pub fn amplitude_to_db(amplitude: f32) -> f32 {
    DB_PER_DECADE_OF_AMPLITUDE * amplitude.log10()
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the expected gains at the two ends are exact"
)]
mod tests {
    use super::*;

    #[test]
    fn the_gain_of_an_amplitude_is_zero_at_unity_and_falls_by_six_db_per_halving() {
        assert_eq!(amplitude_to_db(1.0), 0.0);
        assert!((amplitude_to_db(0.5) + 6.0206).abs() < 1e-3);
        assert!((amplitude_to_db(0.1) + 20.0).abs() < 1e-3);
    }

    #[test]
    fn no_amplitude_is_minus_infinity_db() {
        assert_eq!(amplitude_to_db(0.0), f32::NEG_INFINITY);
    }
}
