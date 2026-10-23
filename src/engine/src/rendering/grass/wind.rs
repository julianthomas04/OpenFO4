//! Vanilla grass wind: how the weather's wind becomes the sway amplitude range,
//! wave frequency and heading the grass vertex shader consumes, and how a GRAS
//! wave period becomes a phase rate.

use std::f32::consts::TAU;

use crate::bethesda_space::UNITS_TO_ENGINE;

/// Game units of sway amplitude per unit of wind speed (the 300 multiplier
/// applied to both amplitude bounds).
const WIND_SPEED_AMPLITUDE: f32 = 300.0;
/// `fWindSpeedLowestLowMultiplier` and `fWindSpeedHighestHighMultiplier`: the
/// share of the wind speed that bounds the amplitude from below and above at
/// full turbulence.
const LOWEST_LOW_SPEED_MULTIPLIER: f32 = 0.0;
const HIGHEST_HIGH_SPEED_MULTIPLIER: f32 = 1.5;
/// `fWindLowestFrequency` and `fWindHighestFrequency`: the wave frequency
/// multiplier at zero and full turbulence.
const LOWEST_FREQUENCY: f32 = 0.5;
const HIGHEST_FREQUENCY: f32 = 4.0;
/// Divisor that turns a GRAS wave period into the wave phase rate in radians per second.
const WAVE_PERIOD_DIVISOR: f32 = 600.0;
/// WTHR wind bytes are fractions of 255; the heading byte spans a full turn.
const BYTE_SCALE: f32 = 255.0;
/// Upper bound of the wave factor on the amplitude, used for the culling bounds.
const MAX_WAVE_FACTOR: f32 = 1.8;
/// Upper bound on the blade lean in game units per unit of amplitude, used for the culling bounds.
const LEAN_PER_AMPLITUDE: f32 = 0.5;

/// The wind of a weather, as the WTHR record stores it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindConditions {
    /// 0..1.
    pub speed: f32,
    /// 0..1.
    pub turbulence: f32,
    /// Radians from +X in the Bethesda XY plane.
    pub heading: f32,
}

impl WindConditions {
    pub fn from_weather_bytes(speed: u8, turbulence: u8, heading: u8) -> Self {
        Self {
            speed: f32::from(speed) / BYTE_SCALE,
            turbulence: f32::from(turbulence) / BYTE_SCALE,
            heading: f32::from(heading) / BYTE_SCALE * TAU,
        }
    }

    /// Stand-in until weather wind is loaded: the values of the vanilla weather
    /// `CommonwealthClear` (wind speed 120, direction 15, turbulence 48).
    pub fn commonwealth_clear() -> Self {
        Self::from_weather_bytes(120, 48, 15)
    }
}

/// The frame-wide wind inputs of the grass vertex shader.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrassWind {
    /// Sway heading, radians from +X in the Bethesda XY plane.
    pub direction: f32,
    /// Bounds of the sway amplitude, in game units.
    pub magnitude_min: f32,
    pub magnitude_max: f32,
    /// Multiplier of the whole wave phase (time and position terms).
    pub frequency: f32,
}

impl GrassWind {
    pub fn from_conditions(wind: WindConditions) -> Self {
        let calm_share = 1.0 - wind.turbulence;
        let amplitude = |multiplier: f32| {
            (wind.turbulence * multiplier + calm_share) * wind.speed * WIND_SPEED_AMPLITUDE
        };
        Self {
            direction: wind.heading,
            magnitude_min: amplitude(LOWEST_LOW_SPEED_MULTIPLIER),
            magnitude_max: amplitude(HIGHEST_HIGH_SPEED_MULTIPLIER),
            frequency: wind.turbulence * HIGHEST_FREQUENCY + calm_share * LOWEST_FREQUENCY,
        }
    }

    pub fn default_weather() -> Self {
        Self::from_conditions(WindConditions::commonwealth_clear())
    }

    /// Upper bound on how far a blade leans from its rest position, in engine units.
    pub fn max_lean(self) -> f32 {
        let half_range = (self.magnitude_max - self.magnitude_min) * 0.5;
        let amplitude = MAX_WAVE_FACTOR * half_range + self.magnitude_min;
        LEAN_PER_AMPLITUDE * amplitude * UNITS_TO_ENGINE
    }
}

/// Radians per second at which a grass type's wave advances before the wind
/// frequency applies, from its GRAS wave period.
pub fn wave_rate(wave_period: f32) -> f32 {
    wave_period * TAU / WAVE_PERIOD_DIVISOR
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-3,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn weather_bytes_become_unit_fractions_and_a_heading_in_radians() {
        let wind = WindConditions::from_weather_bytes(255, 51, 64);
        close(wind.speed, 1.0);
        close(wind.turbulence, 0.2);
        close(wind.heading, 64.0 / 255.0 * TAU);
    }

    #[test]
    fn calm_wind_bounds_the_amplitude_by_the_speed_alone() {
        let wind = GrassWind::from_conditions(WindConditions {
            speed: 0.5,
            turbulence: 0.0,
            heading: 0.0,
        });
        close(wind.magnitude_min, 150.0);
        close(wind.magnitude_max, 150.0);
        close(wind.frequency, 0.5);
    }

    #[test]
    fn full_turbulence_spreads_the_amplitude_and_raises_the_frequency() {
        let wind = GrassWind::from_conditions(WindConditions {
            speed: 1.0,
            turbulence: 1.0,
            heading: 0.0,
        });
        close(wind.magnitude_min, 0.0);
        close(wind.magnitude_max, 450.0);
        close(wind.frequency, 4.0);
    }

    #[test]
    fn no_wind_gives_no_amplitude() {
        let wind = GrassWind::from_conditions(WindConditions {
            speed: 0.0,
            turbulence: 0.7,
            heading: 1.0,
        });
        close(wind.magnitude_min, 0.0);
        close(wind.magnitude_max, 0.0);
        close(wind.max_lean(), 0.0);
        close(wind.direction, 1.0);
    }

    #[test]
    fn default_weather_is_the_commonwealth_clear_wind() {
        let wind = GrassWind::default_weather();
        close(wind.magnitude_min, 114.602);
        close(wind.magnitude_max, 154.464);
        close(wind.frequency, 1.1588);
        close(wind.direction, 15.0 / 255.0 * TAU);
    }

    #[test]
    fn max_lean_bounds_the_lean_by_the_scaled_peak_amplitude_in_engine_units() {
        let wind = GrassWind {
            direction: 0.0,
            magnitude_min: 10.0,
            magnitude_max: 30.0,
            frequency: 1.0,
        };
        // MAX_WAVE_FACTOR * half range + minimum = 28 game units, scaled by LEAN_PER_AMPLITUDE and
        // converted at 0.1 engine units per game unit.
        close(wind.max_lean(), 1.4);
    }

    #[test]
    fn wave_rate_scales_the_period_by_a_turn_over_the_divisor() {
        close(wave_rate(600.0), TAU);
        close(wave_rate(100.0), TAU / 6.0);
        close(wave_rate(0.0), 0.0);
    }
}
