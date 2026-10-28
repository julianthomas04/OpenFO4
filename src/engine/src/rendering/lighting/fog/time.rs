use super::measured::NIGHT_FOG_RAMP_HOURS;
use crate::rendering::lighting::time_of_day::ClimateTimes;

/// How the time of day weighs the two halves of a weather's fog distances. The colours follow
/// the time slots instead.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct FogTime {
    /// 0 where the Day fog distances apply, 1 where the Night ones do.
    pub night_weight: f32,
}

impl FogTime {
    /// Full day, for a weather lit before any clock time is known.
    pub const DAY: Self = Self { night_weight: 0.0 };

    pub fn at(hour: f32, times: &ClimateTimes) -> Self {
        Self {
            night_weight: night_weight(hour, times),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::lighting::time_of_day::test_support::default_climate;

    #[test]
    fn the_night_weight_follows_the_probes_measured_hours() {
        let times = default_climate();
        for (hour, measured) in [
            (0.0, 1.000),
            (3.0, 0.991),
            (4.0, 0.822),
            (5.0, 0.648),
            (5.5, 0.580),
            (6.0, 0.511),
            (6.5, 0.422),
            (7.0, 0.355),
            (8.0, 0.169),
            (12.0, 0.000),
            (18.0, 0.153),
            (19.0, 0.353),
            (20.0, 0.516),
            (21.0, 0.632),
            (22.0, 0.828),
            (23.0, 1.000),
        ] {
            let weight = FogTime::at(hour, &times).night_weight;
            assert!((weight - measured).abs() < 0.04, "{hour}: {weight}");
        }
    }

    #[test]
    fn deep_night_is_all_night_and_midday_all_day() {
        let times = default_climate();
        for hour in [0.0, 2.0, 23.5] {
            assert_eq!(
                FogTime::at(hour, &times),
                FogTime { night_weight: 1.0 },
                "{hour}"
            );
        }
        for hour in [9.0, 12.0, 17.0] {
            assert_eq!(FogTime::at(hour, &times), FogTime::DAY, "{hour}");
        }
    }

    #[test]
    fn the_night_weight_changes_continuously_over_the_day() {
        let times = default_climate();
        let step = 0.01;
        for index in 0..2399 {
            let hour = index as f32 * step;
            let (here, there) = (FogTime::at(hour, &times), FogTime::at(hour + step, &times));
            assert!(
                (here.night_weight - there.night_weight).abs() <= 0.002,
                "{hour}"
            );
        }
    }
}
