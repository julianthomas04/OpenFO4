//! Expected values for sample inputs, run through the lighting functions of the object shader.

#![allow(
    clippy::unreadable_literal,
    clippy::excessive_precision,
    clippy::approx_constant,
    reason = "sample values are written with every digit they carry"
)]
#![allow(clippy::float_cmp, reason = "exact zeros are part of the contract")]

use glam::Vec3;

use super::harness::{Case, Environment, Light, Output, Surface, polar};
use super::{assert_close, assert_close3, run, run_in};
use crate::rendering::placed_light::{
    LIGHT_FLAG_ATTENUATION_ONLY, LIGHT_FLAG_IGNORE_ROUGHNESS, LIGHT_FLAG_NO_RIM_LIGHTING,
    LIGHT_FLAG_NON_SHADOW_SPOTLIGHT, LIGHT_FLAG_NON_SPECULAR, LIGHT_FLAG_SHADOW_HEMISPHERE,
    LIGHT_FLAG_SHADOW_OMNIDIRECTIONAL, LIGHT_FLAG_SHADOW_SPOTLIGHT,
};
use crate::rendering::shadows::lamps::table::SHADOW_SLOT_NONE;

/// Polar and azimuth angles in degrees of a direction.
type Angles = (f64, f64);

/// Off-axis angle in degrees and the cone factor expected there.
type ConeSample = (f32, f32);

