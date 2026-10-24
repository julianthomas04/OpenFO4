//! Invariants the engine relies on, swept over grids of edge-case inputs: finite and bounded
//! results, linearity in the light colour, zero outside the culled region.

#![allow(
    clippy::float_cmp,
    reason = "exact zeros and an exact value on the cone axis are part of the contract"
)]

use glam::{Quat, Vec3};

use super::harness::{Case, Environment, Light, Output, Surface, polar};
use super::{assert_close, assert_close3, run, run_in};
use crate::rendering::placed_light::{
    LIGHT_FLAG_ATTENUATION_ONLY, LIGHT_FLAG_IGNORE_ROUGHNESS, LIGHT_FLAG_NO_RIM_LIGHTING,
    LIGHT_FLAG_NON_SHADOW_SPOTLIGHT, LIGHT_FLAG_NON_SPECULAR, LIGHT_FLAG_SHADOW_HEMISPHERE,
    LIGHT_FLAG_SHADOW_OMNIDIRECTIONAL, LIGHT_FLAG_SHADOW_SPOTLIGHT,
};
use crate::rendering::shadows::lamps::table::SHADOW_SLOT_NONE;

/// Relative tolerance of an exactly linear relation: only rounding may differ.
const LINEARITY_TOLERANCE: f32 = 1e-5;

const POLAR_DEGREES: [f64; 9] = [0.01, 30.0, 60.0, 89.9, 90.0, 90.1, 120.0, 150.0, 179.99];
const AZIMUTH_DEGREES: [f64; 8] = [0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0];
const SMOOTHNESS_VALUES: [f32; 5] = [-1.0, 0.0, 0.35, 1.0, 2.0];
const INTENSITY_VALUES: [f32; 3] = [0.0, 1.0, 1e6];
const MAX_SPECULAR_PEAK: f32 = 15.0 * std::f32::consts::PI;
const SPECULAR_CEILINGS: [f32; 2] = [1.0, 8.0];

/// Unit vectors including every axis (so dot products with +Z of exactly 0, 1 and -1), the
/// horizon and directions on either side of it.
fn directions() -> Vec<Vec3> {
    let mut all = vec![
        Vec3::Z,
        Vec3::NEG_Z,
        Vec3::X,
        Vec3::NEG_X,
        Vec3::Y,
        Vec3::NEG_Y,
    ];
    for &polar_degrees in &POLAR_DEGREES {
        for &azimuth_degrees in &AZIMUTH_DEGREES {
            all.push(polar(polar_degrees, azimuth_degrees));
        }
    }
    all
}

/// A thinner direction set for sweeps with many more parameters.
fn coarse_directions() -> Vec<Vec3> {
    let mut all = vec![Vec3::Z, Vec3::NEG_Z];
    for polar_degrees in [45.0, 90.0, 135.0] {
        for azimuth_degrees in [0.0, 90.0, 180.0] {
            all.push(polar(polar_degrees, azimuth_degrees));
        }
    }
    all
}

fn tilted(direction: Vec3) -> Vec3 {
    Quat::from_axis_angle(Vec3::new(1.0, 2.0, 3.0).normalize(), 0.7) * direction
}

fn surfaces(smoothness: &[f32], intensities: &[f32], albedos: &[Vec3]) -> Vec<Surface> {
    let mut all = Vec::new();
    for &smoothness in smoothness {
        for &spec_intensity in intensities {
            for &albedo in albedos {
                all.push(Surface {
                    albedo,
                    smoothness,
                    spec_intensity,
                    ..Surface::default()
                });
            }
        }
    }
    all
}

fn assert_finite_and_non_negative(outputs: &[Output], what: &str) {
    for (index, output) in outputs.iter().enumerate() {
        for value in [output.diffuse, output.specular] {
            assert!(
                value.is_finite() && value.min_element() >= 0.0,
                "{what} case {index}: {output:?}"
            );
        }
    }
}

