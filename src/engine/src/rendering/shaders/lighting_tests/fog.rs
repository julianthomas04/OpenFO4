//! The distance and height fog of `fog.wgsl` and the sky's share of it in `sky_fog.wgsl`.

#![allow(
    clippy::float_cmp,
    reason = "a sky the fog does not reach keeps its radiance exactly"
)]

use glam::Vec3;

use super::harness::{Case, Environment, Output};
use super::{assert_close, assert_close3, run_in};
use crate::rendering::lighting::fog::{Fog, FogParams, HeightBand, HeightFog};

const RED: Vec3 = Vec3::X;
const BLUE: Vec3 = Vec3::Z;
const SKY: Vec3 = Vec3::Y;

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
/// 25 to 75, where the density halves.
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
            near_band: HeightBand::new(20.0, 10.0),
            far_band: HeightBand::new(50.0, 25.0),
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

#[test]
#[ignore = "needs a GPU adapter"]
fn the_horizon_meets_a_fogged_surface_of_the_sky_colour_at_the_far_distance() {
    let camera = Vec3::new(0.0, 20.0, 0.0);
    let environment = Environment::with_fog(camera, &height_fog());
    let horizon = Vec3::X;
    let (fog_color, opacity) = fog_at(&environment, &[camera + horizon * 100.0])[0];
    let (sky, _) = sky_at(&environment, SKY, &[horizon])[0];
    assert_close3(sky, SKY.lerp(fog_color, opacity), "horizon");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn without_fog_the_sky_keeps_its_radiance() {
    let environment = Environment::with_fog(Vec3::ZERO, &Fog::Disabled);
    for (radiance, _) in sky_at(&environment, SKY, &[at_elevation(-0.5), at_elevation(0.0)]) {
        assert_eq!(radiance, SKY);
    }
}
