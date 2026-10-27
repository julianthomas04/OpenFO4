use super::measured::NIGHT_FOG_RAMP_HOURS;
use crate::rendering::lighting::time_of_day::ClimateTimes;

/// How the time of day weighs the two halves of a weather's fog. The distances and the
/// colours change at different times, so each has its own weight.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct FogTime {
    /// 0 where the Day fog distances apply, 1 where the Night ones do.
    pub night_weight: f32,
    /// 0 where the fog colours take their night factors, 1 where they take their day factors.
    pub day_colour_weight: f32,
}

impl FogTime {
    /// Full day, for a weather lit before any clock time is known.
    pub const DAY: Self = Self {
        night_weight: 0.0,
        day_colour_weight: 1.0,
    };

    pub fn at(hour: f32, times: &ClimateTimes) -> Self {
        Self {
            night_weight: night_weight(hour, times),
            day_colour_weight: day_colour_weight(hour, times),
        }
    }
}

/// Ramps from 0 at sunrise end back over [`NIGHT_FOG_RAMP_HOURS`] towards midnight, and from 0
/// at sunset begin forward over as many.
fn night_weight(hour: f32, times: &ClimateTimes) -> f32 {
    let morning = (times.sunrise_end() - hour) / NIGHT_FOG_RAMP_HOURS;
    let evening = (hour - times.sunset_begin()) / NIGHT_FOG_RAMP_HOURS;
    morning.max(evening).clamp(0.0, 1.0)
}

/// Rises from 0 at the sunrise midpoint to 1 at sunrise end and falls from 1 at sunset begin to
/// 0 at the sunset midpoint.
fn day_colour_weight(hour: f32, times: &ClimateTimes) -> f32 {
    let sunrise_midpoint = times.sunrise_midpoint();
    let sunset_midpoint = times.sunset_midpoint();
    let morning = (hour - sunrise_midpoint) / (times.sunrise_end() - sunrise_midpoint);
    let evening = (sunset_midpoint - hour) / (sunset_midpoint - times.sunset_begin());
    morning.min(evening).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::lighting::time_of_day::test_support::default_climate;

    #[test]
    fn the_night_weight_follows_the_probes_measured_hours() {
        let times = default_climate();
        for (hour, measured) in [
            (0.0, 0.995),
            (6.0, 0.511),
            (6.5, 0.422),
            (7.0, 0.355),
            (8.0, 0.169),
            (12.0, 0.000),
            (18.0, 0.153),
            (19.0, 0.353),
            (20.0, 0.511),
        ] {
            let weight = FogTime::at(hour, &times).night_weight;
            assert!((weight - measured).abs() < 0.025, "{hour}: {weight}");
        }
    }

    #[test]
    fn the_day_colour_weight_follows_the_probes_measured_hours() {
        let times = default_climate();
        for (hour, measured) in [
            (6.0, 0.00),
            (6.5, 0.02),
            (7.0, 0.00),
            (8.0, 0.51),
            (12.0, 0.99),
            (18.0, 0.51),
            (19.0, 0.00),
            (20.0, 0.01),
        ] {
            let weight = FogTime::at(hour, &times).day_colour_weight;
            assert!((weight - measured).abs() < 0.025, "{hour}: {weight}");
        }
    }

    #[test]
    fn deep_night_is_all_night_and_midday_all_day() {
        let times = default_climate();
        for hour in [0.0, 2.0, 23.5] {
            assert_eq!(
                FogTime::at(hour, &times),
                FogTime {
                    night_weight: 1.0,
                    day_colour_weight: 0.0
                },
                "{hour}"
            );
        }
        for hour in [9.0, 12.0, 17.0] {
            assert_eq!(FogTime::at(hour, &times), FogTime::DAY, "{hour}");
        }
    }

    #[test]
    fn both_weights_change_continuously_over_the_day() {
        let times = default_climate();
        let step = 0.01;
        for index in 0..2399 {
            let hour = index as f32 * step;
            let (here, there) = (FogTime::at(hour, &times), FogTime::at(hour + step, &times));
            assert!(
                (here.night_weight - there.night_weight).abs() <= 0.002,
                "{hour}"
            );
            assert!(
                (here.day_colour_weight - there.day_colour_weight).abs() <= 0.006,
                "{hour}"
            );
        }
    }
}
