//! The splashes of a frame: where they stand and what they show.

use std::path::Path;

use glam::{Mat4, UVec2, Vec3, Vec4};

use super::scene::{Look, NO_FOG, SPLASH_ATLAS, Scene, TARGET, is_lit, lit, roof, write_texture};
use super::{EYE, downpour};
use crate::rendering::rain::RainInput;
use crate::rendering::rain::assets::SPLASH_FRAMES;
use crate::rendering::rain::tuning::SPLASH_SECONDS;
use crate::rendering::shadows::test_rig::Slab;

fn splashes_only() -> RainInput {
    RainInput {
        density: 0.0,
        ..downpour()
    }
}

#[test]
fn splashes_stand_only_on_the_ground_the_map_knows() {
    let ground = Slab {
        centre: EYE + Vec3::new(60.0, -35.0, -60.0),
        half_size: 60.0,
        alpha_tested: false,
    };
    let Some(mut scene) = Scene::new("splashes_stand_only_on_the_ground_the_map_knows", &[ground])
    else {
        return;
    };
    let looking_down = Look {
        eye: EYE,
        forward: Vec3::new(0.0, -1.0, -1.0).normalize(),
    };

    let lit = lit(&scene.draw(&splashes_only(), looking_down));

    let right = lit.iter().filter(|pixel| pixel.x >= TARGET / 2).count();
    let far_left = lit.iter().filter(|pixel| pixel.x < TARGET / 2 - 8).count();
    assert!(right > 30, "{right} pixels on the ground");
    assert_eq!(far_left, 0, "splashes where the map holds no height");
}

#[test]
fn splashes_stand_at_the_height_the_map_holds() {
    let Some(mut scene) = Scene::new("splashes_stand_at_the_height_the_map_holds", &[roof(0.0)])
    else {
        return;
    };

    let lit = lit(&scene.draw(&splashes_only(), Look::AHEAD));

    let horizon = TARGET / 2;
    let lowest = lit.iter().map(|pixel| pixel.y).max();
    assert!(lit.len() > 20, "{} pixels", lit.len());
    assert!(
        lowest.is_some_and(|row| (horizon - 2..horizon + 2).contains(&row)),
        "the splashes end at row {lowest:?}, the ground at the horizon row {horizon}"
    );
}

#[test]
fn a_world_without_ground_has_no_splashes_to_see() {
    let Some(mut scene) = Scene::new("a_world_without_ground_has_no_splashes", &[]) else {
        return;
    };

    let pixels = scene.draw(
        &splashes_only(),
        Look {
            eye: EYE,
            forward: Vec3::NEG_Y,
        },
    );

    assert_eq!(lit(&pixels).len(), 0, "splashes on nothing");
}

const FRAME_TEXELS: u32 = 16;
const LAST_FRAME: u32 = SPLASH_FRAMES - 1;

/// The sRGB byte that decodes to the linear value `linear`.
fn srgb_byte(linear: f32) -> u8 {
    let encoded = if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}

/// The red, in linear light, that says a texel belongs to frame `frame` of the gradient atlas.
fn frame_red(frame: u32) -> f32 {
    (frame as f32 + 0.5) / SPLASH_FRAMES as f32
}

/// An atlas of opaque frames, each one flat colour whose red says which frame it is.
fn write_gradient_atlas(root: &Path) {
    let size = UVec2::new(SPLASH_FRAMES * FRAME_TEXELS, 4);
    write_texture(root, SPLASH_ATLAS, size, |texel| {
        [srgb_byte(frame_red(texel.x / FRAME_TEXELS)), 255, 255, 255]
    });
}

/// The frame of the gradient atlas each pixel shows; `None` where no splash is, or where the
/// colour is no clean frame, as at the border of a splash.
fn frames_shown(pixels: &[Vec4]) -> Vec<Option<u32>> {
    pixels
        .iter()
        .map(|pixel| {
            let frame = pixel.x * SPLASH_FRAMES as f32 - 0.5;
            let clean = (frame - frame.round()).abs() < 0.2 && frame > -0.2;
            (is_lit(*pixel) && clean).then(|| frame.round() as u32)
        })
        .collect()
}

/// What every pixel of a ground of splashes shows, in a frame at each of the 16 steps from the
/// start of a splash cycle to the start of the next.
fn frames_of_a_cycle(test: &str) -> Option<Vec<Vec<Option<u32>>>> {
    let ground = Slab {
        centre: EYE + Vec3::new(0.0, -70.0, 0.0),
        half_size: 250.0,
        alpha_tested: false,
    };
    let mut scene = Scene::with_atlas(test, &[ground], write_gradient_atlas)?;
    scene.projection = Mat4::perspective_infinite_reverse_rh(std::f32::consts::FRAC_PI_6, 1.0, 0.5);
    let looking_down = Look {
        eye: EYE,
        forward: Vec3::new(0.0, -0.866, -0.5),
    };
    let step = SPLASH_SECONDS / f64::from(SPLASH_FRAMES);
    let start = 10.0;
    Some(
        (0..=SPLASH_FRAMES)
            .map(|k| {
                let time = start + f64::from(k) * step;
                frames_shown(&scene.draw_at(&splashes_only(), looking_down, NO_FOG, time))
            })
            .collect(),
    )
}

/// The frame each pixel showed in a step and what it shows in the next, for every pixel that
/// showed a frame.
fn steps(frames: &[Vec<Option<u32>>]) -> impl Iterator<Item = (u32, Option<u32>)> + '_ {
    frames.iter().zip(&frames[1..]).flat_map(|(now, next)| {
        now.iter()
            .zip(next)
            .filter_map(|(&before, &after)| before.map(|frame| (frame, after)))
    })
}

#[test]
fn a_splash_plays_its_frames_in_turn() {
    let Some(frames) = frames_of_a_cycle("a_splash_plays_its_frames_in_turn") else {
        return;
    };

    let mut shown = [0usize; SPLASH_FRAMES as usize];
    let (mut followed, mut advanced) = (0, 0);
    for (frame, after) in steps(&frames) {
        shown[frame as usize] += 1;
        if frame < LAST_FRAME {
            followed += 1;
            advanced += usize::from(after == Some(frame + 1));
        }
    }

    let average = shown.iter().sum::<usize>() / shown.len();
    assert!(
        shown.iter().all(|&pixels| pixels * 2 > average),
        "pixels of each frame: {shown:?}"
    );
    assert!(followed > 500, "{followed} pixels followed");
    assert!(
        advanced * 100 >= followed * 85,
        "{advanced} of {followed} pixels went on to the next frame"
    );
}

#[test]
fn a_splash_lands_elsewhere_in_the_next_cycle() {
    let Some(frames) = frames_of_a_cycle("a_splash_lands_elsewhere_in_the_next_cycle") else {
        return;
    };

    let ends: Vec<_> = steps(&frames)
        .filter(|&(frame, _)| frame == LAST_FRAME)
        .collect();

    let stayed = ends.iter().filter(|&&(_, after)| after == Some(0)).count();
    assert!(
        ends.len() > 50,
        "{} pixels reached the last frame",
        ends.len()
    );
    assert!(
        stayed * 100 <= ends.len() * 15,
        "{stayed} of {} pixels began the next cycle where the last one ended",
        ends.len()
    );
}