/// Half-angle in degrees, falloff exponent and the samples of one cone.
type ConeProfile = (f32, f32, &'static [ConeSample]);

struct Configuration {
    name: &'static str,
    light: Angles,
    viewer: Angles,
}

const fn configuration(name: &'static str, light: Angles, viewer: Angles) -> Configuration {
    Configuration {
        name,
        light,
        viewer,
    }
}

/// Light and viewer configurations of the shared-response samples.
const CONFIGURATIONS: [Configuration; 12] = [
    configuration("head-on", (0.0, 0.0), (0.0, 0.0)),
    configuration("light 45, viewer on the normal", (45.0, 0.0), (0.0, 0.0)),
    configuration("mirrored 45", (45.0, 0.0), (45.0, 180.0)),
    configuration("same side 45", (45.0, 0.0), (45.0, 0.0)),
    configuration("60 and 30, azimuths 90 apart", (60.0, 0.0), (30.0, 90.0)),
    configuration("grazing opposite sides", (80.0, 0.0), (80.0, 180.0)),
    configuration("75 and 20, azimuths 135 apart", (75.0, 135.0), (20.0, 0.0)),
    configuration("light at 89", (89.0, 0.0), (10.0, 0.0)),
    configuration("light below the horizon", (120.0, 0.0), (20.0, 0.0)),
    configuration("mirror at 40", (40.0, 0.0), (40.0, 180.0)),
    configuration("viewer grazing opposite", (30.0, 180.0), (85.0, 0.0)),
    configuration("viewer below the surface", (30.0, 0.0), (100.0, 0.0)),
];

const SMOOTHNESS_COLUMNS: [f32; 4] = [0.0, 0.35, 0.7, 1.0];

/// Diffuse and specular of each configuration at each of `SMOOTHNESS_COLUMNS`.
#[rustfmt::skip]
const SHARED_RESPONSE: [[(f32, f32); 4]; 12] = [
    [(0.68153, 0.10000), (0.78715, 0.61569), (0.93182, 6.45000), (1.00000, 47.12389)],
    [(0.48191, 0.06036), (0.55660, 0.07258), (0.65889, 0.00000), (0.70711, 0.00000)],
    [(0.48191, 0.10086), (0.55660, 0.62099), (0.65889, 6.50561), (0.70711, 33.32162)],
    [(0.58512, 0.05000), (0.64935, 0.00024), (0.71514, 0.00000), (0.70711, 0.00000)],
    [(0.34076, 0.03585), (0.39358, 0.00264), (0.46591, 0.00000), (0.50000, 0.00000)],
    [(0.28121, 0.25413), (0.24255, 1.56464), (0.21067, 8.18298), (0.17365, 8.18298)],
    [(0.17639, 0.01214), (0.20373, 0.00223), (0.24117, 0.00000), (0.25882, 0.00000)],
    [(0.01211, 0.00002), (0.01394, 0.00000), (0.01638, 0.00000), (0.01745, 0.00000)],
    [(0.0, 0.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)],
    [(0.52208, 0.10028), (0.60299, 0.61741), (0.71381, 6.46808), (0.76604, 36.09899)],
    [(0.95589, 0.24406), (0.91938, 0.12670), (0.91668, 0.00000), (0.86603, 0.00000)],
    [(0.69032, 0.01597), (0.77164, 0.00000), (0.86153, 0.00000), (0.86603, 0.00000)],
];

fn direction((polar_degrees, azimuth_degrees): Angles) -> Vec3 {
    polar(polar_degrees, azimuth_degrees)
}

#[test]
#[ignore = "needs a GPU adapter"]
fn sun_response_matches_expected_samples() {
    let mut cases = Vec::new();
    let mut rows = Vec::new();
    for (config, columns) in CONFIGURATIONS.iter().zip(&SHARED_RESPONSE) {
        for (&smoothness, &expected) in SMOOTHNESS_COLUMNS.iter().zip(columns) {
            cases.push(Case::direct(
                &Surface::glossy(smoothness),
                direction(config.light),
                direction(config.viewer),
                Vec3::ONE,
            ));
            rows.push((format!("{} g={smoothness}", config.name), expected));
        }
    }
    for (output, (row, (diffuse, specular))) in run(&cases).iter().zip(&rows) {
        assert_close(output.diffuse.x, *diffuse, &format!("{row} diffuse"));
        assert_close(output.specular.x, *specular, &format!("{row} specular"));
        assert_close3(output.specular, Vec3::splat(output.specular.x), row);
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn sun_tints_the_diffuse_by_albedo_and_the_specular_by_the_light_only() {
    let surface = Surface {
        albedo: Vec3::new(0.6, 0.5, 0.4),
        smoothness: 0.35,
        ..Surface::default()
    };
    let color = Vec3::new(1.0, 0.5, 0.25);
    let case = Case::direct(
        &surface,
        direction((45.0, 0.0)),
        direction((45.0, 180.0)),
        color,
    );
    let output = run(&[case])[0];
    assert_close3(
        output.diffuse,
        Vec3::new(0.6, 0.25, 0.1) * 0.55660,
        "tinted diffuse",
    );
    assert_close3(output.specular, color * 0.62099, "tinted specular");
}

/// Indices into `CONFIGURATIONS` of the flag samples: table 2 rows 3, 6, 11, 12 and 9.
const FLAG_CONFIGURATIONS: [usize; 5] = [2, 5, 10, 11, 8];

/// A flag combination and the diffuse and specular of a plain point light at smoothness 0.35
/// under it, one sample per entry of `FLAG_CONFIGURATIONS`.
struct FlagColumn {
    name: &'static str,
    flags: u32,
    samples: [(f32, f32); 5],
}

const fn flag_column(name: &'static str, flags: u32, samples: [(f32, f32); 5]) -> FlagColumn {
    FlagColumn {
        name,
        flags,
        samples,
    }
}

/// The light reaches every surface at full strength, whatever the geometry.
const UNSHADED_SAMPLES: [(f32, f32); 5] = [(1.0, 0.0); 5];

#[rustfmt::skip]
const FLAG_COLUMNS: [FlagColumn; 13] = [
    flag_column("default", 0,
        [(0.55660, 0.62099), (0.24255, 1.56464), (0.91938, 0.12670), (0.77164, 0.0), (0.0, 0.0)]),
    flag_column("NoRimLighting", LIGHT_FLAG_NO_RIM_LIGHTING,
        [(0.55660, 0.62099), (0.13669, 1.56464), (0.68170, 0.12670), (0.77164, 0.0), (0.0, 0.0)]),
    flag_column("IgnoreRoughness", LIGHT_FLAG_IGNORE_ROUGHNESS,
        [(0.70711, 0.62099), (0.27951, 1.56464), (1.10371, 0.12670), (0.86603, 0.0), (0.0, 0.0)]),
    flag_column("NonSpecular", LIGHT_FLAG_NON_SPECULAR,
        [(0.55660, 0.0), (0.24255, 0.0), (0.91938, 0.0), (0.77164, 0.0), (0.0, 0.0)]),
    flag_column("AttenuationOnly", LIGHT_FLAG_ATTENUATION_ONLY, UNSHADED_SAMPLES),
    flag_column("NoRimLighting|IgnoreRoughness", LIGHT_FLAG_NO_RIM_LIGHTING | LIGHT_FLAG_IGNORE_ROUGHNESS,
        [(0.70711, 0.62099), (0.17365, 1.56464), (0.86603, 0.12670), (0.86603, 0.0), (0.0, 0.0)]),
    flag_column("NoRimLighting|NonSpecular", LIGHT_FLAG_NO_RIM_LIGHTING | LIGHT_FLAG_NON_SPECULAR,
        [(0.55660, 0.0), (0.13669, 0.0), (0.68170, 0.0), (0.77164, 0.0), (0.0, 0.0)]),
    flag_column("IgnoreRoughness|NonSpecular", LIGHT_FLAG_IGNORE_ROUGHNESS | LIGHT_FLAG_NON_SPECULAR,
        [(0.70711, 0.0), (0.27951, 0.0), (1.10371, 0.0), (0.86603, 0.0), (0.0, 0.0)]),
    flag_column(
        "NoRimLighting|IgnoreRoughness|NonSpecular",
        LIGHT_FLAG_NO_RIM_LIGHTING | LIGHT_FLAG_IGNORE_ROUGHNESS | LIGHT_FLAG_NON_SPECULAR,
        [(0.70711, 0.0), (0.17365, 0.0), (0.86603, 0.0), (0.86603, 0.0), (0.0, 0.0)]),
    flag_column("AttenuationOnly|NoRimLighting", LIGHT_FLAG_ATTENUATION_ONLY | LIGHT_FLAG_NO_RIM_LIGHTING, UNSHADED_SAMPLES),
    flag_column("AttenuationOnly|IgnoreRoughness", LIGHT_FLAG_ATTENUATION_ONLY | LIGHT_FLAG_IGNORE_ROUGHNESS, UNSHADED_SAMPLES),
    flag_column("AttenuationOnly|NonSpecular", LIGHT_FLAG_ATTENUATION_ONLY | LIGHT_FLAG_NON_SPECULAR, UNSHADED_SAMPLES),
    flag_column(
        "AttenuationOnly|NoRimLighting|IgnoreRoughness|NonSpecular",
        LIGHT_FLAG_ATTENUATION_ONLY | LIGHT_FLAG_NO_RIM_LIGHTING | LIGHT_FLAG_IGNORE_ROUGHNESS | LIGHT_FLAG_NON_SPECULAR,
        UNSHADED_SAMPLES),
];

/// Diffuse of `IgnoreRoughness` on a directional or shadowed light, where the rim goes too: exactly
/// `max(N.L, 0)`, one per entry of `FLAG_CONFIGURATIONS`.
const LAMBERT_ONLY_DIFFUSE: [f32; 5] = [0.70711, 0.17365, 0.86603, 0.86603, 0.0];

/// Whether a light is shaded as a plain point light or as a directional or shadowed one.
#[derive(Copy, Clone, PartialEq, Eq)]
enum LightKind {
    Plain,
    DirectionalOrShadowed,
}

fn drops_the_rim_with_ignore_roughness(kind: LightKind, flags: u32) -> bool {
    kind == LightKind::DirectionalOrShadowed
        && flags & LIGHT_FLAG_IGNORE_ROUGHNESS != 0
        && flags & LIGHT_FLAG_ATTENUATION_ONLY == 0
}

/// Runs the flag samples for lights of `kind` built by `light_for`. Where `IgnoreRoughness` drops
/// the rim, the diffuse is `LAMBERT_ONLY_DIFFUSE` instead of the plain point light sample.
fn run_flag_samples(kind: LightKind, light_for: impl Fn(u32) -> Light) {
    let surface = Surface::glossy(0.35);
    let mut cases = Vec::new();
    let mut rows = Vec::new();
    for column in &FLAG_COLUMNS {
        let lambert_only = drops_the_rim_with_ignore_roughness(kind, column.flags);
        let samples = FLAG_CONFIGURATIONS.iter().zip(&column.samples);
        for (position, (&configuration_index, &(diffuse, specular))) in samples.enumerate() {
            let config = &CONFIGURATIONS[configuration_index];
            cases.push(Case::local(
                &surface,
                &light_for(column.flags),
                direction(config.light),
                direction(config.viewer),
                Vec3::ONE,
            ));
            let diffuse = if lambert_only {
                LAMBERT_ONLY_DIFFUSE[position]
            } else {
                diffuse
            };
            rows.push((
                format!("{} {}", config.name, column.name),
                diffuse,
                specular,
            ));
        }
    }
    for (output, (row, diffuse, specular)) in run(&cases).iter().zip(&rows) {
        assert_close(output.diffuse.x, *diffuse, &format!("{row} diffuse"));
        assert_close(output.specular.x, *specular, &format!("{row} specular"));
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn plain_point_light_flags_match_expected_samples() {
    run_flag_samples(LightKind::Plain, Light::with_flags);
    run_flag_samples(LightKind::Plain, |flags| {
        Light::with_flags(flags | LIGHT_FLAG_SHADOW_OMNIDIRECTIONAL)
    });
}

#[test]
#[ignore = "needs a GPU adapter"]
fn directional_and_shadowed_lights_drop_the_rim_with_ignore_roughness() {
    let kind = LightKind::DirectionalOrShadowed;
    run_flag_samples(kind, |flags| {
        Light::with_flags(flags | LIGHT_FLAG_NON_SHADOW_SPOTLIGHT)
    });
    run_flag_samples(kind, |flags| {
        Light::with_flags(flags | LIGHT_FLAG_SHADOW_SPOTLIGHT)
    });
    run_flag_samples(kind, |flags| {
        Light::with_flags(flags | LIGHT_FLAG_SHADOW_HEMISPHERE)
    });
    run_flag_samples(kind, |flags| Light {
        shadow_slot: 3,
        ..Light::with_flags(flags | LIGHT_FLAG_SHADOW_OMNIDIRECTIONAL)
    });
}

/// (constant, scalar, exponent) and the attenuation at distance over radius
/// 0.25, 0.5, 0.75, 0.9 and 0.97. The radius window `(distance / radius)^2` is a
/// floor under the lost share, which decides the rows whose own loss stays below it.
#[rustfmt::skip]
const ATTENUATION_ROWS: [([f32; 3], [f32; 5]); 10] = [
    ([0.0, 1.0, 2.0], [0.867635, 0.531049, 0.162238, 0.025898, 0.001984]),
    ([0.0, 1.0, 1.0], [0.531049, 0.217638, 0.047366, 0.006310, 0.000446]),
    ([0.0, 1.0, 0.5], [0.217638, 0.067106, 0.012007, 0.001454, 0.000099]),
    ([0.2, 0.8, 1.0], [0.325037, 0.133209, 0.028991, 0.003862, 0.000273]),
    ([0.0, 2.0, 2.0], [0.745449, 0.217638, 0.0, 0.0, 0.0]),
    ([0.5, 0.5, 3.0], [0.210226, 0.162238, 0.065190, 0.012310, 0.001019]),
    ([0.0, 1.0, 4.0], [0.867635, 0.531049, 0.162238, 0.025898, 0.001984]),
    ([-0.2, 1.0, 2.0], [0.867635, 0.531049, 0.162238, 0.025898, 0.001984]),
    ([0.0, 0.5, 2.0], [0.867635, 0.531049, 0.162238, 0.025898, 0.001984]),
    ([0.0, 1.2, 3.0], [0.867635, 0.531049, 0.162238, 0.010345, 0.0]),
];
const ATTENUATION_DISTANCE_FRACTIONS: [f32; 5] = [0.25, 0.5, 0.75, 0.9, 0.97];
const ATTENUATION_RADIUS: f32 = 100.0;

#[test]
#[ignore = "needs a GPU adapter"]
fn attenuation_matches_expected_samples() {
    let mut cases = Vec::new();
    let mut rows = Vec::new();
    for &([constant, scalar, exponent], expected) in &ATTENUATION_ROWS {
        let light = Light::attenuated(constant, scalar, exponent);
        for (&fraction, &value) in ATTENUATION_DISTANCE_FRACTIONS.iter().zip(&expected) {
            cases.push(Case::attenuation(&light, fraction * ATTENUATION_RADIUS));
            rows.push((
                format!("c={constant} s={scalar} e={exponent} x={fraction}"),
                value,
            ));
        }
    }
    for (output, (row, expected)) in run(&cases).iter().zip(&rows) {
        assert_close(output.scalar(), *expected, row);
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn attenuation_at_the_light_is_the_constant_term_only() {
    let light = Light::attenuated(0.5, 0.5, 3.0);
    let near = run(&[Case::attenuation(&light, 1e-4)])[0].scalar();
    assert_close(near, 0.217638, "constant 0.5 at the centre");
}

/// Cosine-profile samples; entries below 1e-3 are omitted.
#[rustfmt::skip]
const COSINE_PROFILE: [ConeProfile; 12] = [
    (30.0, 0.5, &[(0.0, 1.0), (10.0, 0.94160), (20.0, 0.74153), (25.0, 0.54834), (29.0, 0.25328)]),
    (30.0, 1.0, &[(0.0, 1.0), (10.0, 0.88660), (20.0, 0.54986), (25.0, 0.30067), (29.0, 0.06415)]),
    (30.0, 2.0, &[(0.0, 1.0), (10.0, 0.78607), (20.0, 0.30235), (25.0, 0.09040), (29.0, 0.00412)]),
    (30.0, 4.0, &[(0.0, 1.0), (10.0, 0.61790), (20.0, 0.09141), (25.0, 0.00817)]),
    (45.0, 0.5, &[(0.0, 1.0), (10.0, 0.97372), (20.0, 0.89112), (30.0, 0.73660), (40.0, 0.44858)]),
    (45.0, 1.0, &[(0.0, 1.0), (10.0, 0.94813), (20.0, 0.79410), (30.0, 0.54258), (40.0, 0.20123)]),
    (45.0, 2.0, &[(0.0, 1.0), (10.0, 0.89895), (20.0, 0.63059), (30.0, 0.29440), (40.0, 0.04049)]),
    (45.0, 4.0, &[(0.0, 1.0), (10.0, 0.80811), (20.0, 0.39765), (30.0, 0.08667), (40.0, 0.00164)]),
    (60.0, 0.5, &[(0.0, 1.0), (20.0, 0.93776), (30.0, 0.85560), (40.0, 0.72944), (55.0, 0.38361)]),
    (60.0, 1.0, &[(0.0, 1.0), (20.0, 0.87939), (30.0, 0.73205), (40.0, 0.53209), (55.0, 0.14715)]),
    (60.0, 2.0, &[(0.0, 1.0), (20.0, 0.77332), (30.0, 0.53590), (40.0, 0.28312), (55.0, 0.02165)]),
    (60.0, 4.0, &[(0.0, 1.0), (20.0, 0.59802), (30.0, 0.28719), (40.0, 0.08016)]),
];

/// Radial tangent profile.
#[rustfmt::skip]
const RADIAL_PROFILE: [ConeProfile; 8] = [
    (30.0, 0.5, &[(0.0, 1.0), (10.0, 0.44736), (20.0, 0.20601), (29.0, 0.02016)]),
    (30.0, 1.0, &[(0.0, 1.0), (10.0, 0.69459), (20.0, 0.36959), (29.0, 0.03991)]),
    (30.0, 2.0, &[(0.0, 1.0), (10.0, 0.90673), (20.0, 0.60258), (29.0, 0.07822)]),
    (45.0, 0.5, &[(0.0, 1.0), (10.0, 0.58009), (20.0, 0.39670), (31.0, 0.22485), (44.0, 0.01731)]),
    (45.0, 1.0, &[(0.0, 1.0), (10.0, 0.82367), (20.0, 0.63603), (31.0, 0.39914), (44.0, 0.03431)]),
    (45.0, 2.0, &[(0.0, 1.0), (10.0, 0.96891), (20.0, 0.86753), (31.0, 0.63897), (44.0, 0.06745)]),
    (70.0, 1.0, &[(0.0, 1.0), (20.0, 0.86753), (40.0, 0.69459), (60.0, 0.36959), (69.0, 0.05183)]),
    (80.0, 2.0, &[(0.0, 1.0), (30.0, 0.98964), (60.0, 0.90673), (75.0, 0.56696), (79.0, 0.17713)]),
];

fn run_cone_profiles(profiles: &[ConeProfile], flags: u32, shadow_slot: u32) {
    let mut cases = Vec::new();
    let mut rows = Vec::new();
    for &(half_angle, exponent, samples) in profiles {
        let light = Light::spot(flags, half_angle, exponent, shadow_slot);
        for &(off_axis, expected) in samples {
            cases.push(Case::cone(&light, polar(f64::from(off_axis), 0.0)));
            rows.push((
                format!("half {half_angle} p={exponent} at {off_axis}"),
                expected,
            ));
        }
    }
    for (output, (row, expected)) in run(&cases).iter().zip(&rows) {
        assert_close(output.scalar(), *expected, row);
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn non_shadow_spot_follows_the_cosine_profile() {
    run_cone_profiles(
        &COSINE_PROFILE,
        LIGHT_FLAG_NON_SHADOW_SPOTLIGHT,
        SHADOW_SLOT_NONE,
    );
}

#[test]
#[ignore = "needs a GPU adapter"]
fn shadow_spot_without_a_slot_follows_the_cosine_profile() {
    run_cone_profiles(
        &COSINE_PROFILE,
        LIGHT_FLAG_SHADOW_SPOTLIGHT,
        SHADOW_SLOT_NONE,
    );
}

#[test]
#[ignore = "needs a GPU adapter"]
fn shadow_spot_with_a_slot_follows_the_radial_profile() {
    run_cone_profiles(&RADIAL_PROFILE, LIGHT_FLAG_SHADOW_SPOTLIGHT, 0);
}

#[test]
#[ignore = "needs a GPU adapter"]
fn non_shadow_spot_ignores_its_slot() {
    run_cone_profiles(&COSINE_PROFILE, LIGHT_FLAG_NON_SHADOW_SPOTLIGHT, 5);
}

#[test]
#[ignore = "needs a GPU adapter"]
fn both_spot_bits_select_the_profile_by_slot() {
    let both = LIGHT_FLAG_SHADOW_SPOTLIGHT | LIGHT_FLAG_NON_SHADOW_SPOTLIGHT;
    run_cone_profiles(&RADIAL_PROFILE, both, 2);
    run_cone_profiles(&COSINE_PROFILE, both, SHADOW_SLOT_NONE);
}

#[test]
#[ignore = "needs a GPU adapter"]
fn omnidirectional_lights_have_no_angular_falloff() {
    let behind = polar(170.0, 30.0);
    for flags in [
        0,
        LIGHT_FLAG_SHADOW_OMNIDIRECTIONAL,
        LIGHT_FLAG_NON_SPECULAR,
    ] {
        let light = Light::with_flags(flags);
        let factors = run(&[Case::cone(&light, Vec3::Z), Case::cone(&light, behind)]);
        assert_close(factors[0].scalar(), 1.0, "omni in front");
        assert_close(factors[1].scalar(), 1.0, "omni behind");
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn spot_bits_select_the_profile_and_ignore_the_hemisphere_bit() {
    let hemisphere_spot = LIGHT_FLAG_SHADOW_HEMISPHERE | LIGHT_FLAG_NON_SHADOW_SPOTLIGHT;
    run_cone_profiles(&COSINE_PROFILE, hemisphere_spot, SHADOW_SLOT_NONE);
    run_cone_profiles(
        &RADIAL_PROFILE,
        LIGHT_FLAG_SHADOW_HEMISPHERE | LIGHT_FLAG_SHADOW_SPOTLIGHT,
        0,
    );
    let light = Light::spot(hemisphere_spot, 30.0, 1.0, SHADOW_SLOT_NONE);
    let in_front_of_the_plane_outside_the_cone = Case::cone(&light, polar(60.0, 0.0));
    assert_close(
        run(&[in_front_of_the_plane_outside_the_cone])[0].scalar(),
        0.0,
        "outside the cone",
    );
}

/// Half-angles in degrees around 60, and the radial profile at 40 degrees off the axis with
/// exponent 2: `1 - (tan 40 / tan half-angle)^2`.
const HALF_ANGLE_SWEEP: [(f32, f32); 3] = [(59.9, 0.76341), (60.0, 0.76530), (60.1, 0.76719)];

#[test]
#[ignore = "needs a GPU adapter"]
fn shadow_spot_with_a_slot_changes_smoothly_with_the_half_angle() {
    let off_axis = polar(40.0, 0.0);
    let cases: Vec<Case> = HALF_ANGLE_SWEEP
        .iter()
        .map(|&(half_angle, _)| {
            let light = Light::spot(LIGHT_FLAG_SHADOW_SPOTLIGHT, half_angle, 2.0, 0);
            Case::cone(&light, off_axis)
        })
        .collect();
    for (output, (half_angle, expected)) in run(&cases).iter().zip(HALF_ANGLE_SWEEP) {
        assert_close(
            output.scalar(),
            expected,
            &format!("half {half_angle} p=2 at 40"),
        );
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn hemisphere_light_lights_the_plane_through_itself() {
    let light = Light::with_flags(LIGHT_FLAG_SHADOW_HEMISPHERE);
    let on_plane = [Vec3::X, Vec3::Y, Vec3::NEG_X, Vec3::NEG_Y];
    let factors = run(&on_plane.map(|direction| Case::cone(&light, direction)));
    let values: Vec<f32> = factors.iter().map(Output::scalar).collect();
    assert_eq!(values, vec![1.0; 4]);
}

#[test]
#[ignore = "needs a GPU adapter"]
fn hemisphere_light_lights_only_the_front_side() {
    let light = Light::with_flags(LIGHT_FLAG_SHADOW_HEMISPHERE);
    let factors = run(&[
        Case::cone(&light, Vec3::Z),
        Case::cone(&light, polar(80.0, 10.0)),
        Case::cone(&light, polar(100.0, 10.0)),
        Case::cone(&light, Vec3::NEG_Z),
    ]);
    let values: Vec<f32> = factors.iter().map(Output::scalar).collect();
    assert_eq!(values, vec![1.0, 1.0, 0.0, 0.0]);
}

/// Gamma-space face colours of the six-face cube that reproduces the ambient test gradient, in
/// the cube order +X, -X, +Y, -Y, +Z, -Z.
const GAMMA_FACES: [[f32; 3]; 6] = [
    [0.600, 0.400, 0.250],
    [0.400, 0.400, 0.350],
    [0.550, 0.300, 0.300],
    [0.450, 0.500, 0.300],
    [0.700, 0.500, 0.600],
    [0.300, 0.300, 0.000],
];

fn gradient_cube() -> [Vec3; 6] {
    GAMMA_FACES.map(|face| Vec3::from(face.map(|channel| channel.powf(2.2))))
}

fn ambient_environment(max_specular_intensity: Option<f32>) -> Environment {
    Environment {
        max_specular_intensity,
        ambient_cube: gradient_cube(),
    }
}

const AMBIENT_DIFFUSE_ALONG_Z: Vec3 = Vec3::new(0.45626, 0.21764, 0.32504);

struct AmbientRow {
    viewer: Angles,
    smoothness: f32,
    intensity: f32,
    specular: [f32; 3],
}

const fn ambient_row(
    viewer: Angles,
    smoothness: f32,
    intensity: f32,
    specular: [f32; 3],
) -> AmbientRow {
    AmbientRow {
        viewer,
        smoothness,
        intensity,
        specular,
    }
}

/// Axis-aligned ambient samples: normal +Z, ambient diffuse `AMBIENT_DIFFUSE_ALONG_Z`. The last
/// six rows look at the surface from behind or use smoothness outside [0, 1]; their specular is
/// computed from the same gradient and, for out-of-range smoothness, repeats the in-range rows.
#[rustfmt::skip]
const AMBIENT_ROWS: [AmbientRow; 18] = [
    ambient_row((0.0, 0.0), 0.0, 1.0, [0.0, 0.0, 0.0]),
    ambient_row((0.0, 0.0), 0.5, 1.0, [0.0, 0.0, 0.0]),
    ambient_row((0.0, 0.0), 1.0, 2.0, [0.0, 0.0, 0.0]),
    ambient_row((45.0, 0.0), 0.0, 1.0, [0.00183, 0.00120, 0.00167]),
    ambient_row((45.0, 0.0), 0.5, 1.0, [0.00338, 0.00221, 0.00308]),
    ambient_row((45.0, 0.0), 1.0, 2.0, [0.01249, 0.00817, 0.01140]),
    ambient_row((70.0, 0.0), 0.0, 1.0, [0.01381, 0.01136, 0.01227]),
    ambient_row((70.0, 0.0), 0.5, 1.0, [0.01702, 0.01401, 0.01512]),
    ambient_row((70.0, 0.0), 1.0, 2.0, [0.04197, 0.03454, 0.03729]),
    ambient_row((80.0, 90.0), 0.0, 1.0, [0.02878, 0.03288, 0.01419]),
    ambient_row((80.0, 90.0), 0.5, 1.0, [0.03166, 0.03617, 0.01561]),
    ambient_row((80.0, 90.0), 1.0, 2.0, [0.06965, 0.07959, 0.03435]),
    ambient_row((100.0, 0.0), 0.0, 1.0, [0.02752, 0.03020, 0.01732]),
    ambient_row((120.0, 90.0), 0.5, 1.0, [0.02588, 0.04038, 0.00385]),
    ambient_row((150.0, 200.0), 1.0, 2.0, [0.06030, 0.03442, 0.00006]),
    ambient_row((45.0, 0.0), -0.5, 1.0, [0.00183, 0.00120, 0.00167]),
    ambient_row((70.0, 0.0), 3.0, 2.0, [0.04197, 0.03454, 0.03729]),
    ambient_row((80.0, 90.0), 100.0, 2.0, [0.06965, 0.07959, 0.03435]),
];

/// Runs the axis-aligned ambient rows under the given ceiling; `specular_scale` maps a row's
/// intensity to the share of the expected specular that the ceiling leaves.
fn run_ambient_rows(max_specular_intensity: Option<f32>, specular_scale: impl Fn(f32) -> f32) {
    let cases: Vec<Case> = AMBIENT_ROWS
        .iter()
        .map(|row| {
            let surface = Surface {
                smoothness: row.smoothness,
                spec_intensity: row.intensity,
                ..Surface::default()
            };
            Case::ambient(&surface, direction(row.viewer))
        })
        .collect();
    let outputs = run_in(&ambient_environment(max_specular_intensity), &cases);
    for (index, (output, row)) in outputs.iter().zip(&AMBIENT_ROWS).enumerate() {
        let name = format!(
            "ambient row {} viewer {:?} g={} k={}",
            index + 1,
            row.viewer,
            row.smoothness,
            row.intensity
        );
        assert_close3(output.diffuse, AMBIENT_DIFFUSE_ALONG_Z, &name);
        assert_close3(
            output.specular,
            Vec3::from(row.specular) * specular_scale(row.intensity),
            &name,
        );
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn ambient_matches_expected_samples_without_a_specular_ceiling() {
    run_ambient_rows(Some(8.0), |_| 1.0);
}

#[test]
#[ignore = "needs a GPU adapter"]
fn ambient_specular_is_capped_at_the_default_intensity_ceiling() {
    run_ambient_rows(None, |intensity| intensity.min(1.0) / intensity);
}

struct VaryingNormalRow {
    normal: [f32; 3],
    viewer: [f32; 3],
    diffuse: [f32; 3],
    specular: [f32; 3],
}

const fn varying_row(
    normal: [f32; 3],
    viewer: [f32; 3],
    diffuse: [f32; 3],
    specular: [f32; 3],
) -> VaryingNormalRow {
    VaryingNormalRow {
        normal,
        viewer,
        diffuse,
        specular,
    }
}

/// Normal, viewer, ambient diffuse and ambient specular (smoothness 0.5, intensity 1).
#[rustfmt::skip]
const AMBIENT_VARYING_NORMAL: [VaryingNormalRow; 6] = [
    varying_row([1.0, 0.0, 0.0], [0.947634, -0.199502, -0.249377], [0.32504, 0.13321, 0.04737], [0.00006, 0.00002, 0.00001]),
    varying_row([0.0, 1.0, 0.0], [0.755263, 0.503509, -0.419591], [0.26841, 0.07074, 0.07074], [0.01090, 0.00552, 0.00800]),
    varying_row([0.0, 0.0, -1.0], [0.501550, -0.222911, -0.835917], [0.07074, 0.07074, 0.0], [0.00018, 0.00018, 0.00001]),
    varying_row([0.6, 0.0, -0.8], [0.740797, -0.197546, -0.642024], [0.13321, 0.08153, 0.00045], [0.00001, 0.00001, 0.0]),
    varying_row([-0.500002, 0.500002, -0.707103], [0.313582, 0.078398, -0.946319], [0.08934, 0.06044, 0.00824], [0.00641, 0.00376, 0.00432]),
    varying_row([0.303046, -0.808122, -0.505076], [0.607858, -0.610423, -0.507831], [0.12520, 0.15643, 0.01188], [0.00003, 0.00005, 0.00001]),
];

#[test]
#[ignore = "needs a GPU adapter"]
fn ambient_matches_expected_samples_for_varying_normals() {
    let cases: Vec<Case> = AMBIENT_VARYING_NORMAL
        .iter()
        .map(|row| {
            let surface = Surface {
                normal: Vec3::from(row.normal).normalize(),
                ..Surface::default()
            };
            Case::ambient(&surface, Vec3::from(row.viewer).normalize())
        })
        .collect();
    let outputs = run_in(&ambient_environment(None), &cases);
    for (index, (output, row)) in outputs.iter().zip(&AMBIENT_VARYING_NORMAL).enumerate() {
        let name = format!("varying normal row {}", index + 1);
        assert_close3(output.diffuse, Vec3::from(row.diffuse), &name);
        assert_close3(output.specular, Vec3::from(row.specular), &name);
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn ambient_toward_a_uniform_cube_is_that_colour_in_every_direction() {
    let color = Vec3::new(0.2, 0.05, 0.6);
    let environment = Environment {
        ambient_cube: [color; 6],
        ..Environment::default()
    };
    let directions = [
        Vec3::X,
        Vec3::NEG_Y,
        Vec3::new(1.0, 2.0, -3.0).normalize(),
        Vec3::new(-0.3, 0.4, 0.5).normalize(),
    ];
    let cases: Vec<Case> = directions
        .iter()
        .map(|&d| Case::ambient_toward(d))
        .collect();
    for (output, direction) in run_in(&environment, &cases).iter().zip(directions) {
        assert_close3(
            output.diffuse,
            color,
            &format!("uniform cube towards {direction}"),
        );
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn ambient_toward_is_zero_where_the_gradient_is_negative() {
    let environment = Environment {
        ambient_cube: [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::ZERO,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::ZERO,
        ],
        ..Environment::default()
    };
    // Every channel's gradient is 1/6 in the directions it has no face of its own, and
    // 1/6 - 1/2 = -1/3 away from its own face.
    let untouched = (1.0f32 / 6.0).powf(2.2);
    let rows = [
        (Vec3::NEG_X, Vec3::new(0.0, untouched, untouched)),
        (Vec3::NEG_Y, Vec3::new(untouched, 0.0, untouched)),
        (Vec3::NEG_Z, Vec3::new(untouched, untouched, 0.0)),
        (Vec3::splat(-1.0).normalize(), Vec3::ZERO),
    ];
    let cases: Vec<Case> = rows
        .iter()
        .map(|&(direction, _)| Case::ambient_toward(direction))
        .collect();
    for (output, (direction, expected)) in run_in(&environment, &cases).iter().zip(rows) {
        let row = format!("ambient towards {direction}");
        for channel in 0..3 {
            if expected[channel] == 0.0 {
                assert_eq!(output.diffuse[channel], 0.0, "{row} channel {channel}");
            } else {
                assert_close(output.diffuse[channel], expected[channel], &row);
            }
        }
    }
}

/// A corner-case sample: surface, geometry and the expected diffuse and specular.
struct CornerCase {
    name: &'static str,
    light: Angles,
    viewer: Angles,
    smoothness: f32,
    intensity: f32,
    diffuse: f32,
    specular: f32,
}

const fn corner(
    name: &'static str,
    light: Angles,
    viewer: Angles,
    smoothness: f32,
    diffuse: f32,
    specular: f32,
) -> CornerCase {
    CornerCase {
        name,
        light,
        viewer,
        smoothness,
        intensity: 1.0,
        diffuse,
        specular,
    }
}

const HEAD_ON: Angles = (0.0, 0.0);
const MIRROR_LIGHT: Angles = (40.0, 0.0);
const MIRROR_VIEWER: Angles = (40.0, 180.0);

fn corner_cases() -> Vec<CornerCase> {
    let at_intensity = |intensity: f32, specular: f32| CornerCase {
        intensity,
        ..corner(
            "mirror 40 g=0.5",
            MIRROR_LIGHT,
            MIRROR_VIEWER,
            0.5,
            0.649269,
            specular,
        )
    };
    vec![
        corner("n.l 1e-4", (89.99427, 0.0), HEAD_ON, 0.5, 0.000085, 0.0),
        corner("n.v 0 az 0", (45.0, 0.0), (90.0, 0.0), 0.6, 0.773616, 0.0),
        corner("n.v 0 az 90", (45.0, 0.0), (90.0, 90.0), 0.6, 0.629616, 0.0),
        corner(
            "n.v 0 az 180",
            (45.0, 0.0),
            (90.0, 180.0),
            0.6,
            0.829616,
            0.000598,
        ),
        corner(
            "n.v -0.5 az 0",
            (45.0, 0.0),
            (120.0, 0.0),
            0.9,
            0.717886,
            0.0,
        ),
        corner(
            "n.v -0.5 az 90",
            (45.0, 0.0),
            (120.0, 90.0),
            0.9,
            0.726011,
            0.0,
        ),
        corner(
            "n.v -0.5 az 180",
            (45.0, 0.0),
            (120.0, 180.0),
            0.9,
            0.769312,
            0.0,
        ),
        corner(
            "l = -v g=0.6",
            (30.0, 0.0),
            (150.0, 180.0),
            0.6,
            1.117529,
            0.0,
        ),
        corner(
            "l = -v g=0",
            (30.0, 0.0),
            (150.0, 180.0),
            0.0,
            1.456247,
            0.0,
        ),
        corner("l = v", MIRROR_LIGHT, MIRROR_LIGHT, 0.6, 0.731260, 0.0),
        corner("head-on g=0", HEAD_ON, HEAD_ON, 0.0, 0.681529, 0.1),
        corner("head-on g=0.5", HEAD_ON, HEAD_ON, 0.5, 0.847561, 1.65),
        corner("head-on g=1", HEAD_ON, HEAD_ON, 1.0, 1.0, 47.123891),
        corner("head-on g<0", HEAD_ON, HEAD_ON, -0.5, 0.681529, 0.1),
        corner("head-on g>1", HEAD_ON, HEAD_ON, 3.0, 1.0, 47.123891),
        at_intensity(0.0, 0.0),
        at_intensity(0.5, 0.827313),
    ]
}

#[test]
#[ignore = "needs a GPU adapter"]
fn sun_matches_expected_corner_case_samples() {
    let rows = corner_cases();
    let cases: Vec<Case> = rows
        .iter()
        .map(|row| {
            let surface = Surface {
                smoothness: row.smoothness,
                spec_intensity: row.intensity,
                ..Surface::default()
            };
            Case::direct(
                &surface,
                direction(row.light),
                direction(row.viewer),
                Vec3::ONE,
            )
        })
        .collect();
    for (output, row) in run(&cases).iter().zip(&rows) {
        assert_close(
            output.diffuse.x,
            row.diffuse,
            &format!("{} diffuse", row.name),
        );
        assert_close(
            output.specular.x,
            row.specular,
            &format!("{} specular", row.name),
        );
    }
}

/// The mirror configuration at smoothness 0.5 for intensities beyond 1, and the specular each
/// ceiling gives: (intensity, ceiling, specular).
const INTENSITY_CEILING_ROWS: [(f32, f32, f32); 6] = [
    (2.0, 1.0, 1.654626),
    (100.0, 1.0, 1.654626),
    (2.0, 8.0, 3.309252),
    (100.0, 8.0, 13.23701),
    (100.0, 100.0, 13.23701),
    (0.5, 8.0, 0.827313),
];

fn mirror_output(environment: &Environment, intensity: f32) -> Output {
    let surface = Surface {
        spec_intensity: intensity,
        ..Surface::default()
    };
    let case = Case::direct(
        &surface,
        direction(MIRROR_LIGHT),
        direction(MIRROR_VIEWER),
        Vec3::ONE,
    );
    run_in(environment, &[case])[0]
}

#[test]
#[ignore = "needs a GPU adapter"]
fn specular_intensity_is_limited_by_the_ceiling_and_the_hard_limit() {
    for &(intensity, ceiling, specular) in &INTENSITY_CEILING_ROWS {
        let output = mirror_output(&Environment::with_specular_ceiling(ceiling), intensity);
        assert_close(
            output.specular.x,
            specular,
            &format!("intensity {intensity} ceiling {ceiling}"),
        );
        assert_close(output.diffuse.x, 0.649269, "diffuse ignores the ceiling");
    }
}

/// Specular of the mirror configuration at smoothness 0.5 under the ceiling a pipeline without an
/// override uses: (intensity, specular).
const DEFAULT_CEILING_ROWS: [(f32, f32); 3] = [(0.5, 0.827313), (2.0, 1.654626), (100.0, 1.654626)];

#[test]
#[ignore = "needs a GPU adapter"]
fn pipeline_without_an_override_limits_the_specular_intensity_to_one() {
    for &(intensity, specular) in &DEFAULT_CEILING_ROWS {
        let output = mirror_output(&Environment::default(), intensity);
        assert_close(
            output.specular.x,
            specular,
            &format!("intensity {intensity} without an override"),
        );
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn end_to_end_example_reproduces_the_final_colour() {
    let albedo = Vec3::new(0.6, 0.5, 0.4);
    let surface = Surface {
        albedo,
        smoothness: 0.6,
        ..Surface::default()
    };
    let viewer = direction((25.0, 200.0));
    let sun_direction = direction((55.0, 100.0));
    let sun_color = Vec3::new(1.0, 0.9, 0.8);
    let to_light = direction((27.0, 20.0));
    let light = Light {
        position: to_light * 6.0,
        radius: 20.0,
        ..Light::default()
    };
    let light_color = Vec3::new(1.0, 0.7, 0.4);

    let environment = ambient_environment(None);
    let attenuation = run_in(&environment, &[Case::attenuation(&light, 6.0)])[0].scalar();
    assert_close(attenuation, 0.81263, "attenuation at x = 0.3");

    let parts = run_in(
        &environment,
        &[
            Case::direct(&surface, sun_direction, viewer, sun_color),
            Case::ambient(&surface, viewer),
            Case::local(
                &surface,
                &light,
                to_light,
                viewer,
                light_color * attenuation,
            ),
        ],
    );
    assert_close3(
        parts[0].diffuse,
        albedo * Vec3::new(0.51072, 0.45965, 0.40857),
        "sun diffuse",
    );
    assert_close3(parts[0].specular, Vec3::ZERO, "sun specular");
    assert_close3(
        parts[1].diffuse,
        albedo * AMBIENT_DIFFUSE_ALONG_Z,
        "ambient diffuse",
    );
    assert_close3(
        parts[1].specular,
        Vec3::new(0.00042, 0.00017, 0.00023),
        "ambient specular",
    );
    assert_close3(
        parts[2].diffuse,
        albedo * Vec3::new(0.64471, 0.45129, 0.25788),
        "local diffuse",
    );
    assert_close3(
        parts[2].specular,
        Vec3::new(2.54642, 1.78250, 1.01857),
        "local specular",
    );

    let final_color: Vec3 = parts.iter().map(|part| part.diffuse).sum::<Vec3>()
        + parts.iter().map(|part| part.specular).sum::<Vec3>();
    assert_close3(
        final_color,
        Vec3::new(3.51386, 2.34695, 1.41540),
        "final colour",
    );
}
