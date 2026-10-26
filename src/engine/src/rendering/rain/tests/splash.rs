//! The splashes of a frame: where they stand and what they show.

use glam::Vec3;

use super::scene::{Look, Scene, TARGET, lit, roof};
use super::{EYE, downpour};
use crate::rendering::rain::RainInput;
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
