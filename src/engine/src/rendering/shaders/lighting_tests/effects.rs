//! Effect shading and environment reflection functions of the object shader: the falloff
//! ramp, the premultiplied output of every blend class and the reflection term.

#![allow(
    clippy::float_cmp,
    reason = "exact zeros and ones are part of the contract"
)]

use glam::{Vec3, Vec4};

use super::harness::{Case, EffectLayer, EnvTerm, Environment, Output, Shaded};
use super::{assert_close, assert_close3, run, run_in};
use crate::rendering::gpu_driven::{
    EFFECT_FLAG_ADDITIVE, EFFECT_FLAG_EFFECT, EFFECT_FLAG_ENV, EFFECT_FLAG_FALLOFF,
    EFFECT_FLAG_FALLOFF_COLOR, EFFECT_FLAG_PALETTE_ALPHA, EFFECT_FLAG_PALETTE_COLOR,
};

const OBJECT_WGSL: &str = include_str!("../object.wgsl");

const BODY: Vec3 = Vec3::new(0.2, 0.4, 0.6);
const FOGGED_BODY: Vec3 = Vec3::new(0.3, 0.3, 0.3);
const REFLECTION: Vec3 = Vec3::new(0.1, 0.2, 0.3);

fn shaded(transmittance: f32) -> Shaded {
    Shaded {
        body: BODY,
        fogged_body: FOGGED_BODY,
        reflection: REFLECTION,
        transmittance,
    }
}

fn falloff_at(falloff: Vec4, facing: f32) -> f32 {
    run(&[Case::falloff(falloff, facing)])[0].scalar()
}

