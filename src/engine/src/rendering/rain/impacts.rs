//! Where raindrops strike the water: the points to hand the water displacement as impacts, so
//! the rain stirs ripples in the surface around the camera.
//!
//! The points lie on the horizontal disc around the centre; the frame keeps the ones that fall
//! on water and sets their height to the surface.

use std::f32::consts::{PI, TAU};

use glam::Vec3;

use super::tuning::{
    IMPACTS_PER_SQUARE_METER_PER_SECOND, LONGEST_STEP_SECONDS, MAX_IMPACTS_PER_FRAME,
};
use crate::bethesda_space::ENGINE_UNITS_PER_METER;
use crate::random::RandomSource;
use crate::utils::finite_or_zero;

/// A whole number of impacts averaging `expected`: its integer part, and one more with the
/// chance of its fraction.
fn impact_count(rng: &mut (impl RandomSource + ?Sized), expected: f32) -> usize {
    let whole = expected.floor();
    whole as usize + usize::from(rng.unit() < expected - whole)
}

/// The area in square metres of the disc of `radius` engine units; zero where that is no number.
fn disc_area(radius: f32) -> f32 {
    let metres = radius / ENGINE_UNITS_PER_METER;
    finite_or_zero(PI * metres * metres)
}

/// The impacts of one frame of `dt` seconds on the disc of `radius` engine units around
/// `centre`: on average [`IMPACTS_PER_SQUARE_METER_PER_SECOND`] times the area of the disc in
/// square metres times `intensity` (0 to 1) times `dt`, and at most [`MAX_IMPACTS_PER_FRAME`].
/// Each lies at a uniformly random spot of the disc and at the height of `centre`. Nothing is
/// drawn from `rng` when there are none.
pub fn rain_impacts<R: RandomSource + ?Sized>(
    rng: &mut R,
    dt: f32,
    intensity: f32,
    centre: Vec3,
    radius: f32,
) -> impl Iterator<Item = Vec3> {
    let seconds = finite_or_zero(dt).clamp(0.0, LONGEST_STEP_SECONDS);
    let radius = finite_or_zero(radius).max(0.0);
    let expected = (IMPACTS_PER_SQUARE_METER_PER_SECOND
        * disc_area(radius)
        * finite_or_zero(intensity).clamp(0.0, 1.0)
        * seconds)
        .min(MAX_IMPACTS_PER_FRAME as f32);
    let count = if expected > 0.0 && centre.is_finite() {
        impact_count(rng, expected)
    } else {
        0
    };
    (0..count).map(move |_| {
        let distance = radius * rng.unit().sqrt();
        let (sin, cos) = (TAU * rng.unit()).sin_cos();
        centre + Vec3::new(cos * distance, 0.0, sin * distance)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::random::SplitMix64;

    const CENTRE: Vec3 = Vec3::new(1234.5, 42.0, -987.25);
    const RADIUS: f32 = 10.0 * ENGINE_UNITS_PER_METER;
    const FRAME: f32 = 1.0 / 60.0;
    /// Frames enough for the count of impacts to settle within 3 per cent of its average.
    const LONG_RUN: u32 = 60_000;

    /// The impacts a second of full rain makes on the disc of `radius` engine units.
    fn per_second(radius: f32) -> f32 {
        IMPACTS_PER_SQUARE_METER_PER_SECOND * PI * (radius / ENGINE_UNITS_PER_METER).powi(2)
    }

    /// Answers every request for a unit value with the same one.
    struct Constant(f32);

    impl RandomSource for Constant {
        fn below(&mut self, n: usize) -> usize {
            (self.0 * n as f32) as usize
        }
    }

    fn count_with(unit: f32, dt: f32) -> usize {
        rain_impacts(&mut Constant(unit), dt, 1.0, CENTRE, RADIUS).count()
    }

    fn impacts(rng: &mut SplitMix64, frames: u32, intensity: f32) -> Vec<Vec3> {
        (0..frames)
            .flat_map(|_| rain_impacts(rng, FRAME, intensity, CENTRE, RADIUS).collect::<Vec<_>>())
            .collect()
    }

    #[test]
    fn a_dry_sky_or_an_empty_frame_has_no_impacts() {
        let mut rng = SplitMix64::new(1);
        for (dt, intensity, radius) in [
            (FRAME, 0.0, RADIUS),
            (0.0, 1.0, RADIUS),
            (FRAME, 1.0, 0.0),
            (-1.0, 1.0, RADIUS),
            (FRAME, -1.0, RADIUS),
            (f32::NAN, 1.0, RADIUS),
            (FRAME, f32::NAN, RADIUS),
            (FRAME, 1.0, f32::NAN),
            (FRAME, 1.0, f32::INFINITY),
            (FRAME, 1.0, f32::MAX),
        ] {
            let found = rain_impacts(&mut rng, dt, intensity, CENTRE, radius).count();
            assert_eq!(found, 0, "dt {dt}, intensity {intensity}, radius {radius}");
        }
        assert_eq!(
            rain_impacts(&mut rng, FRAME, 1.0, Vec3::NAN, RADIUS).count(),
            0,
            "no centre"
        );
    }

    #[test]
    fn a_frame_without_impacts_leaves_the_generator_alone() {
        let mut rng = SplitMix64::new(7);
        let mut untouched = rng.clone();

        assert_eq!(
            rain_impacts(&mut rng, FRAME, 0.0, CENTRE, RADIUS).count(),
            0
        );

        assert_eq!(rng.below(1000), untouched.below(1000));
    }

    #[test]
    fn the_expected_count_is_split_into_whole_impacts_and_a_chance_of_one_more() {
        let dt = 2.5 / per_second(RADIUS);
        assert_eq!(count_with(0.0, dt), 3);
        assert_eq!(count_with(0.4, dt), 3);
        assert_eq!(count_with(0.6, dt), 2);
        assert_eq!(count_with(0.99, dt), 2);
    }

    #[test]
    fn the_impacts_average_the_rate_times_the_intensity_and_the_time() {
        for intensity in [1.0, 0.5, 0.1] {
            let mut rng = SplitMix64::new(11);
            let found = impacts(&mut rng, LONG_RUN, intensity).len() as f32;
            let expected = per_second(RADIUS) * intensity * FRAME * LONG_RUN as f32;
            assert!(
                (found - expected).abs() < 0.03 * expected,
                "intensity {intensity}: {found} impacts against {expected}"
            );
        }
    }

    #[test]
    fn the_rate_is_per_square_metre_whatever_the_size_of_the_disc() {
        for metres in [3.0, 6.0, 10.0] {
            let radius = metres * ENGINE_UNITS_PER_METER;
            let mut rng = SplitMix64::new(11);
            let found: usize = (0..LONG_RUN)
                .map(|_| rain_impacts(&mut rng, FRAME, 1.0, CENTRE, radius).count())
                .sum();
            let square_metres = PI * metres * metres;
            let rate = found as f32 / (square_metres * FRAME * LONG_RUN as f32);
            assert!(
                (rate - IMPACTS_PER_SQUARE_METER_PER_SECOND).abs()
                    < 0.03 * IMPACTS_PER_SQUARE_METER_PER_SECOND,
                "a disc of {metres} m: {rate} impacts per square metre and second"
            );
        }
    }

    #[test]
    fn a_frame_never_hands_over_more_than_its_cap() {
        let wide = 100.0 * ENGINE_UNITS_PER_METER;
        assert!(per_second(wide) * FRAME > MAX_IMPACTS_PER_FRAME as f32);
        for dt in [FRAME, 60.0] {
            let found = rain_impacts(&mut SplitMix64::new(3), dt, 1.0, CENTRE, wide).count();
            assert_eq!(found, MAX_IMPACTS_PER_FRAME, "a frame of {dt} s");
        }
    }

    #[test]
    fn the_intensity_stops_at_one() {
        let mut rng = SplitMix64::new(11);
        let full = impacts(&mut rng, 2000, 1.0).len();
        let mut rng = SplitMix64::new(11);
        assert_eq!(impacts(&mut rng, 2000, 50.0).len(), full);
    }

    #[test]
    fn a_stalled_frame_counts_as_a_short_one() {
        let burst = rain_impacts(&mut SplitMix64::new(3), 60.0, 1.0, CENTRE, RADIUS).count();
        let most = (per_second(RADIUS) * LONGEST_STEP_SECONDS).ceil() as usize;
        assert!(burst <= most, "{burst} impacts");
        assert!(burst >= most - 1, "{burst} impacts");
    }

    #[test]
    fn the_impacts_land_on_the_disc_at_the_height_of_its_centre() {
        let mut rng = SplitMix64::new(5);
        let all = impacts(&mut rng, 2000, 1.0);
        assert!(all.len() > 1000);
        for impact in &all {
            let offset = *impact - CENTRE;
            assert_eq!(impact.y, CENTRE.y);
            assert!(offset.length() <= RADIUS * 1.0001, "{impact}");
        }
    }

    #[test]
    fn the_impacts_cover_the_disc_evenly() {
        let mut rng = SplitMix64::new(5);
        let all = impacts(&mut rng, 20_000, 1.0);
        let share = |keep: &dyn Fn(Vec3) -> bool| {
            all.iter().filter(|&&impact| keep(impact - CENTRE)).count() as f32 / all.len() as f32
        };
        let inner = share(&|offset| offset.length() < RADIUS * 0.5);
        assert!(
            (inner - 0.25).abs() < 0.02,
            "a quarter of the area, found {inner}"
        );
        for (name, quadrant) in [
            ("+x +z", share(&|o| o.x >= 0.0 && o.z >= 0.0)),
            ("-x +z", share(&|o| o.x < 0.0 && o.z >= 0.0)),
            ("+x -z", share(&|o| o.x >= 0.0 && o.z < 0.0)),
            ("-x -z", share(&|o| o.x < 0.0 && o.z < 0.0)),
        ] {
            assert!((quadrant - 0.25).abs() < 0.02, "{name}: {quadrant}");
        }
        let rim = share(&|offset| offset.length() > RADIUS * 0.95);
        assert!(rim > 0.05, "the rim is reached: {rim}");
    }

    #[test]
    fn the_same_seed_makes_the_same_rain() {
        let first = impacts(&mut SplitMix64::new(9), 300, 1.0);
        let second = impacts(&mut SplitMix64::new(9), 300, 1.0);
        assert_eq!(first, second);
        assert_ne!(first, impacts(&mut SplitMix64::new(10), 300, 1.0));
    }
}
