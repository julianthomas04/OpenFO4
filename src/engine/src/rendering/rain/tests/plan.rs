//! The numbers the rain hands the shader each frame, without a GPU.

use glam::{UVec2, Vec2, Vec3};

use super::{EYE, GENERATION, downpour};
use crate::bethesda_space::ENGINE_UNITS_PER_METER;
use crate::rendering::rain::motion::{DropDrift, drop_count, splash_count};
use crate::rendering::rain::occlusion::{EXTENT, HEIGHT_RANGE, OcclusionWindow};
use crate::rendering::rain::tuning::{
    LEAST_VARIATION, MAX_DROPS, MIN_BOX_EDGE, SPLASH_OPACITY, SPLASH_SECONDS, STREAK_LENGTH_SCALE,
    STREAK_OPACITY, STREAK_WIDTH_SCALE,
};
use crate::rendering::rain::uniform::{Draws, Fall, RainUniform};
use crate::rendering::rain::{RainInput, RainView, StreakMaterial};
use crate::rendering::shaders;

fn view() -> RainView {
    RainView {
        eye: EYE,
        forward: Vec3::NEG_Z,
        time: 10.0,
        world_generation: GENERATION,
        underwater: false,
    }
}

fn window() -> OcclusionWindow {
    OcclusionWindow::around(EYE)
}

fn uniform_of(input: &RainInput, view: &RainView) -> RainUniform {
    RainUniform::new(
        input,
        view,
        window(),
        &Fall::of(input),
        &DropDrift::default(),
    )
}

#[test]
fn a_dry_sky_asks_for_no_drops_and_no_splashes() {
    let dry = RainInput {
        intensity: 0.0,
        ..downpour()
    };
    assert_eq!(Draws::wanted(&dry, &view()), Draws::NONE);
}

#[test]
fn the_drops_follow_the_density_of_the_weather_and_both_follow_the_intensity() {
    let full = Draws::wanted(&downpour(), &view());
    assert_eq!(
        full,
        Draws {
            drops: drop_count(1.0, 7.0),
            splashes: splash_count(1.0)
        }
    );
    let half = Draws::wanted(
        &RainInput {
            intensity: 0.5,
            ..downpour()
        },
        &view(),
    );
    assert_eq!(
        half,
        Draws {
            drops: drop_count(0.5, 7.0),
            splashes: splash_count(0.5)
        }
    );
    let dense = Draws::wanted(
        &RainInput {
            density: 100.0,
            ..downpour()
        },
        &view(),
    );
    assert_eq!(dense.drops, MAX_DROPS);
}

#[test]
fn no_rain_is_asked_for_under_water() {
    let under_water = RainView {
        underwater: true,
        ..view()
    };
    assert_eq!(Draws::wanted(&downpour(), &under_water), Draws::NONE);
}

#[test]
fn a_camera_that_is_no_point_sees_no_rain() {
    for eye in [Vec3::NAN, Vec3::new(0.0, f32::INFINITY, 0.0)] {
        let view = RainView { eye, ..view() };
        assert_eq!(Draws::wanted(&downpour(), &view), Draws::NONE, "eye {eye}");
    }
}

#[test]
fn the_map_is_handed_over_as_the_window_places_it() {
    let window = window();
    let uniform = uniform_of(&downpour(), &view());
    assert_eq!(
        uniform.window,
        [
            window.origin().x,
            window.origin().y,
            EXTENT,
            window.bottom()
        ]
    );
    assert_eq!(uniform.heights, [HEIGHT_RANGE, 1.0, 0.0, 0.0]);
}

#[test]
fn roofs_shelter_only_when_the_weather_lets_them() {
    let open = RainInput {
        occludable: false,
        ..downpour()
    };
    assert_eq!(uniform_of(&open, &view()).heights[1], 0.0);
    assert_eq!(
        Draws::wanted(&open, &view()).splashes,
        splash_count(1.0),
        "the map still places splashes"
    );
}

#[test]
fn the_units_of_the_weather_become_engine_units() {
    let input = RainInput {
        wind: Vec2::new(100.0, -50.0),
        ..downpour()
    };
    let fall = Fall::of(&input);
    assert!(
        (fall.velocity - Vec3::new(10.0, -75.0, -5.0))
            .abs()
            .max_element()
            < 1e-4
    );
    assert!((fall.edge - 120.0).abs() < 1e-3);

    let uniform = uniform_of(&input, &view());
    assert!(
        (uniform.along[3] - 1.5 * STREAK_LENGTH_SCALE * ENGINE_UNITS_PER_METER).abs() < 1e-4,
        "a streak is as long as the particle size x says"
    );
    assert!(
        (uniform.across[3] - 0.35 * STREAK_WIDTH_SCALE * ENGINE_UNITS_PER_METER).abs() < 1e-4,
        "and as wide as the particle size y says"
    );
}

#[test]
fn a_box_is_never_smaller_than_its_least_edge() {
    let fall = Fall::of(&RainInput {
        box_size: 0.0,
        ..downpour()
    });
    assert!((fall.edge - MIN_BOX_EDGE).abs() < f32::EPSILON);
}

#[test]
fn the_looks_of_the_rain_are_handed_over_as_tuned() {
    let uniform = uniform_of(&downpour(), &view());
    assert_eq!(
        uniform.looks,
        [STREAK_OPACITY, SPLASH_OPACITY, LEAST_VARIATION, 0.0]
    );
}

