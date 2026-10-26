#![allow(
    clippy::float_cmp,
    reason = "the blends of the tests land on exact fractions or are compared by tolerance"
)]

use std::f32::consts::TAU;

use glam::Vec3;

use super::fixtures::{A, B, DAY, DUSK, NIGHT, library, settled, transition, weather, with_sky};
use super::*;
use crate::rendering::lighting::time_of_day::TimeSlot;
use crate::rendering::weather::WeatherDef;
use crate::rendering::weather::library::fixtures::library_of;
use crate::sim::LightningStrike;
use crate::weather::WindParams;

fn blend(
    library: &WeatherLibrary,
    snapshot: &WeatherSnapshot<'_>,
    time: SlotBlend,
) -> WeatherFrame {
    WeatherFrame::blend(library, snapshot, time).unwrap()
}

/// `A` is green by day and black by night, `B` white by day and blue by night.
fn green_and_white() -> WeatherLibrary {
    let by_day_and_night = |day, night| {
        move |slot| {
            if slot == TimeSlot::Night { night } else { day }
        }
    };
    library_of(vec![
        with_sky(weather("A"), by_day_and_night((0, 255, 0), (0, 0, 0))),
        with_sky(weather("B"), by_day_and_night((255, 255, 255), (0, 0, 255))),
    ])
}

fn close(actual: Vec3, expected: Vec3) {
    assert!(
        actual.abs_diff_eq(expected, 1e-5),
        "expected {expected}, got {actual}"
    );
}

fn with_wind(
    mut weather: WeatherDef,
    speed: f32,
    turbulence: f32,
    direction_raw: u8,
) -> WeatherDef {
    weather.wind = WindParams {
        speed,
        turbulence: Some(turbulence),
        direction_raw,
        direction_range_raw: 0,
    };
    weather
}

fn with_lightning_colour(mut weather: WeatherDef, colour: [u8; 3]) -> WeatherDef {
    weather.lightning.color_srgb = colour;
    weather
}

fn strike(weather: WeatherId, flash_intensity: f32) -> LightningStrike {
    LightningStrike {
        weather,
        distance_m: 900.0,
        thunder_delay_s: 2.6,
        flash_intensity,
    }
}

#[test]
fn the_sky_of_a_settled_weather_is_what_its_slots_make_of_the_time_of_day() {
    let library = green_and_white();
    for time in [DAY, NIGHT, DUSK] {
        for id in [A, B] {
            let frame = blend(&library, &settled(id), time);
            assert_eq!(
                frame.sky,
                library.get(id).unwrap().slots.sample(time),
                "{id:?} at {time:?}"
            );
        }
    }
}

#[test]
fn a_transition_starts_at_the_sky_of_the_weather_it_leaves_and_ends_at_that_of_the_weather_it_comes_to()
 {
    let library = green_and_white();
    let sky = |progress| {
        blend(&library, &transition(B, A, progress), DAY)
            .sky
            .sky_upper
    };
    close(
        sky(0.0),
        library.get(A).unwrap().slots.sample(DAY).sky_upper,
    );
    close(
        sky(1.0),
        library.get(B).unwrap().slots.sample(DAY).sky_upper,
    );
}

#[test]
fn during_a_transition_the_sky_is_blended_by_how_far_it_has_come() {
    let sky = blend(&green_and_white(), &transition(B, A, 0.25), DAY).sky;
    close(sky.sky_upper, Vec3::new(0.25, 1.0, 0.25));
}

#[test]
fn both_weathers_are_sampled_at_the_time_of_day_before_they_are_blended() {
    let library = green_and_white();
    let sky = |time| blend(&library, &transition(B, A, 0.5), time).sky.sky_upper;
    close(sky(DAY), Vec3::new(0.5, 1.0, 0.5));
    close(sky(NIGHT), Vec3::new(0.0, 0.0, 0.5));
}

#[test]
fn the_sky_is_named_for_the_weather_it_is_nearer_to() {
    let library = green_and_white();
    let name = |progress| {
        blend(&library, &transition(B, A, progress), DAY)
            .sky
            .form_key
    };
    assert_eq!(&*name(0.25), "A");
    assert_eq!(&*name(0.75), "B");
}

#[test]
fn the_wind_of_a_settled_weather_is_its_own() {
    let library = library_of(vec![with_wind(weather("A"), 0.3, 0.1, 64)]);
    let frame = blend(&library, &settled(A), DAY);
    assert_eq!(frame.wind, WindFrame::of(&library.get(A).unwrap().wind));
}

#[test]
fn during_a_transition_the_wind_is_blended_by_how_far_it_has_come() {
    let library = library_of(vec![
        with_wind(weather("A"), 0.2, 0.0, 0),
        with_wind(weather("B"), 0.6, 0.4, 64),
    ]);
    let wind = |progress| blend(&library, &transition(B, A, progress), DAY).wind;
    let halfway = wind(0.5);
    assert!((halfway.speed - 0.4).abs() < 1e-6);
    assert!((halfway.turbulence - 0.2).abs() < 1e-6);
    assert!((halfway.heading - 32.0 / 255.0 * TAU).abs() < 1e-5);
    assert_eq!(wind(0.0).speed, 0.2);
}

#[test]
fn a_frame_without_lightning_has_no_strikes() {
    assert_eq!(blend(&library(), &settled(A), DAY).strikes, []);
}

#[test]
fn a_strike_flashes_at_its_intensity_in_the_lightning_colour_of_its_weather() {
    let library = library_of(vec![
        with_lightning_colour(weather("A"), [128, 64, 200]),
        with_lightning_colour(weather("B"), [30, 160, 90]),
    ]);
    let strikes = [strike(B, 0.5), strike(A, 1.0)];
    let snapshot = WeatherSnapshot {
        strikes: &strikes,
        ..transition(B, A, 0.5)
    };
    let frame = blend(&library, &snapshot, DAY);
    assert_eq!(
        frame.strikes,
        [
            Strike::from_srgb8(0.5, [30, 160, 90]),
            Strike::from_srgb8(1.0, [128, 64, 200]),
        ]
    );
}

#[test]
fn a_weather_the_library_does_not_have_keeps_the_frame_from_being_made() {
    let library = library();
    let unknown = WeatherId::new(9);
    let strikes = [strike(unknown, 1.0)];
    let with_unknown_strike = WeatherSnapshot {
        strikes: &strikes,
        ..settled(A)
    };
    for snapshot in [
        settled(unknown),
        transition(A, unknown, 0.5),
        with_unknown_strike,
    ] {
        assert_eq!(
            WeatherFrame::blend(&library, &snapshot, DAY),
            Err(WeatherFrameError::UnknownWeather {
                weather: unknown,
                count: 2
            })
        );
    }
}

#[test]
fn a_transition_whose_progress_is_no_fraction_keeps_the_frame_from_being_made() {
    for progress in [-0.5, 1.5, f32::NAN] {
        let result = WeatherFrame::blend(&library(), &transition(B, A, progress), DAY);
        assert!(
            matches!(result, Err(WeatherFrameError::Progress { .. })),
            "{progress}: {result:?}"
        );
    }
}

#[test]
fn the_clouds_input_lends_the_layers_of_the_frame_with_the_flash_and_the_solo_layer() {
    let frame = blend(&library(), &settled(A), DAY);
    let input = frame.clouds_input(0.4, Some(3));
    assert!(matches!(input.layers, Cow::Borrowed(_)));
    assert_eq!(input.layers, frame.cloud_layers);
    assert!(!input.layers.is_empty());
    assert_eq!(input.flash, 0.4);
    assert_eq!(input.solo, Some(3));
}
