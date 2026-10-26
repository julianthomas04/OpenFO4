//! The distance and height fog of `fog.wgsl` and the sky's share of it in `sky_fog.wgsl`.

#![allow(
    clippy::float_cmp,
    reason = "a sky the fog does not reach keeps its radiance exactly"
)]

use glam::Vec3;

use super::harness::{Case, Environment, Output};
use super::{assert_close, assert_close3, run_in};
use crate::rendering::lighting::fog::{Fog, FogParams, HeightBand, HeightFog};
use crate::rendering::lighting::god_rays::{
    GodRaysData, InScatterUniformFields, SUN_IN_SCATTER_GAIN, SunInScatter,
};

const RED: Vec3 = Vec3::X;
const BLUE: Vec3 = Vec3::Z;
const SKY: Vec3 = Vec3::Y;
const SUN_RADIANCE: Vec3 = Vec3::new(2.0, 1.5, 1.0);

/// Fog from 10 to 110 units ahead, by the square of the distance, up to half opacity.
fn distance_fog() -> Fog {
    Fog::Enabled(FogParams {
        near_color: RED,
        far_color: BLUE,
        near: 10.0,
        far: 110.0,
        power: 2.0,
        max: 0.5,
        height: HeightFog::NONE,
    })
}

/// Linear fog over 0 to 100 units, the near band over heights 10 to 30 and the far band over
/// 25 to 75, where the density halves. A band starts at its mid and spans its range above it.
fn height_fog() -> Fog {
    Fog::Enabled(FogParams {
        near_color: RED,
        far_color: BLUE,
        near: 0.0,
        far: 100.0,
        power: 1.0,
        max: 1.0,
        height: HeightFog {
            near_high_color: Vec3::Y,
            far_high_color: Vec3::ONE,
            near_band: HeightBand::new(10.0, 20.0),
            far_band: HeightBand::new(25.0, 50.0),
            high_density_scale: 0.5,
        },
    })
}

fn fog_at(environment: &Environment, points: &[Vec3]) -> Vec<(Vec3, f32)> {
    let cases: Vec<Case> = points.iter().map(|point| Case::fog(*point)).collect();
    run_in(environment, &cases)
        .iter()
        .map(Output::color_alpha)
        .collect()
}

/// A surface of `color` at each of `points` under the fog.
fn surface_at(environment: &Environment, color: Vec3, points: &[Vec3]) -> Vec<Vec3> {
    let cases: Vec<Case> = points
        .iter()
        .map(|point| Case::apply_fog(color, *point))
        .collect();
    run_in(environment, &cases)
        .iter()
        .map(|output| output.color_alpha().0)
        .collect()
}

/// Just past the fog's far distance of 100, where the fog no longer changes with distance.
const PAST_FAR: f32 = 100.5;

fn sky_at(environment: &Environment, radiance: Vec3, directions: &[Vec3]) -> Vec<(Vec3, f32)> {
    let cases: Vec<Case> = directions
        .iter()
        .map(|direction| Case::sky_fog(radiance, *direction))
        .collect();
    run_in(environment, &cases)
        .iter()
        .map(Output::color_alpha)
        .collect()
}

/// The unit direction at `sine` of elevation, facing +X.
fn at_elevation(sine: f32) -> Vec3 {
    Vec3::new((1.0 - sine * sine).sqrt(), sine, 0.0)
}

/// The point `distance` from the origin at `height`.
fn at_height(height: f32, distance: f32) -> Vec3 {
    Vec3::new((distance * distance - height * height).sqrt(), height, 0.0)
}

fn assert_fog(actual: (Vec3, f32), color: Vec3, opacity: f32, row: &str) {
    assert_close3(actual.0, color, &format!("{row} colour"));
    assert_close(actual.1, opacity, &format!("{row} opacity"));
}

#[test]
#[ignore = "needs a GPU adapter"]
fn the_fog_colour_follows_the_powered_and_capped_fog_factor() {
    let environment = Environment::with_fog(Vec3::ZERO, &distance_fog());
    let fog = fog_at(
        &environment,
        &[
            Vec3::new(0.0, 0.0, -5.0),
            Vec3::new(0.0, 0.0, -60.0),
            Vec3::new(0.0, 0.0, -200.0),
        ],
    );
    assert_fog(fog[0], RED, 0.0, "before the fog starts");
    // Half way the factor is 0.5 squared, not the linear 0.5.
    assert_fog(fog[1], Vec3::new(0.75, 0.0, 0.25), 0.25, "half way");
    assert_fog(
        fog[2],
        Vec3::new(0.5, 0.0, 0.5),
        0.5,
        "capped at the maximum",
    );
}