#[test]
fn the_streaks_catch_the_light_through_the_raindrop_material_and_the_splashes_as_it_is() {
    let light = Vec3::new(0.2, 0.4, 0.8);
    let base_color = Vec3::new(0.5, 0.25, 1.0);

    let uniform = uniform_of(
        &RainInput {
            tint: light,
            streak_material: StreakMaterial {
                base_color,
                ..StreakMaterial::FALLBACK
            },
            ..downpour()
        },
        &view(),
    );

    assert_eq!(uniform.tint, light.extend(0.0).to_array());
    assert_eq!(
        uniform.streak_tint,
        (light * base_color).extend(0.0).to_array()
    );
}

#[test]
fn a_texture_of_no_subtextures_is_one_whole_streak() {
    let uniform = uniform_of(&downpour(), &view());
    assert_eq!(&uniform.atlas[..2], &[1.0, 1.0]);
    let atlas = uniform_of(
        &RainInput {
            subtextures: UVec2::new(4, 2),
            ..downpour()
        },
        &view(),
    );
    assert_eq!(&atlas.atlas[..2], &[4.0, 2.0]);
}

#[test]
fn streaks_lean_back_against_the_wind_and_face_the_camera() {
    let leaning = RainInput {
        wind: Vec2::new(250.0, 0.0),
        ..downpour()
    };
    let uniform = uniform_of(&leaning, &view());
    let along = Vec3::new(uniform.along[0], uniform.along[1], uniform.along[2]);
    let across = Vec3::new(uniform.across[0], uniform.across[1], uniform.across[2]);
    assert!(along.x < 0.0 && along.y > 0.0, "{along}");
    assert!(across.dot(along).abs() < 1e-5);
    assert!(across.dot(Vec3::NEG_Z).abs() < 1e-5, "{across}");
}

#[test]
fn drops_need_a_size_to_be_drawn_but_splashes_do_not() {
    for size in [Vec2::ZERO, Vec2::new(1.5, 0.0), Vec2::new(-1.0, 0.35)] {
        let draws = Draws::wanted(&RainInput { size, ..downpour() }, &view());
        assert_eq!(
            draws,
            Draws {
                drops: 0,
                splashes: splash_count(1.0)
            },
            "size {size}"
        );
    }
}

#[test]
fn a_weather_of_nonsense_still_hands_the_shader_finite_numbers() {
    let nonsense = RainInput {
        intensity: f32::NAN,
        fall_speed: f32::INFINITY,
        size: Vec2::NAN,
        density: f32::NAN,
        box_size: f32::NEG_INFINITY,
        wind: Vec2::new(f32::NAN, f32::INFINITY),
        tint: Vec3::new(f32::NAN, -1.0, f32::INFINITY),
        ..downpour()
    };
    let view = RainView {
        eye: Vec3::NAN,
        forward: Vec3::new(f32::INFINITY, 0.0, 0.0),
        time: f64::NAN,
        ..view()
    };
    assert_eq!(Draws::wanted(&nonsense, &view), Draws::NONE);
    let uniform = uniform_of(&nonsense, &view);
    let words: &[u32] = bytemuck::cast_slice(std::slice::from_ref(&uniform));
    let floats = &words[..words.len() - 4];
    assert!(
        floats.iter().all(|&bits| f32::from_bits(bits).is_finite()),
        "{uniform:?}"
    );
}

#[test]
fn the_splashes_follow_the_clock_of_the_game() {
    let at = |time: f64| uniform_of(&downpour(), &RainView { time, ..view() });
    let start = at(0.0);
    assert_eq!((start.splash[0], start.splash_cycle[0]), (0.0, 0));
    let later = at(SPLASH_SECONDS * 3.25);
    assert_eq!(later.splash_cycle[0], 3);
    assert!((later.splash[0] - 0.25).abs() < 1e-6);
    assert_eq!(
        later.splash[1], start.splash[1],
        "the same circle of splashes"
    );
}

/// The name and byte offset of each of the listed fields of the uniform, as Rust lays them out.
macro_rules! offsets_of {
    ($($field:ident),+ $(,)?) => {
        [$((stringify!($field), std::mem::offset_of!(RainUniform, $field))),+]
    };
}

#[test]
fn the_uniform_is_laid_out_as_the_shader_declares_it() {
    let module = naga::front::wgsl::parse_str(shaders::RAIN.wgsl()).expect("the shader parses");
    let (members, span) = module
        .types
        .iter()
        .find_map(|(_, ty)| match (&ty.name, &ty.inner) {
            (Some(name), naga::TypeInner::Struct { members, span }) if name == "RainUniform" => {
                Some((members, *span))
            }
            _ => None,
        })
        .expect("the shader declares RainUniform");

    let declared: Vec<_> = members
        .iter()
        .map(|member| {
            (
                member.name.as_deref().unwrap_or_default(),
                member.offset as usize,
            )
        })
        .collect();
    let laid_out = offsets_of!(
        along,
        across,
        lattice,
        tint,
        streak_tint,
        atlas,
        window,
        heights,
        fades,
        looks,
        splash,
        splash_cycle
    );
    assert_eq!(declared, laid_out);
    assert_eq!(span as usize, std::mem::size_of::<RainUniform>());
}

#[test]
fn the_shader_has_an_entry_point_for_each_of_the_two_pipelines() {
    let module = naga::front::wgsl::parse_str(shaders::RAIN.wgsl()).expect("the shader parses");
    let entries: Vec<_> = module
        .entry_points
        .iter()
        .map(|entry| (entry.name.as_str(), entry.stage))
        .collect();
    for expected in [
        ("vs_streak", naga::ShaderStage::Vertex),
        ("fs_streak", naga::ShaderStage::Fragment),
        ("vs_splash", naga::ShaderStage::Vertex),
        ("fs_splash", naga::ShaderStage::Fragment),
    ] {
        assert!(entries.contains(&expected), "{expected:?} in {entries:?}");
    }
}
