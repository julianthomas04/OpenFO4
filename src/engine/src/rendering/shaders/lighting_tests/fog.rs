//! The distance and height fog of `fog.wgsl`, against the CPU model it mirrors and against the
//! values measured in vanilla Fallout 4.

use glam::Vec3;

use super::harness::{Case, Environment, Output};
use super::{assert_close, assert_close3, run_in};
use crate::rendering::lighting::fog::model_probe::{self as probe, ahead};
use crate::rendering::lighting::fog::{
    Fog, FogBlend, FogColors, FogParams, HeightBand, HeightBands, SunlightTerm,
};
use crate::rendering::world_streamer::SCALE;

fn fog_at(environment: &Environment, points: &[Vec3]) -> Vec<FogBlend> {
    let cases: Vec<Case> = points.iter().map(|point| Case::fog(*point)).collect();
    run_in(environment, &cases)
        .iter()
        .map(Output::color_alpha)
        .map(|(color, opacity)| FogBlend { color, opacity })
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

/// The shader's fog at every point is the CPU model's.
fn assert_matches_the_model(camera: Vec3, fog: &Fog, points: &[Vec3]) {
    let environment = Environment::with_fog(camera, fog);
    for (point, gpu) in points.iter().zip(fog_at(&environment, points)) {
        let cpu = fog.at(camera, *point);
        let row = format!("{point}");
        assert_close(gpu.opacity, cpu.opacity, &format!("{row} opacity"));
        if cpu.opacity > 0.0 {
            assert_close3(gpu.color, cpu.color, &format!("{row} colour"));
        }
    }
}

/// The fog over a black surface at each probe distance is the measured factor.
fn assert_measured_factors(fog: &FogParams, measured: &[(f32, f32)], tolerance: f32) {
    let points: Vec<Vec3> = measured
        .iter()
        .map(|&(distance, _)| ahead(distance))
        .collect();
    let environment = Environment::with_fog(Vec3::ZERO, &Fog::Enabled(*fog));
    let seen = surface_at(&environment, Vec3::ZERO, &points);
    for (&(distance, expected), seen) in measured.iter().zip(seen) {
        assert!(
            (seen.x - expected).abs() < tolerance,
            "d {distance}: {} against the measured {expected}",
            seen.x
        );
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn the_shader_reproduces_the_measured_distance_ramp() {
    assert_measured_factors(
        &probe::fog(probe::grey(), probe::DIST_BASE, true),
        &probe::DIST_BASE_FACTORS,
        0.015,
    );
}

#[test]
#[ignore = "needs a GPU adapter"]
fn the_shader_reproduces_the_measured_power() {
    assert_measured_factors(
        &probe::fog(probe::grey(), probe::DIST_P050, true),
        &probe::DIST_P050_FACTORS,
        0.015,
    );
}

#[test]
#[ignore = "needs a GPU adapter"]
fn the_shader_reproduces_the_measured_max_plateau_and_ramp_to_full_fog() {
    assert_measured_factors(
        &probe::fog(probe::grey(), probe::DIST_F3000_MAX050, true),
        &probe::DIST_F3000_MAX050_FACTORS,
        0.025,
    );
}

#[test]
#[ignore = "needs a GPU adapter"]
fn the_shader_weighs_near_and_far_colours_as_measured() {
    let fog = Fog::Enabled(probe::fog(
        probe::red_near_green_far(),
        probe::DIST_BASE,
        true,
    ));
    let environment = Environment::with_fog(Vec3::ZERO, &fog);
    let points: Vec<Vec3> = probe::DIST_BASE_COLOUR_WEIGHTS
        .iter()
        .map(|&(distance, _, _)| ahead(distance))
        .collect();
    let seen = surface_at(&environment, Vec3::ZERO, &points);
    for (&(distance, near, far), seen) in probe::DIST_BASE_COLOUR_WEIGHTS.iter().zip(seen) {
        assert!(
            (seen.x - near).abs() < 0.01,
            "near at {distance}: {}",
            seen.x
        );
        assert!((seen.y - far).abs() < 0.01, "far at {distance}: {}", seen.y);
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn the_shader_takes_the_night_factor_for_the_fill() {
    let fog = Fog::Enabled(probe::fog(probe::grey(), probe::DIST_BASE, false));
    let environment = Environment::with_fog(Vec3::ZERO, &fog);
    let seen = surface_at(&environment, Vec3::ZERO, &[ahead(9000.0)]);
    assert!((seen[0].x - probe::NIGHT_FILL).abs() < 0.005, "{}", seen[0]);
}

/// The probe's height rig: both bands at mid 17304 and range 500, half density at the top.
fn probe_height_fog() -> Fog {
    let band = HeightBand::new(17_304.0 * SCALE, 500.0 * SCALE);
    Fog::Enabled(FogParams {
        height: HeightBands {
            near: band,
            far: band,
            high_density_scale: 0.5,
        },
        ..probe::fog(probe::grey(), probe::DIST_BASE, true)
    })
}

#[test]
#[ignore = "needs a GPU adapter"]
fn the_shader_thins_the_fog_over_the_measured_height_profile() {
    let camera = Vec3::new(0.0, 17_304.0 * SCALE, 0.0);
    let environment = Environment::with_fog(camera, &probe_height_fog());
    let distance = 3000.0 * SCALE;
    let at_height = |bethesda_z: f32| {
        let rise = bethesda_z * SCALE - camera.y;
        camera + Vec3::new(0.0, rise, -(distance * distance - rise * rise).sqrt())
    };
    let below = at_height(16_000.0);
    let profile = [
        (16_609.0, 1.000),
        (17_087.0, 0.853),
        (17_421.0, 0.685),
        (17_777.0, 0.508),
        (18_167.0, 0.502),
    ];
    let mut points = vec![below];
    points.extend(profile.iter().map(|&(z, _)| at_height(z)));
    let opacity: Vec<f32> = fog_at(&environment, &points)
        .iter()
        .map(|blend| blend.opacity)
        .collect();
    for ((z, measured_ratio), seen) in profile.iter().zip(&opacity[1..]) {
        let ratio = seen / opacity[0];
        assert!(
            (ratio - measured_ratio).abs() < 0.025,
            "z {z}: {ratio} against the measured {measured_ratio}"
        );
    }
}

fn height_fog() -> Fog {
    Fog::Enabled(FogParams {
        colors: FogColors {
            near: Vec3::X,
            far: Vec3::Z,
            near_high: Vec3::Y,
            far_high: Vec3::ONE,
        },
        near: 0.0,
        far: 100.0,
        power: 1.5,
        max: 0.6,
        height: HeightBands {
            near: HeightBand::new(20.0, 10.0),
            far: HeightBand::new(50.0, 25.0),
            high_density_scale: 0.5,
        },
        sunlight: SunlightTerm {
            kept: 0.8,
            added: Vec3::new(0.05, 0.1, 0.15),
        },
    })
}

/// Before Near, the normal branch at several heights, the plateau, the ramp and the fill.
fn sample_points(camera: Vec3) -> Vec<Vec3> {
    let point = |height: f32, distance: f32| {
        camera + Vec3::new((distance * distance - height * height).sqrt(), height, 0.0)
    };
    vec![
        point(0.0, 0.0),
        point(5.0, 30.0),
        point(25.0, 50.0),
        point(45.0, 60.0),
        point(10.0, 75.0),
        point(60.0, 85.0),
        point(30.0, 95.0),
        point(70.0, 150.0),
    ]
}

#[test]
#[ignore = "needs a GPU adapter"]
fn the_shader_matches_the_model_in_every_branch() {
    let camera = Vec3::new(3.0, 0.0, -2.0);
    assert_matches_the_model(camera, &height_fog(), &sample_points(camera));
}

#[test]
#[ignore = "needs a GPU adapter"]
fn the_shader_matches_the_model_with_a_steep_power_and_no_cap() {
    let Fog::Enabled(fog) = height_fog() else {
        unreachable!("height_fog is enabled");
    };
    let steep = Fog::Enabled(FogParams {
        power: 0.25,
        max: 1.0,
        ..fog
    });
    assert_matches_the_model(Vec3::ZERO, &steep, &sample_points(Vec3::ZERO));
}

fn edge_fog(power: f32, max: f32, height: HeightBands) -> Fog {
    Fog::Enabled(FogParams {
        colors: FogColors {
            near: Vec3::X,
            far: Vec3::Z,
            near_high: Vec3::Y,
            far_high: Vec3::ONE,
        },
        near: 0.0,
        far: 100.0,
        power,
        max,
        height,
        sunlight: SunlightTerm {
            kept: 0.8,
            added: Vec3::splat(0.05),
        },
    })
}

#[test]
#[ignore = "needs a GPU adapter"]
fn the_shader_matches_the_model_on_the_branch_edges() {
    let at = |distance: f32| Vec3::new(0.0, 0.0, -distance);
    let flat = HeightBands::NONE;
    // Exactly at Far, and exactly at x == Max (d 50 with power 1 and Max 0.5).
    assert_matches_the_model(
        Vec3::ZERO,
        &edge_fog(1.0, 0.5, flat),
        &[at(100.0), at(50.0), at(49.9)],
    );
    // A zero power: no fog at the camera, the colour weight 1 everywhere after it.
    assert_matches_the_model(
        Vec3::ZERO,
        &edge_fog(0.0, 0.5, flat),
        &[at(0.0), at(1.0), at(90.0)],
    );
    // A band of range 0 is unset and leaves every height at its low colour.
    let zero_range = HeightBands {
        near: HeightBand::new(10.0, 0.0),
        far: HeightBand::new(10.0, 0.0),
        high_density_scale: 0.5,
    };
    assert_matches_the_model(
        Vec3::ZERO,
        &edge_fog(1.0, 1.0, zero_range),
        &[Vec3::new(0.0, 20.0, -40.0), Vec3::new(0.0, -20.0, -40.0)],
    );
}

#[test]
#[ignore = "needs a GPU adapter"]
fn the_shader_never_makes_a_dense_fog_more_than_full() {
    let dense = HeightBands {
        near: HeightBand::new(0.0, 1.0),
        far: HeightBand::new(0.0, 1.0),
        high_density_scale: 3.0,
    };
    let points = [Vec3::new(0.0, 50.0, -60.0), Vec3::new(0.0, 50.0, -80.0)];
    for fog in [edge_fog(1.0, 1.0, dense), edge_fog(1.0, 0.5, dense)] {
        let environment = Environment::with_fog(Vec3::ZERO, &fog);
        for blend in fog_at(&environment, &points) {
            assert_close(blend.opacity, 1.0, "dense fog");
        }
        assert_matches_the_model(Vec3::ZERO, &fog, &points);
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn the_shader_divides_a_too_short_ramp_by_the_minimum_like_the_model() {
    let Fog::Enabled(fog) = edge_fog(1.0, 1.0, HeightBands::NONE) else {
        unreachable!("edge_fog is enabled");
    };
    let short = Fog::Enabled(FogParams {
        near: 10.0,
        far: 10.0005,
        ..fog
    });
    assert_matches_the_model(Vec3::ZERO, &short, &[Vec3::new(0.0, 0.0, -10.0003)]);
}

#[test]
#[ignore = "needs a GPU adapter"]
fn disabled_fog_has_no_opacity_anywhere() {
    let environment = Environment::with_fog(Vec3::ZERO, &Fog::Disabled);
    for blend in fog_at(
        &environment,
        &[
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, -1.0e6),
            Vec3::new(5.0, 1.0e4, 0.0),
        ],
    ) {
        assert_eq!(blend, FogBlend::NONE);
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn a_surface_turns_towards_the_fog_colour_by_the_fog_opacity() {
    let camera = Vec3::new(0.0, 20.0, 0.0);
    let environment = Environment::with_fog(camera, &height_fog());
    let points = sample_points(camera);
    let fog = fog_at(&environment, &points);
    let surfaces = surface_at(&environment, Vec3::Y, &points);
    for ((point, blend), surface) in points.iter().zip(&fog).zip(&surfaces) {
        assert_close3(*surface, blend.apply(Vec3::Y), &format!("{point}"));
    }
}