#[test]
#[ignore = "needs a GPU adapter"]
fn fog_without_height_data_is_the_same_at_every_height() {
    let environment = Environment::with_fog(Vec3::ZERO, &distance_fog());
    let fog = fog_at(
        &environment,
        &[
            at_height(-50.0, 60.0),
            at_height(0.0, 60.0),
            at_height(55.0, 60.0),
        ],
    );
    for (row, sample) in ["below", "level", "above"].into_iter().zip(&fog) {
        assert_fog(*sample, fog[1].0, fog[1].1, row);
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn height_turns_each_colour_towards_its_high_colour_and_thins_the_fog_over_the_far_band() {
    let environment = Environment::with_fog(Vec3::ZERO, &height_fog());
    let fog = fog_at(
        &environment,
        &[
            at_height(20.0, 50.0),
            at_height(50.0, 100.0),
            Vec3::new(0.0, 100.0, 0.0),
        ],
    );
    // Half way up the near band, under the far band: the near colour is half green.
    assert_fog(fog[0], Vec3::new(0.25, 0.25, 0.5), 0.5, "low and half way");
    // Over the near band and half way up the far band, fully developed.
    assert_fog(
        fog[1],
        Vec3::new(0.5, 0.5, 1.0),
        0.75,
        "half way up the far band",
    );
    assert_fog(fog[2], Vec3::ONE, 0.5, "over both bands");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn disabled_fog_has_no_opacity_anywhere() {
    let environment = Environment::with_fog(Vec3::ZERO, &Fog::Disabled);
    for (color, opacity) in fog_at(
        &environment,
        &[
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, -1.0e6),
            Vec3::new(5.0, 1.0e4, 0.0),
        ],
    ) {
        assert_eq!((color, opacity), (Vec3::ZERO, 0.0));
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn the_sky_takes_the_fog_at_and_below_the_horizon_and_clears_with_elevation() {
    let environment = Environment::with_fog(Vec3::new(0.0, 5.0, 0.0), &distance_fog());
    let sky = sky_at(
        &environment,
        SKY,
        &[
            at_elevation(-0.3),
            at_elevation(0.0),
            at_elevation(0.1),
            at_elevation(0.2),
            at_elevation(0.7),
        ],
    );
    // At the far distance the fog is at its maximum, half of the purple far end.
    let fogged = Vec3::new(0.25, 0.5, 0.25);
    assert_fog(sky[0], fogged, 1.0, "below the horizon");
    assert_fog(sky[1], fogged, 1.0, "at the horizon");
    assert_close(sky[2].1, 0.5, "half way to the clear elevation");
    assert_eq!(sky[3], (SKY, 0.0), "clear elevation");
    assert_eq!(sky[4], (SKY, 0.0), "high sky");
}

/// The sky at the horizon looks like a surface of the sky's colour just past the far distance
/// that way.
fn assert_horizon_meets_the_fogged_surface(environment: &Environment, camera: Vec3, horizon: Vec3) {
    let surface = surface_at(environment, SKY, &[camera + horizon * PAST_FAR])[0];
    let (sky, _) = sky_at(environment, SKY, &[horizon])[0];
    assert_close3(sky, surface, &format!("horizon {horizon}"));
}

#[test]
#[ignore = "needs a GPU adapter"]
fn the_horizon_meets_a_fogged_surface_of_the_sky_colour_just_past_the_far_distance() {
    let camera = Vec3::new(0.0, 20.0, 0.0);
    let environment = Environment::with_fog(camera, &height_fog());
    assert_horizon_meets_the_fogged_surface(&environment, camera, Vec3::X);
}

#[test]
#[ignore = "needs a GPU adapter"]
fn without_fog_the_sky_keeps_its_radiance() {
    let environment = Environment::with_fog(Vec3::ZERO, &Fog::Disabled);
    for (radiance, _) in sky_at(&environment, SKY, &[at_elevation(-0.5), at_elevation(0.0)]) {
        assert_eq!(radiance, SKY);
    }
}

fn god_rays(intensity: f32) -> GodRaysData {
    GodRaysData {
        back_color: [64, 128, 255],
        forward_color: [255, 200, 100],
        air_color: [100, 150, 200],
        back_color_scale: 3.0,
        forward_color_scale: 1.0,
        air_color_scale: 0.5,
        intensity,
        back_phase: 0.4,
        forward_phase: 0.8,
    }
}

fn in_scatter(intensity: f32) -> SunInScatter {
    SunInScatter::from_god_rays(&god_rays(intensity)).unwrap()
}

/// The sun low in the east, along +X tilted up.
fn to_sun() -> Vec3 {
    Vec3::new(1.0, 0.3, 0.0).normalize()
}

fn in_scatter_fields(intensity: f32) -> InScatterUniformFields {
    in_scatter(intensity).uniform_fields(SUN_RADIANCE, to_sun())
}

#[test]
#[ignore = "needs a GPU adapter"]
fn the_in_scatter_adds_the_phase_weighted_sun_light_to_the_fog_colour() {
    let environment =
        Environment::with_fog_and_in_scatter(Vec3::ZERO, &distance_fog(), in_scatter_fields(2.0));
    let directions = [
        to_sun(),
        Vec3::X,
        Vec3::Z,
        -Vec3::X,
        -to_sun(),
        Vec3::new(0.0, -1.0, 1.0).normalize(),
    ];
    let points: Vec<Vec3> = directions
        .iter()
        .map(|direction| *direction * 200.0)
        .collect();
    let fog = fog_at(&environment, &points);
    // Fully developed: half of the purple far end, before the sun's light.
    let base = Vec3::new(0.5, 0.0, 0.5);
    for (direction, sample) in directions.iter().zip(&fog) {
        let scattered =
            in_scatter(2.0).radiance(direction.dot(to_sun())) * SUN_RADIANCE * SUN_IN_SCATTER_GAIN;
        assert_fog(*sample, base + scattered, 0.5, &format!("{direction}"));
    }
    assert!(
        fog[0].0.x > fog[1].0.x && fog[1].0.x > fog[2].0.x,
        "the fog brightens towards the sun: {fog:?}"
    );
}

#[test]
#[ignore = "needs a GPU adapter"]
fn without_god_rays_or_at_zero_intensity_the_fog_and_sky_stay_bit_for_bit() {
    let plain = Environment::with_fog(Vec3::new(0.0, 5.0, 0.0), &height_fog());
    let dark = Environment::with_fog_and_in_scatter(
        Vec3::new(0.0, 5.0, 0.0),
        &height_fog(),
        in_scatter_fields(0.0),
    );
    let points = [
        Vec3::new(0.0, 5.0, 0.0),
        Vec3::new(0.0, 5.0, -60.0),
        at_height(40.0, 90.0),
        to_sun() * 500.0,
    ];
    assert_eq!(fog_at(&dark, &points), fog_at(&plain, &points));
    let directions = [at_elevation(-0.2), at_elevation(0.05), to_sun()];
    assert_eq!(
        sky_at(&dark, SKY, &directions),
        sky_at(&plain, SKY, &directions)
    );
}

#[test]
#[ignore = "needs a GPU adapter"]
fn with_in_scatter_the_horizon_still_meets_a_fogged_surface_at_the_far_distance() {
    let camera = Vec3::new(0.0, 20.0, 0.0);
    let environment =
        Environment::with_fog_and_in_scatter(camera, &height_fog(), in_scatter_fields(2.0));
    let toward_sun = Vec3::new(to_sun().x, 0.0, to_sun().z).normalize();
    for horizon in [toward_sun, Vec3::Z, -toward_sun] {
        assert_horizon_meets_the_fogged_surface(&environment, camera, horizon);
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn without_fog_the_in_scatter_shows_nowhere() {
    let environment =
        Environment::with_fog_and_in_scatter(Vec3::ZERO, &Fog::Disabled, in_scatter_fields(2.0));
    for (color, opacity) in fog_at(&environment, &[to_sun() * 500.0, Vec3::Z * 500.0]) {
        assert_eq!((color, opacity), (Vec3::ZERO, 0.0));
    }
    for (radiance, _) in sky_at(&environment, SKY, &[to_sun(), at_elevation(0.0)]) {
        assert_eq!(radiance, SKY);
    }
}