fn assert_scaled(scaled: Vec3, base: Vec3, scale: f32, row: &str) {
    let expected = base * scale;
    for channel in 0..3 {
        let slack = LINEARITY_TOLERANCE * expected[channel].abs();
        assert!(
            (scaled[channel] - expected[channel]).abs() <= slack,
            "{row} channel {channel}: got {}, expected {}",
            scaled[channel],
            expected[channel]
        );
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn sun_is_finite_non_negative_and_bounded_on_every_edge_input() {
    let albedos = [Vec3::ZERO, Vec3::new(0.2, 0.7, 1.0)];
    let mut cases = Vec::new();
    for surface in surfaces(&SMOOTHNESS_VALUES, &INTENSITY_VALUES, &albedos) {
        for rotate in [false, true] {
            for &l in &directions() {
                for &v in &directions() {
                    let place =
                        |direction: Vec3| if rotate { tilted(direction) } else { direction };
                    let surface = Surface {
                        normal: place(Vec3::Z),
                        ..surface
                    };
                    cases.push(Case::direct(&surface, place(l), place(v), Vec3::ONE));
                }
            }
        }
    }
    for ceiling in SPECULAR_CEILINGS {
        let outputs = run_in(&Environment::with_specular_ceiling(ceiling), &cases);
        assert_finite_and_non_negative(&outputs, "sun");
        let peak = MAX_SPECULAR_PEAK * ceiling + 1e-3;
        for output in &outputs {
            assert!(
                output.specular.max_element() <= peak,
                "specular peak {output:?}"
            );
        }
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn sun_is_dark_when_the_normal_faces_away_from_the_light() {
    let mut cases = Vec::new();
    let mut facing_away = Vec::new();
    for surface in surfaces(&SMOOTHNESS_VALUES, &[1.0], &[Vec3::ONE]) {
        for &l in &directions() {
            for &v in &directions() {
                cases.push(Case::direct(&surface, l, v, Vec3::ONE));
                facing_away.push(l.z <= 0.0);
            }
        }
    }
    for ((output, dark), case) in run(&cases).iter().zip(facing_away).zip(0..) {
        if dark {
            assert_eq!(output.diffuse, Vec3::ZERO, "diffuse of case {case}");
            assert_eq!(output.specular, Vec3::ZERO, "specular of case {case}");
        }
    }
}

const OPPOSING_OFFSETS: [f32; 6] = [0.0, 1e-6, 5e-6, 2e-5, 5e-5, 9e-5];
const BELOW_HORIZON_DEGREES: [f64; 4] = [0.0, 0.5, 5.0, 30.0];
const HALF_VECTOR_LIMIT: f32 = 1e-4;

#[test]
#[ignore = "needs a GPU adapter"]
fn sun_has_no_specular_when_light_and_viewer_nearly_oppose() {
    let mut cases = Vec::new();
    for surface in surfaces(&[0.35, 1.0], &[1.0], &[Vec3::ONE]) {
        for rotate in [false, true] {
            let place = |direction: Vec3| if rotate { tilted(direction) } else { direction };
            let normal = place(Vec3::Z);
            let surface = Surface { normal, ..surface };
            for &below_horizon in &BELOW_HORIZON_DEGREES {
                for &azimuth in &AZIMUTH_DEGREES {
                    let v = place(polar(90.0 + below_horizon, azimuth));
                    for &offset in &OPPOSING_OFFSETS {
                        let l = (-v + normal * offset).normalize();
                        if (l + v).length() < HALF_VECTOR_LIMIT && l.dot(normal) > 0.0 {
                            cases.push(Case::direct(&surface, l, v, Vec3::ONE));
                        }
                    }
                }
            }
        }
    }
    assert!(
        cases.len() > 300,
        "only {} degenerate configurations",
        cases.len()
    );
    for (index, output) in run(&cases).iter().enumerate() {
        assert_eq!(output.specular, Vec3::ZERO, "case {index}");
    }
}

/// Unit vectors spread evenly over the sphere that are not axis-aligned, so their dot products
/// carry rounding errors.
fn spiral_directions(count: usize) -> Vec<Vec3> {
    let golden_angle = std::f32::consts::PI * (3.0 - 5.0_f32.sqrt());
    (0..count)
        .map(|index| {
            let z = 1.0 - 2.0 * (index as f32 + 0.5) / count as f32;
            let ring = (1.0 - z * z).sqrt();
            let angle = golden_angle * index as f32;
            Vec3::new(ring * angle.cos(), ring * angle.sin(), z).normalize()
        })
        .collect()
}

#[test]
#[ignore = "needs a GPU adapter"]
fn sun_is_finite_for_directions_that_are_not_exactly_unit_length() {
    let scales = [1.0 + 2.0 * f32::EPSILON, 1.0 - 2.0 * f32::EPSILON];
    let mut cases = Vec::new();
    for surface in surfaces(&[0.0, 0.5, 1.0], &[1.0], &[Vec3::ONE]) {
        for scale in scales {
            for &l in &directions() {
                for &v in &directions() {
                    cases.push(Case::direct(&surface, l * scale, v * scale, Vec3::ONE));
                }
            }
        }
        let spiral = spiral_directions(24);
        for &normal in &spiral {
            let surface = Surface { normal, ..surface };
            for &v in &spiral {
                cases.push(Case::direct(&surface, normal, v, Vec3::ONE));
                for &l in &spiral {
                    cases.push(Case::direct(&surface, l, v, Vec3::ONE));
                }
            }
        }
    }
    assert_finite_and_non_negative(&run(&cases), "sun");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn a_negative_specular_ceiling_leaves_no_specular() {
    let environment = Environment {
        ambient_cube: [Vec3::splat(0.4); 6],
        ..Environment::with_specular_ceiling(-1.0)
    };
    let surface = Surface::default();
    let outputs = run_in(
        &environment,
        &[
            Case::direct(&surface, polar(40.0, 0.0), polar(40.0, 180.0), Vec3::ONE),
            Case::ambient(&surface, polar(70.0, 0.0)),
        ],
    );
    for output in outputs {
        assert_eq!(output.specular, Vec3::ZERO);
    }
}

const LOCAL_TYPE_FLAGS: [u32; 5] = [
    0,
    LIGHT_FLAG_SHADOW_SPOTLIGHT,
    LIGHT_FLAG_SHADOW_HEMISPHERE,
    LIGHT_FLAG_SHADOW_OMNIDIRECTIONAL,
    LIGHT_FLAG_NON_SHADOW_SPOTLIGHT,
];
const LOCAL_BEHAVIOUR_FLAGS: [u32; 4] = [
    LIGHT_FLAG_NON_SPECULAR,
    LIGHT_FLAG_ATTENUATION_ONLY,
    LIGHT_FLAG_IGNORE_ROUGHNESS,
    LIGHT_FLAG_NO_RIM_LIGHTING,
];

/// Every combination of a light type, behaviour flags and with or without a lamp-shadow slot.
fn local_lights() -> Vec<Light> {
    let mut all = Vec::new();
    for &type_flags in &LOCAL_TYPE_FLAGS {
        for mask in 0..1u32 << LOCAL_BEHAVIOUR_FLAGS.len() {
            let behaviour = LOCAL_BEHAVIOUR_FLAGS
                .iter()
                .enumerate()
                .filter(|(bit, _)| mask & (1 << bit) != 0)
                .fold(0, |flags, (_, flag)| flags | flag);
            for shadow_slot in [SHADOW_SLOT_NONE, 3] {
                all.push(Light {
                    shadow_slot,
                    ..Light::with_flags(type_flags | behaviour)
                });
            }
        }
    }
    all
}

fn local_cases(color: Vec3, surfaces: &[Surface]) -> Vec<Case> {
    let mut cases = Vec::new();
    for light in local_lights() {
        for surface in surfaces {
            for &l in &coarse_directions() {
                for &v in &coarse_directions() {
                    cases.push(Case::local(surface, &light, l, v, color));
                }
            }
        }
    }
    cases
}

#[test]
#[ignore = "needs a GPU adapter"]
fn local_lights_are_finite_and_non_negative_for_every_flag_combination() {
    let albedos = [Vec3::ZERO, Vec3::new(0.3, 0.6, 0.9)];
    let cases = local_cases(
        Vec3::ONE,
        &surfaces(&[0.0, 0.35, 1.0, 2.0], &[1.0, 1e6], &albedos),
    );
    assert_finite_and_non_negative(&run(&cases), "local light");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn local_lights_scale_linearly_with_their_colour_including_negative_and_zero() {
    let surface = [Surface {
        albedo: Vec3::new(0.3, 0.6, 0.9),
        smoothness: 0.6,
        ..Surface::default()
    }];
    let base_color = Vec3::new(1.0, 0.5, 0.25);
    let base = run(&local_cases(base_color, &surface));
    for scale in [0.0, -1.0, 2.0, 0.37, 1000.0] {
        let scaled = run(&local_cases(base_color * scale, &surface));
        for (index, (scaled, base)) in scaled.iter().zip(&base).enumerate() {
            let row = format!("scale {scale} case {index}");
            assert_scaled(scaled.diffuse, base.diffuse, scale, &row);
            assert_scaled(scaled.specular, base.specular, scale, &row);
            if scale == 0.0 {
                assert_eq!(scaled.diffuse, Vec3::ZERO, "{row}");
                assert_eq!(scaled.specular, Vec3::ZERO, "{row}");
            }
        }
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn sun_scales_linearly_with_its_colour_including_negative_and_zero() {
    let surface = Surface {
        albedo: Vec3::new(0.3, 0.6, 0.9),
        smoothness: 0.6,
        ..Surface::default()
    };
    let cases = |color: Vec3| -> Vec<Case> {
        let mut cases = Vec::new();
        for &l in &directions() {
            for &v in &directions() {
                cases.push(Case::direct(&surface, l, v, color));
            }
        }
        cases
    };
    let base_color = Vec3::new(1.0, 0.5, 0.25);
    let base = run(&cases(base_color));
    for scale in [0.0, -1.0, 2.0, 0.37] {
        for (index, (scaled, base)) in run(&cases(base_color * scale))
            .iter()
            .zip(&base)
            .enumerate()
        {
            let row = format!("scale {scale} case {index}");
            assert_scaled(scaled.diffuse, base.diffuse, scale, &row);
            assert_scaled(scaled.specular, base.specular, scale, &row);
        }
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn attenuation_only_lights_ignore_the_geometry_and_have_no_specular() {
    let surface = Surface {
        albedo: Vec3::new(0.3, 0.6, 0.9),
        ..Surface::default()
    };
    let color = Vec3::new(0.5, 1.5, 2.0);
    let light = Light::with_flags(LIGHT_FLAG_ATTENUATION_ONLY);
    let mut cases = Vec::new();
    for &l in &directions() {
        for &v in &directions() {
            cases.push(Case::local(&surface, &light, l, v, color));
        }
    }
    for output in run(&cases) {
        assert_close3(
            output.diffuse,
            surface.albedo * color,
            "attenuation-only diffuse",
        );
        assert_eq!(output.specular, Vec3::ZERO);
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn specular_intensity_is_linear_below_the_ceiling_and_saturates_above_it() {
    let l = polar(40.0, 0.0);
    let v = polar(40.0, 180.0);
    let specular_at = |environment: &Environment, intensity: f32| {
        let surface = Surface {
            spec_intensity: intensity,
            ..Surface::default()
        };
        run_in(environment, &[Case::direct(&surface, l, v, Vec3::ONE)])[0]
            .specular
            .x
    };
    for ceiling in SPECULAR_CEILINGS {
        let environment = Environment::with_specular_ceiling(ceiling);
        let unit = specular_at(&environment, 1.0);
        for fraction in [0.0, 0.25, 0.5, 1.0] {
            assert_close(
                specular_at(&environment, fraction * ceiling),
                unit * fraction * ceiling,
                &format!("intensity {} under ceiling {ceiling}", fraction * ceiling),
            );
        }
        for beyond in [ceiling * 1.5, 100.0, 1e9] {
            assert_close(
                specular_at(&environment, beyond),
                unit * ceiling,
                &format!("intensity {beyond} over ceiling {ceiling}"),
            );
        }
        assert_close(specular_at(&environment, -3.0), 0.0, "negative intensity");
    }
}

const AMBIENT_CUBES: [[Vec3; 6]; 5] = [
    [Vec3::ZERO; 6],
    [Vec3::splat(0.5); 6],
    [
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(0.0, 0.0, 0.0),
    ],
    [
        Vec3::new(2.0, 0.1, 0.0),
        Vec3::new(0.0, 0.0, 0.3),
        Vec3::new(0.01, 0.5, 0.0),
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(1.0, 1.0, 1.0),
        Vec3::new(0.0, 0.7, 0.0),
    ],
    [Vec3::splat(100.0); 6],
];

#[test]
#[ignore = "needs a GPU adapter"]
fn ambient_is_finite_and_non_negative_for_every_cube_and_view() {
    let mut cases = Vec::new();
    for surface in surfaces(&SMOOTHNESS_VALUES, &INTENSITY_VALUES, &[Vec3::ONE]) {
        for &normal in &directions() {
            for &v in &directions() {
                let surface = Surface { normal, ..surface };
                cases.push(Case::ambient(&surface, v));
            }
        }
    }
    for cube in AMBIENT_CUBES {
        let environment = Environment {
            max_specular_intensity: Some(8.0),
            ambient_cube: cube,
        };
        assert_finite_and_non_negative(&run_in(&environment, &cases), "ambient");
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn ambient_vanishes_without_light_and_does_not_depend_on_albedo_for_the_specular() {
    let cases = [
        Case::ambient(&Surface::default(), polar(60.0, 0.0)),
        Case::ambient(
            &Surface {
                albedo: Vec3::ZERO,
                ..Surface::default()
            },
            polar(60.0, 0.0),
        ),
    ];
    let dark = run_in(&Environment::default(), &cases);
    assert_eq!(
        dark[0],
        Output {
            diffuse: Vec3::ZERO,
            specular: Vec3::ZERO
        }
    );

    let lit = Environment {
        ambient_cube: [Vec3::splat(0.4); 6],
        ..Environment::default()
    };
    let outputs = run_in(&lit, &cases);
    assert_close3(outputs[1].diffuse, Vec3::ZERO, "black albedo");
    assert_close3(
        outputs[0].specular,
        outputs[1].specular,
        "specular ignores albedo",
    );
}

const ATTENUATION_CONSTANTS: [f32; 5] = [-1.0, 0.0, 0.5, 1.0, 5.0];
const ATTENUATION_SCALARS: [f32; 4] = [-2.0, 0.0, 1.0, 1e3];
const ATTENUATION_EXPONENTS: [f32; 8] = [-1e6, -1e3, -3.0, 0.0, 0.5, 2.0, 100.0, 1e6];
const ATTENUATION_RADII: [f32; 3] = [0.5, 100.0, 1e4];
const DISTANCE_FRACTIONS: [f32; 7] = [1e-4 / 1e4, 1e-3, 0.25, 0.5, 0.9, 0.999, 0.999_999];

fn attenuation_lights() -> Vec<Light> {
    let mut all = Vec::new();
    for &constant in &ATTENUATION_CONSTANTS {
        for &scalar in &ATTENUATION_SCALARS {
            for &exponent in &ATTENUATION_EXPONENTS {
                for &radius in &ATTENUATION_RADII {
                    all.push(Light {
                        radius,
                        ..Light::attenuated(constant, scalar, exponent)
                    });
                }
            }
        }
    }
    all
}

#[test]
#[ignore = "needs a GPU adapter"]
fn attenuation_is_a_finite_fraction_inside_the_radius_and_zero_from_it_outwards() {
    let mut cases = Vec::new();
    let mut inside = Vec::new();
    for light in attenuation_lights() {
        for &fraction in &DISTANCE_FRACTIONS {
            cases.push(Case::attenuation(
                &light,
                (fraction * light.radius).max(1e-4),
            ));
            inside.push(true);
        }
        for outside in [1.0, 1.5, 1e3] {
            cases.push(Case::attenuation(&light, outside * light.radius));
            inside.push(false);
        }
    }
    for (index, (output, inside)) in run(&cases).iter().zip(inside).enumerate() {
        let value = output.scalar();
        assert!(
            value.is_finite() && (0.0..=1.0).contains(&value),
            "case {index}: {value}"
        );
        if !inside {
            assert_eq!(value, 0.0, "case {index} outside the radius");
        }
    }
}

/// Falloff of a light that has lost `lost` of its strength.
fn falloff_after_losing(lost: f32) -> f32 {
    (1.0 - lost.clamp(0.0, 1.0)).powf(2.2)
}

/// Falloff of a light whose record loses `record_loss`, at `distance` from a light of `radius`:
/// the radius window `normalised^2` is a floor under the loss.
fn windowed_falloff(record_loss: f32, distance: f32, radius: f32) -> f32 {
    let normalised = distance / radius;
    falloff_after_losing(record_loss.max(normalised * normalised))
}

#[test]
#[ignore = "needs a GPU adapter"]
fn attenuation_without_a_scalar_is_the_constant_term_or_the_radius_window() {
    let mut cases = Vec::new();
    let mut expected = Vec::new();
    for light in attenuation_lights()
        .into_iter()
        .filter(|light| light.attenuation.y == 0.0)
    {
        for &fraction in &DISTANCE_FRACTIONS {
            let distance = (fraction * light.radius).max(1e-4);
            cases.push(Case::attenuation(&light, distance));
            expected.push(windowed_falloff(
                light.attenuation.x,
                distance,
                light.radius,
            ));
        }
    }
    for (index, (output, expected)) in run(&cases).iter().zip(expected).enumerate() {
        assert_close(output.scalar(), expected, &format!("case {index}"));
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn attenuation_with_a_zero_exponent_depends_on_the_distance_only_through_the_radius_window() {
    let mut cases = Vec::new();
    let mut expected = Vec::new();
    for (constant, scalar) in [(0.0, 1.0), (0.2, 0.3), (0.5, 0.25), (-0.5, 1.0), (0.0, 0.5)] {
        let light = Light::attenuated(constant, scalar, 0.0);
        for fraction in [0.1, 0.5, 0.9] {
            let distance = fraction * light.radius;
            cases.push(Case::attenuation(&light, distance));
            expected.push(windowed_falloff(constant + scalar, distance, light.radius));
        }
    }
    for (index, (output, expected)) in run(&cases).iter().zip(expected).enumerate() {
        assert_close(output.scalar(), expected, &format!("case {index}"));
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn attenuation_without_a_floor_term_falls_monotonically_with_distance() {
    let light = Light::default();
    let cases: Vec<Case> = (1..100)
        .map(|step| Case::attenuation(&light, step as f32))
        .collect();
    let values: Vec<f32> = run(&cases).iter().map(Output::scalar).collect();
    assert!(
        values.windows(2).all(|pair| pair[1] <= pair[0]),
        "{values:?}"
    );
}

/// Attenuation triples (constant, scalar, exponent) of vanilla-like records.
const VANILLA_ATTENUATIONS: [[f32; 3]; 10] = [
    [0.0, 1.0, 2.0],
    [0.0, 0.0, 0.0],
    [0.0, 0.0, 2.0],
    [0.0, 1.0, 0.25],
    [0.0, 1.0, 1.0],
    [0.0, 1.0, 22.0],
    [0.0, 1.0, 100.0],
    [0.1, 1.0, 0.8],
    [0.25, 1.0, 2.0],
    [0.3, 0.5, 1.0],
];
/// Slack of the comparison against a bound the shader may only reach, in falloff units.
const EDGE_BOUND_TOLERANCE: f32 = 1e-5;
/// Slack of a monotonicity check against rounding of the GPU's `pow`, in falloff units.
const MONOTONICITY_TOLERANCE: f32 = 1e-6;

/// Distances over the radius that close in on the radius: 1 - 2^-k for k in 1..=20.
fn radius_approach() -> Vec<f32> {
    (1..=20).map(|k| 1.0 - 0.5_f32.powi(k)).collect()
}

fn edge_bound_lights() -> Vec<Light> {
    let vanilla = VANILLA_ATTENUATIONS
        .iter()
        .map(|&[constant, scalar, exponent]| Light::attenuated(constant, scalar, exponent));
    attenuation_lights().into_iter().chain(vanilla).collect()
}

#[test]
#[ignore = "needs a GPU adapter"]
fn attenuation_never_exceeds_the_radius_window_when_approaching_the_radius() {
    let mut cases = Vec::new();
    let mut bounds = Vec::new();
    for light in edge_bound_lights() {
        for fraction in radius_approach() {
            let distance = fraction * light.radius;
            cases.push(Case::attenuation(&light, distance));
            bounds.push(falloff_after_losing((distance / light.radius).powi(2)));
        }
    }
    for (index, (output, bound)) in run(&cases).iter().zip(bounds).enumerate() {
        let value = output.scalar();
        assert!(
            value <= bound + EDGE_BOUND_TOLERANCE,
            "case {index}: {value} over the window {bound}"
        );
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn attenuation_of_the_default_triple_is_the_radius_window() {
    let light = Light::default();
    let fractions = (1..100)
        .map(|step| step as f32 / 100.0)
        .chain(radius_approach());
    let mut cases = Vec::new();
    let mut expected = Vec::new();
    for fraction in fractions {
        cases.push(Case::attenuation(&light, fraction * light.radius));
        expected.push(falloff_after_losing(fraction * fraction));
    }
    for (index, (output, expected)) in run(&cases).iter().zip(expected).enumerate() {
        assert_close(output.scalar(), expected, &format!("case {index}"));
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn attenuation_of_a_light_without_a_scalar_fades_out_towards_the_radius() {
    const FRACTIONS: [f32; 6] = [0.75, 0.8, 0.9, 0.95, 0.99, 0.999];
    let mut cases = Vec::new();
    let mut lights = Vec::new();
    for constant in [0.0, 0.25, 0.5] {
        for exponent in [0.0, 2.0] {
            let light = Light::attenuated(constant, 0.0, exponent);
            for fraction in FRACTIONS {
                cases.push(Case::attenuation(&light, fraction * light.radius));
            }
            lights.push((constant, exponent));
        }
    }
    let outputs = run(&cases);
    for ((constant, exponent), values) in lights.iter().zip(outputs.chunks(FRACTIONS.len())) {
        let values: Vec<f32> = values.iter().map(Output::scalar).collect();
        let row = format!("c={constant} e={exponent}: {values:?}");
        assert!(values.windows(2).all(|pair| pair[1] < pair[0]), "{row}");
        assert!(values[FRACTIONS.len() - 1] < 1e-3, "{row}");
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn attenuation_with_a_non_negative_scalar_and_exponent_never_rises_with_distance() {
    let mut fractions = vec![1e-3, 0.01, 0.05, 0.1, 0.2, 0.3, 0.5, 0.7, 0.9];
    fractions.extend(radius_approach());
    fractions.sort_by(f32::total_cmp);
    fractions.dedup();
    let lights: Vec<Light> = attenuation_lights()
        .into_iter()
        .filter(|light| light.attenuation.y >= 0.0 && light.attenuation.z >= 0.0)
        .collect();
    let cases: Vec<Case> = lights
        .iter()
        .flat_map(|light| {
            fractions
                .iter()
                .map(|&fraction| Case::attenuation(light, fraction * light.radius))
        })
        .collect();
    let outputs = run(&cases);
    for (light, values) in lights.iter().zip(outputs.chunks(fractions.len())) {
        let values: Vec<f32> = values.iter().map(Output::scalar).collect();
        assert!(
            values
                .windows(2)
                .all(|pair| pair[1] <= pair[0] + MONOTONICITY_TOLERANCE),
            "{:?} radius {}: {values:?}",
            light.attenuation,
            light.radius
        );
    }
}

const COSINE_OUTER_VALUES: [f32; 10] = [
    -1.0,
    -0.5,
    0.0,
    0.1,
    0.5,
    std::f32::consts::FRAC_1_SQRT_2,
    0.9,
    0.9999,
    0.999_999_9,
    1.0,
];
const CONE_EXPONENTS: [f32; 6] = [-1.0, 0.0, 0.5, 1.0, 4.0, 100.0];
const CONE_MARGIN_DEGREES: f64 = 0.5;

fn spot_variants(cos_outer: f32, exponent: f32) -> Vec<Light> {
    [
        (LIGHT_FLAG_NON_SHADOW_SPOTLIGHT, SHADOW_SLOT_NONE),
        (LIGHT_FLAG_SHADOW_SPOTLIGHT, SHADOW_SLOT_NONE),
        (LIGHT_FLAG_SHADOW_SPOTLIGHT, 0),
        (
            LIGHT_FLAG_SHADOW_SPOTLIGHT | LIGHT_FLAG_NON_SHADOW_SPOTLIGHT,
            7,
        ),
    ]
    .map(|(flags, shadow_slot)| Light {
        flags,
        spot_exponent: exponent,
        spot_cos_outer: cos_outer,
        shadow_slot,
        ..Light::default()
    })
    .to_vec()
}

#[test]
#[ignore = "needs a GPU adapter"]
fn spot_cone_is_a_finite_fraction_and_zero_outside_the_cone() {
    let mut cases = Vec::new();
    let mut outside = Vec::new();
    for &cos_outer in &COSINE_OUTER_VALUES {
        let half_angle = f64::from(cos_outer).acos().to_degrees();
        for &exponent in &CONE_EXPONENTS {
            for light in spot_variants(cos_outer, exponent) {
                for step in 0..=72 {
                    let off_axis = f64::from(step) * 2.5;
                    for azimuth in [0.0, 120.0, 250.0] {
                        cases.push(Case::cone(&light, polar(off_axis, azimuth)));
                        outside.push(off_axis > half_angle + CONE_MARGIN_DEGREES);
                    }
                }
            }
        }
    }
    for (index, (output, outside)) in run(&cases).iter().zip(outside).enumerate() {
        let value = output.scalar();
        assert!(
            value.is_finite() && (0.0..=1.0 + 1e-6).contains(&value),
            "case {index}: {value}"
        );
        if outside {
            assert!(value <= 1e-6, "case {index} outside the cone: {value}");
        }
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn spot_cone_is_full_on_the_axis() {
    for exponent in [0.5, 1.0, 4.0] {
        for light in spot_variants(std::f32::consts::FRAC_1_SQRT_2, exponent) {
            let on_axis = run(&[Case::cone(&light, Vec3::Z)])[0].scalar();
            assert_close(
                on_axis,
                1.0,
                &format!("axis, exponent {exponent}, flags {:#x}", light.flags),
            );
        }
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn hemisphere_and_omni_cones_are_binary() {
    let mut cases = Vec::new();
    for light in [
        Light::with_flags(LIGHT_FLAG_SHADOW_HEMISPHERE),
        Light::with_flags(LIGHT_FLAG_SHADOW_OMNIDIRECTIONAL),
        Light::with_flags(0),
    ] {
        for &direction in &directions() {
            cases.push((light.flags, direction, Case::cone(&light, direction)));
        }
    }
    let outputs = run(&cases.iter().map(|(_, _, case)| *case).collect::<Vec<_>>());
    for ((flags, direction, _), output) in cases.iter().zip(outputs) {
        let expected = if *flags == LIGHT_FLAG_SHADOW_HEMISPHERE && direction.z < 0.0 {
            0.0
        } else {
            1.0
        };
        assert_eq!(
            output.scalar(),
            expected,
            "flags {flags:#x} towards {direction}"
        );
    }
}