#[test]
#[ignore = "needs a GPU adapter"]
fn falloff_ramps_between_the_facing_thresholds() {
    let falloff = Vec4::new(1.0, 0.5, 0.0, 1.0);
    for (facing, expected) in [(1.0, 0.0), (0.75, 0.5), (0.5, 1.0), (0.0, 1.0)] {
        assert_close(
            falloff_at(falloff, facing),
            expected,
            &format!("facing {facing}"),
        );
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn falloff_ramps_upwards_when_the_stop_is_above_the_start() {
    let falloff = Vec4::new(0.0, 1.0, 0.2, 0.8);
    assert_close(falloff_at(falloff, 0.0), 0.2, "start");
    assert_close(falloff_at(falloff, 0.5), 0.5, "middle");
    assert_close(falloff_at(falloff, 1.0), 0.8, "stop");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn falloff_with_equal_thresholds_is_a_step_at_the_threshold() {
    let falloff = Vec4::new(0.5, 0.5, 0.1, 0.9);
    assert_close(falloff_at(falloff, 0.49), 0.9, "below: stop opacity");
    assert_close(falloff_at(falloff, 0.5), 0.1, "at: start opacity");
    assert_close(falloff_at(falloff, 0.9), 0.1, "above: start opacity");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn the_degenerate_step_agrees_with_the_descending_ramp_at_its_ends() {
    let ramp = Vec4::new(0.5, 0.4, 0.1, 0.9);
    let step = Vec4::new(0.5, 0.5, 0.1, 0.9);
    for facing in [0.0, 1.0] {
        assert_close(
            falloff_at(step, facing),
            falloff_at(ramp, facing),
            &format!("facing {facing}"),
        );
    }
}

fn layer_of(layer: &EffectLayer) -> (Vec3, f32, Vec3) {
    let output = run(&[Case::effect_layer(layer)])[0];
    (
        output.diffuse,
        output.specular.x,
        Vec3::new(output.specular.y, output.specular.z, 0.0),
    )
}

fn falloff_layer(flags: u32) -> EffectLayer {
    EffectLayer {
        flags,
        falloff: Vec4::new(1.0, 0.0, 1.0, 0.25),
        tint: Vec3::new(1.0, 0.5, 0.0),
        texel: Vec4::new(0.8, 0.8, 0.8, 0.8),
        alpha_mult: 2.0,
        facing: 0.0,
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn falloff_scales_the_alpha_and_only_scales_the_colour_when_flagged() {
    let alpha_only = layer_of(&falloff_layer(EFFECT_FLAG_FALLOFF));
    let with_color = layer_of(&falloff_layer(
        EFFECT_FLAG_FALLOFF | EFFECT_FLAG_FALLOFF_COLOR,
    ));
    let albedo = Vec3::new(0.8, 0.4, 0.0);
    assert_close3(alpha_only.0, albedo, "colour untouched");
    assert_close(alpha_only.1, 0.8 * 2.0 * 0.25, "alpha scaled");
    assert_close3(with_color.0, albedo * 0.25, "colour scaled by the opacity");
    assert_close(with_color.1, 0.8 * 2.0 * 0.25, "alpha scaled");
    assert_close3(
        with_color.2,
        albedo.with_z(0.0),
        "albedo is the unfaded colour",
    );
}

#[test]
#[ignore = "needs a GPU adapter"]
fn without_the_falloff_flag_the_layer_is_only_tinted() {
    let (color, alpha, _) = layer_of(&falloff_layer(EFFECT_FLAG_FALLOFF_COLOR));
    assert_close3(color, Vec3::new(0.8, 0.4, 0.0), "colour");
    assert_close(alpha, 1.6, "alpha is texel alpha times the multiplier");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn facing_ignores_the_side_of_the_surface_the_viewer_is_on() {
    let facing = |normal, view| run(&[Case::effect_facing(normal, view)])[0].scalar();
    assert_close(facing(Vec3::Z, Vec3::Z), 1.0, "front");
    assert_close(facing(Vec3::Z, Vec3::NEG_Z), 1.0, "back");
    assert_close(facing(Vec3::Z, Vec3::X), 0.0, "edge on");
    assert_close(
        facing(Vec3::Z, Vec3::new(0.6, 0.0, -0.8)),
        0.8,
        "oblique back",
    );
}

#[test]
#[ignore = "needs a GPU adapter"]
fn lit_reflectance_converts_the_srgb_specular_colour_to_linear_exactly_once() {
    let f0 = |rgb, intensity| run(&[Case::lit_env_f0(rgb, intensity)])[0].scalar();
    let mid_grey_linear = 0.215_860_5;
    assert_close(f0([255, 255, 255], 1.0), 1.0, "white");
    assert_close(
        f0([128, 128, 128], 1.0),
        mid_grey_linear,
        "mid grey is linearised once",
    );
    assert_close(
        f0([128, 128, 128], 0.5),
        mid_grey_linear * 0.5,
        "scaled by intensity",
    );
    assert_close(f0([0, 0, 0], 1.0), 0.04, "dielectric floor");
    let weighted = 0.2126 * 1.0 + 0.7152 * 0.0 + 0.0722 * 0.0;
    assert_close(f0([255, 0, 0], 1.0), weighted, "luminance weighted");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn the_cube_direction_maps_engine_axes_to_the_authoring_axes() {
    let direction = |engine| run(&[Case::env_direction(engine)])[0].diffuse;
    assert_close3(direction(Vec3::X), Vec3::X, "engine +X");
    assert_close3(direction(Vec3::Y), Vec3::Z, "engine +Y is cube +Z");
    assert_close3(direction(Vec3::NEG_Z), Vec3::Y, "engine -Z is cube +Y");
    assert_close3(direction(Vec3::Z), Vec3::NEG_Y, "engine +Z is cube -Y");
}

const BASE: Vec4 = Vec4::new(0.2, 0.4, 0.6, 0.3);
const BY_LUMINANCE: Vec4 = Vec4::new(0.9, 0.8, 0.7, 0.11);
const BY_ALPHA: Vec4 = Vec4::new(0.1, 0.2, 0.3, 0.77);

fn palette_resolved(flags: u32) -> (Vec3, f32) {
    run(&[Case::apply_palette(flags, BASE, BY_LUMINANCE, BY_ALPHA)])[0].color_alpha()
}

#[test]
#[ignore = "needs a GPU adapter"]
fn the_palette_recolours_by_luminance_and_drives_the_alpha_by_the_base_alpha() {
    let coordinates = run(&[Case::palette_coordinates(BASE, 0.25)])[0];
    assert_close(
        coordinates.diffuse.x,
        0.2126 * 0.2 + 0.7152 * 0.4 + 0.0722 * 0.6,
        "colour coordinate is the base luminance",
    );
    assert_close(
        coordinates.diffuse.y,
        0.3,
        "alpha coordinate is the base alpha",
    );
    assert_close(coordinates.diffuse.z, 0.25, "u is the coordinate");
    assert_close(
        coordinates.specular.x,
        0.5,
        "single-row palette is read mid-row",
    );
}

#[test]
#[ignore = "needs a GPU adapter"]
fn each_palette_flag_replaces_only_its_own_channels() {
    let (color, alpha) = palette_resolved(0);
    assert_close3(color, BASE.truncate(), "no flags: colour");
    assert_close(alpha, 0.3, "no flags: alpha");
    let (color, alpha) = palette_resolved(EFFECT_FLAG_PALETTE_COLOR);
    assert_close3(color, BY_LUMINANCE.truncate(), "colour flag: colour");
    assert_close(alpha, 0.3, "colour flag keeps the base alpha");
    let (color, alpha) = palette_resolved(EFFECT_FLAG_PALETTE_ALPHA);
    assert_close3(color, BASE.truncate(), "alpha flag keeps the base colour");
    assert_close(alpha, 0.77, "alpha flag: alpha");
    let (color, alpha) = palette_resolved(EFFECT_FLAG_PALETTE_COLOR | EFFECT_FLAG_PALETTE_ALPHA);
    assert_close3(color, BY_LUMINANCE.truncate(), "both: colour");
    assert_close(alpha, 0.77, "both: alpha");
}

fn entry_point_body(name: &str) -> &'static str {
    let start = OBJECT_WGSL
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("{name} is declared"));
    let body = &OBJECT_WGSL[start..];
    &body[..body.find("\n}").expect("entry point closes")]
}

#[test]
fn each_entry_point_composes_with_its_own_blend_class() {
    let composers = ["compose_opaque", "compose_blend", "compose_multiply"];
    for (entry, own) in [
        ("opaque_rgb", "compose_opaque"),
        ("fs_blend", "compose_blend"),
        ("fs_multiply", "compose_multiply"),
    ] {
        let body = entry_point_body(entry);
        for composer in composers {
            assert_eq!(
                body.contains(composer),
                composer == own,
                "{entry} uses {composer}"
            );
        }
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn alpha_blend_is_premultiplied_and_the_reflection_ignores_the_alpha() {
    let outputs = run(&[
        Case::compose_blend(EFFECT_FLAG_EFFECT, &shaded(1.0), 0.5),
        Case::compose_blend(EFFECT_FLAG_EFFECT, &shaded(1.0), 0.0),
        Case::compose_blend(EFFECT_FLAG_EFFECT, &shaded(1.0), 1.5),
        Case::compose_blend(EFFECT_FLAG_EFFECT, &shaded(0.5), 0.5),
    ]);
    let rows = [
        (FOGGED_BODY * 0.5 + REFLECTION, 0.5, "half"),
        (REFLECTION, 0.0, "transparent pane still mirrors"),
        (FOGGED_BODY + REFLECTION, 1.0, "alpha clamped to coverage 1"),
        (
            FOGGED_BODY * 0.5 + REFLECTION * 0.5,
            0.5,
            "reflection fades with the fog",
        ),
    ];
    for (output, (color, alpha, row)) in outputs.iter().zip(rows) {
        let (actual_color, actual_alpha) = output.color_alpha();
        assert_close3(actual_color, color, row);
        assert_close(actual_alpha, alpha, row);
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn additive_scales_by_the_unclamped_alpha_and_writes_no_coverage() {
    let outputs = run(&[
        Case::compose_blend(EFFECT_FLAG_ADDITIVE, &shaded(1.0), 2.0),
        Case::compose_blend(EFFECT_FLAG_ADDITIVE, &shaded(0.0), 2.0),
    ]);
    let (bright, bright_alpha) = outputs[0].color_alpha();
    assert_close3(bright, (BODY + REFLECTION) * 2.0, "brightens above one");
    assert_close(bright_alpha, 0.0, "no coverage");
    let (gone, _) = outputs[1].color_alpha();
    assert_close3(gone, Vec3::ZERO, "fully fogged additive vanishes");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn multiply_tends_to_white_without_alpha_or_with_distance() {
    let outputs = run(&[
        Case::compose_multiply(&shaded(1.0), 0.5),
        Case::compose_multiply(&shaded(1.0), 0.0),
        Case::compose_multiply(&shaded(1.0), 2.0),
        Case::compose_multiply(&shaded(0.0), 1.0),
    ]);
    let rows = [
        (Vec3::splat(0.5) + BODY * 0.5, "half"),
        (Vec3::ONE, "no alpha"),
        (BODY, "alpha saturates"),
        (Vec3::ONE, "fully fogged"),
    ];
    for (output, (color, row)) in outputs.iter().zip(rows) {
        assert_close3(output.color_alpha().0, color, row);
    }
}

/// The reflection colour, its weight and the mip level.
fn term_output(term: &EnvTerm) -> (Vec3, f32, f32) {
    let output = run(&[Case::env_term(term)])[0];
    (output.diffuse, output.specular.x, output.specular.y)
}

#[test]
#[ignore = "needs a GPU adapter"]
fn schlick_weight_runs_from_f0_at_normal_incidence_to_one_at_grazing() {
    let weight = |n_dot_v| {
        term_output(&EnvTerm {
            n_dot_v,
            ..EnvTerm::default()
        })
        .1
    };
    assert_close(weight(1.0), 0.04, "normal incidence");
    assert_close(weight(0.0), 1.0, "grazing");
    assert_close(weight(0.5), 0.04 + 0.96 * 0.5_f32.powi(5), "half");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn the_reflection_is_radiance_times_weight_and_exposure() {
    let term = EnvTerm {
        radiance: Vec3::new(1.0, 0.5, 0.25),
        n_dot_v: 0.0,
        mask: 0.5,
        mask_scale: 0.4,
        exposure: 0.5,
        ..EnvTerm::default()
    };
    let (color, weight, _) = term_output(&term);
    assert_close(weight, 0.2, "weight");
    assert_close3(color, Vec3::new(1.0, 0.5, 0.25) * 0.2 * 0.5, "colour");
    let (dark, _, _) = term_output(&EnvTerm {
        exposure: 0.0,
        ..term
    });
    assert_close3(dark, Vec3::ZERO, "no exposure");
    let (black, _, _) = term_output(&EnvTerm {
        radiance: Vec3::ZERO,
        ..term
    });
    assert_close3(black, Vec3::ZERO, "black cube");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn the_mip_level_follows_the_roughness_and_stays_inside_the_chain() {
    let lod = |roughness_lod, level_count| {
        term_output(&EnvTerm {
            roughness_lod,
            level_count,
            ..EnvTerm::default()
        })
        .2
    };
    assert_close(lod(0.5, 9), 4.0, "half of eight steps");
    assert_close(lod(2.0, 9), 8.0, "clamped to the last level");
    assert_close(lod(-1.0, 9), 0.0, "clamped to the first level");
    assert_close(lod(0.5, 1), 0.0, "single level");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn exposure_follows_the_ambient_luminance_up_to_one() {
    let exposure = |ambient: f32| {
        let environment = Environment {
            ambient_cube: [Vec3::splat(ambient); 6],
            ..Environment::default()
        };
        run_in(&environment, &[Case::env_exposure(Vec3::Y)])[0].scalar()
    };
    assert_close(exposure(0.0), 0.0, "dark");
    assert_close(exposure(0.25), 0.5, "dim");
    assert_close(exposure(4.0), 1.0, "bright");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn the_lookup_is_skipped_without_the_flag_or_with_the_black_cube_slot() {
    let outputs = run(&[
        Case::env_enabled(EFFECT_FLAG_ENV, 1),
        Case::env_enabled(EFFECT_FLAG_ENV, 0),
        Case::env_enabled(EFFECT_FLAG_EFFECT, 1),
    ]);
    let enabled: Vec<f32> = outputs.iter().map(Output::scalar).collect();
    assert_eq!(enabled, [1.0, 0.0, 0.0]);
}
