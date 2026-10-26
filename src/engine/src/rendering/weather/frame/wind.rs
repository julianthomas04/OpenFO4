use std::f32::consts::{PI, TAU};

use crate::rendering::lighting::lerp::Lerp;
use crate::weather::WindParams;

/// The wind of a frame: the wind of the weather, or of both weathers of a transition blended.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindFrame {
    /// 0..1.
    pub speed: f32,
    /// 0..1.
    pub turbulence: f32,
    /// Radians from +X in the Bethesda XY plane, `0..=TAU`: the angle the weather's direction
    /// byte stands for.
    pub heading: f32,
}

impl WindFrame {
    pub const CALM: Self = Self {
        speed: 0.0,
        turbulence: 0.0,
        heading: 0.0,
    };

    /// The wind of a weather; a weather whose data predates turbulence has none.
    pub fn of(wind: &WindParams) -> Self {
        Self {
            speed: wind.speed,
            turbulence: wind.turbulence.unwrap_or(0.0),
            heading: wind.heading(),
        }
    }

    /// The strongest wind that blending any of `winds` can make: their greatest speed and their
    /// greatest turbulence, with no heading. Calm without any wind.
    pub fn ceiling(winds: impl IntoIterator<Item = Self>) -> Self {
        winds.into_iter().fold(Self::CALM, |ceiling, wind| Self {
            speed: ceiling.speed.max(wind.speed),
            turbulence: ceiling.turbulence.max(wind.turbulence),
            heading: ceiling.heading,
        })
    }
}

/// The speed and the turbulence blend linearly and the heading goes the shorter way round.
impl Lerp for WindFrame {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        Self {
            speed: Lerp::lerp(&self.speed, &other.speed, t),
            turbulence: Lerp::lerp(&self.turbulence, &other.turbulence, t),
            heading: lerp_heading(self.heading, other.heading, t),
        }
    }
}

/// `from` turned `t` of the way to `to` by the shorter way round. Two opposite headings turn
/// towards the smaller angle, whichever is given first.
fn lerp_heading(from: f32, to: f32, t: f32) -> f32 {
    let turn = (to - from + PI).rem_euclid(TAU) - PI;
    (from + turn * t).rem_euclid(TAU)
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

    fn wind(speed: f32, turbulence: f32, heading: f32) -> WindFrame {
        WindFrame {
            speed,
            turbulence,
            heading,
        }
    }

    fn params(turbulence: Option<f32>) -> WindParams {
        WindParams {
            speed: 0.4,
            turbulence,
            direction_raw: 64,
            direction_range_raw: 10,
        }
    }

    #[test]
    fn the_wind_of_a_weather_carries_its_speed_turbulence_and_heading() {
        let frame = WindFrame::of(&params(Some(0.2)));
        assert_eq!((frame.speed, frame.turbulence), (0.4, 0.2));
        close(frame.heading, 64.0 / 255.0 * TAU);
    }

    #[test]
    fn a_weather_whose_data_predates_turbulence_has_none() {
        assert_eq!(WindFrame::of(&params(None)).turbulence, 0.0);
    }

    #[test]
    fn speed_and_turbulence_blend_linearly() {
        let blended = wind(0.2, 0.8, 1.0).lerp(&wind(0.6, 0.0, 1.0), 0.25);
        close(blended.speed, 0.3);
        close(blended.turbulence, 0.6);
    }

    #[test]
    fn a_blend_starts_at_the_first_wind_and_ends_at_the_second() {
        let (from, to) = (wind(0.2, 0.8, 1.0), wind(0.6, 0.0, 2.0));
        assert_eq!(from.lerp(&to, 0.0), from);
        let end = from.lerp(&to, 1.0);
        close(end.speed, to.speed);
        close(end.turbulence, to.turbulence);
        close(end.heading, to.heading);
    }

    #[test]
    fn the_heading_turns_the_shorter_way_round() {
        let heading = |degrees: f32| wind(0.0, 0.0, degrees.to_radians());
        let across_north = heading(340.0).lerp(&heading(40.0), 0.25);
        close(across_north.heading.to_degrees(), 355.0);
        let back = heading(40.0).lerp(&heading(340.0), 0.75);
        close(back.heading.to_degrees(), 355.0);
        let plain = heading(80.0).lerp(&heading(100.0), 0.5);
        close(plain.heading.to_degrees(), 90.0);
    }

    #[test]
    fn the_heading_stays_within_a_turn() {
        for (from, to) in [(0.1, 6.2), (6.2, 0.1), (TAU, 0.0), (3.0, 3.0)] {
            for step in 0..=10 {
                let t = step as f32 / 10.0;
                let heading = lerp_heading(from, to, t);
                assert!(
                    (0.0..=TAU).contains(&heading),
                    "{from} to {to} at {t}: {heading}"
                );
            }
        }
    }

    #[test]
    fn opposite_headings_turn_towards_the_smaller_angle_whichever_is_first() {
        close(lerp_heading(0.0, PI, 0.25), 1.75 * PI);
        close(lerp_heading(PI, 0.0, 0.25), 0.75 * PI);
    }

    #[test]
    fn the_ceiling_is_the_greatest_speed_and_the_greatest_turbulence_of_any_wind() {
        let ceiling = WindFrame::ceiling([wind(0.5, 0.1, 1.0), wind(0.2, 0.9, 2.0)]);
        assert_eq!((ceiling.speed, ceiling.turbulence), (0.5, 0.9));
    }

    #[test]
    fn without_any_wind_the_ceiling_is_calm() {
        assert_eq!(WindFrame::ceiling([]), WindFrame::CALM);
    }

    #[test]
    fn no_blend_of_two_winds_exceeds_their_ceiling() {
        let winds = [
            wind(0.9, 0.1, 0.5),
            wind(0.3, 0.8, 4.0),
            wind(0.6, 0.4, 6.0),
        ];
        let ceiling = WindFrame::ceiling(winds);
        for from in winds {
            for to in winds {
                for step in 0..=20 {
                    let blended = from.lerp(&to, step as f32 / 20.0);
                    assert!(blended.speed <= ceiling.speed + 1e-6, "{blended:?}");
                    assert!(
                        blended.turbulence <= ceiling.turbulence + 1e-6,
                        "{blended:?}"
                    );
                }
            }
        }
    }
}
