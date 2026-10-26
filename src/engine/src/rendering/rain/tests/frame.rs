//! The rain of a frame: its streaks, what shelters them, and what a frame draws of the rain it
//! was, and was not, worked out for.

use glam::{Mat4, UVec2, Vec3, Vec4};

use super::scene::{BACKGROUND, Fog, Look, NO_FOG, Scene, TARGET, lit, roof, roof_over};
use super::{EYE, downpour};
use crate::rendering::rain::motion::{drop_count, splash_count};
use crate::rendering::rain::uniform::{Draws, Fall};
use crate::rendering::rain::{RainInput, RainTextureState};

#[test]
fn both_pipelines_are_created_against_the_real_frame_layout() {
    let Some(scene) = Scene::new(
        "both_pipelines_are_created_against_the_real_frame_layout",
        &[],
    ) else {
        return;
    };
    assert_eq!(scene.rain.asset_status().splashes, RainTextureState::Loaded);
    assert_eq!(
        scene.rain.asset_status().streaks,
        RainTextureState::NotRequested
    );
}

#[test]
fn a_dry_sky_records_no_draw_and_leaves_the_scene_as_it_is() {
    let Some(mut scene) = Scene::new("a_dry_sky_records_no_draw", &[]) else {
        return;
    };
    let dry = RainInput {
        intensity: 0.0,
        ..downpour()
    };

    let draws = scene.prepared_draws(&dry, Look::AHEAD);
    let pixels = scene.draw(&dry, Look::AHEAD);

    assert_eq!(draws, Draws::NONE);
    assert_eq!(lit(&pixels).len(), 0, "rain over a clear scene");
    assert_eq!(
        scene.rain.asset_status().streaks,
        RainTextureState::NotRequested,
        "nothing is loaded while it does not rain"
    );
    assert_eq!(scene.rain.map.window(), None, "and no map is drawn for it");
}

#[test]
fn a_downpour_draws_streaks_over_the_scene() {
    let Some(mut scene) = Scene::new("a_downpour_draws_streaks_over_the_scene", &[]) else {
        return;
    };

    let draws = scene.prepared_draws(&downpour(), Look::AHEAD);
    let pixels = scene.draw(&downpour(), Look::AHEAD);

    assert_eq!(
        draws,
        Draws {
            drops: drop_count(1.0, 7.0),
            splashes: splash_count(1.0)
        }
    );
    let lit = lit(&pixels);
    assert!(lit.len() > 200, "{} pixels", lit.len());
    let brightest = pixels.iter().map(|pixel| pixel.x).fold(0.0, f32::max);
    assert!(brightest > BACKGROUND + 0.1, "{brightest}");
}

#[test]
fn scenery_in_front_hides_the_rain() {
    let Some(mut scene) = Scene::new("scenery_in_front_hides_the_rain", &[]) else {
        return;
    };
    let in_the_open = lit(&scene.draw(&downpour(), Look::AHEAD)).len();
    scene.backdrop_depth = 1.0;

    let behind_scenery = lit(&scene.draw(&downpour(), Look::AHEAD)).len();

    assert!(in_the_open > 200, "{in_the_open} pixels");
    assert_eq!(behind_scenery, 0);
}

#[test]
fn a_roof_over_the_camera_shelters_the_drops_under_it() {
    let Some(mut scene) = Scene::new("a_roof_over_the_camera_shelters_the_drops", &[roof(70.0)])
    else {
        return;
    };

    let sheltered = scene.draw(&downpour(), Look::AHEAD);
    let open = scene.draw(
        &RainInput {
            occludable: false,
            ..downpour()
        },
        Look::AHEAD,
    );

    assert_eq!(lit(&sheltered).len(), 0, "pixels of rain under the roof");
    assert!(lit(&open).len() > 200, "{} pixels", lit(&open).len());
}

#[test]
fn a_roof_at_the_height_of_the_camera_cuts_the_streaks_off_at_the_horizon() {
    let Some(mut scene) = Scene::new(
        "a_roof_at_the_height_of_the_camera_cuts_the_streaks",
        &[roof(0.0)],
    ) else {
        return;
    };

    let lit = lit(&scene.draw(&downpour(), Look::AHEAD));

    let horizon = TARGET / 2;
    let above = lit.iter().filter(|pixel| pixel.y < horizon).count();
    let below = lit.iter().filter(|pixel| pixel.y > horizon + 1).count();
    assert!(above > 100, "{above} pixels over the horizon");
    assert_eq!(below, 0, "streaks reach under the roof");
}

#[test]
fn a_camera_one_box_edge_further_sees_the_same_rain() {
    let Some(mut scene) = Scene::new("a_camera_one_box_edge_further_sees_the_same_rain", &[])
    else {
        return;
    };
    let edge = Fall::of(&downpour()).edge;
    let first = scene.draw(&downpour(), Look::AHEAD);

    let second = scene.draw(
        &downpour(),
        Look {
            eye: EYE + Vec3::new(edge, 0.0, -2.0 * edge),
            ..Look::AHEAD
        },
    );

    let difference: f32 = first
        .iter()
        .zip(&second)
        .map(|(a, b)| (*a - *b).abs().max_element())
        .sum();
    let seen = lit(&first).len() as f32;
    assert!(seen > 200.0);
    assert!(
        difference < 0.02 * seen,
        "{difference} against {seen} pixels"
    );
}

#[test]
fn the_drops_stay_where_they_are_in_the_world_while_the_camera_moves() {
    let Some(mut scene) = Scene::new("the_drops_stay_where_they_are_in_the_world", &[]) else {
        return;
    };
    // One engine unit to the pixel, so a camera step of whole units shifts the image by whole
    // pixels.
    let half_view = TARGET as f32 / 2.0;
    scene.projection =
        Mat4::orthographic_rh(-half_view, half_view, -half_view, half_view, 2000.0, 0.0);
    let step = 24;
    let few = RainInput {
        density: 0.15,
        ..downpour()
    };
    let before = lit(&scene.draw(&few, Look::AHEAD));
    let after = lit(&scene.draw(
        &few,
        Look {
            eye: EYE + Vec3::new(step as f32, 0.0, 0.0),
            ..Look::AHEAD
        },
    ));

    // The part of the view both cameras see well inside their boxes, where nothing fades.
    let seen_by_both =
        |pixel: &&UVec2| (28..100).contains(&pixel.y) && (52..100).contains(&pixel.x);
    let before: Vec<UVec2> = before.iter().filter(seen_by_both).copied().collect();
    let moved_on = |pixel: &UVec2| UVec2::new(pixel.x - step, pixel.y);
    let missing = before
        .iter()
        .filter(|pixel| !after.contains(&moved_on(pixel)))
        .count();
    assert!(before.len() > 100, "{} pixels", before.len());
    assert!(
        missing * 20 < before.len(),
        "{missing} of {} pixels did not move with the world",
        before.len()
    );
}

#[test]
fn the_rain_falls_as_time_passes() {
    let Some(mut scene) = Scene::new("the_rain_falls_as_time_passes", &[]) else {
        return;
    };
    scene.draw_at(&downpour(), Look::AHEAD, NO_FOG, 5.0);
    let before = scene.draw_at(&downpour(), Look::AHEAD, NO_FOG, 5.0);

    let after = scene.draw_at(&downpour(), Look::AHEAD, NO_FOG, 5.05);

    let moved = before
        .iter()
        .zip(&after)
        .filter(|(a, b)| (**a - **b).abs().max_element() > 0.02)
        .count();
    assert!(moved > 200, "{moved} pixels changed");
}

#[test]
fn the_fog_covers_the_rain_like_everything_else() {
    let Some(mut scene) = Scene::new("the_fog_covers_the_rain_like_everything_else", &[]) else {
        return;
    };
    let clear = scene.draw(&downpour(), Look::AHEAD);
    let fogged = scene.draw_at(
        &downpour(),
        Look::AHEAD,
        Fog {
            colour: Vec3::new(1.0, 0.0, 0.0),
            opacity: 1.0,
        },
        0.0,
    );

    let in_clear = lit(&clear);
    let in_fog = lit(&fogged);
    assert!(in_clear.len() > 200 && in_fog.len() > 200);
    let green_of = |pixels: &[Vec4], lit: &[UVec2]| {
        lit.iter()
            .map(|pixel| pixels[(pixel.y * TARGET + pixel.x) as usize].y)
            .fold(f32::INFINITY, f32::min)
    };
    assert!(
        green_of(&clear, &in_clear) >= BACKGROUND - 1e-3,
        "white rain"
    );
    assert!(green_of(&fogged, &in_fog) < BACKGROUND - 0.02, "red rain");
}

#[test]
fn a_missing_streak_texture_is_reported_and_draws_no_streaks() {
    let Some(mut scene) = Scene::new("a_missing_streak_texture_is_reported", &[]) else {
        return;
    };
    let missing = RainInput {
        texture: "effects\\nothing_here.dds".into(),
        ..downpour()
    };

    let draws = scene.prepared_draws(&missing, Look::AHEAD);
    let pixels = scene.draw(&missing, Look::AHEAD);

    assert_eq!(
        draws,
        Draws {
            drops: 0,
            splashes: splash_count(1.0)
        },
        "the splashes do not wait for the streak texture"
    );
    assert_eq!(lit(&pixels).len(), 0, "rain over a clear scene");
    match &scene.rain.asset_status().streaks {
        RainTextureState::Failed(message) => {
            assert!(message.contains("nothing_here.dds"), "{message}");
            assert!(message.contains("not in the install"), "{message}");
        }
        other => panic!("expected a failed load, found {other:?}"),
    }
    let found = scene.draw(&downpour(), Look::AHEAD);
    assert!(
        lit(&found).len() > 200,
        "the next weather brings its texture"
    );
    assert_eq!(scene.rain.asset_status().streaks, RainTextureState::Loaded);
}

#[test]
fn a_texture_that_is_still_the_streak_texture_is_not_loaded_again() {
    let Some(mut scene) = Scene::new("a_texture_that_is_still_the_streak_texture", &[]) else {
        return;
    };
    scene.draw(&downpour(), Look::AHEAD);
    std::fs::remove_file(scene.assets.path().join("textures/effects/test_rain.dds")).unwrap();

    let pixels = scene.draw(&downpour(), Look::AHEAD);

    assert_eq!(scene.rain.asset_status().streaks, RainTextureState::Loaded);
    assert!(lit(&pixels).len() > 200);
}

#[test]
fn the_map_follows_the_camera_to_the_roof_it_stands_under() {
    let far = EYE + Vec3::new(700.0, 0.0, 0.0);
    let Some(mut scene) = Scene::new(
        "the_map_follows_the_camera_to_the_roof",
        &[roof_over(far, 70.0)],
    ) else {
        return;
    };

    let at_the_start = scene.draw(&downpour(), Look::AHEAD);
    let under_the_roof = scene.draw(
        &downpour(),
        Look {
            eye: far,
            ..Look::AHEAD
        },
    );

    assert!(lit(&at_the_start).len() > 200, "open ground at the start");
    assert_eq!(
        lit(&under_the_roof).len(),
        0,
        "pixels of rain under the roof"
    );
}

#[test]
fn the_map_is_drawn_again_once_the_world_around_the_camera_has_changed() {
    let Some(mut scene) = Scene::new("the_map_is_drawn_again_once_the_world", &[]) else {
        return;
    };
    assert!(lit(&scene.draw(&downpour(), Look::AHEAD)).len() > 200);
    scene.rig.place(&[roof(70.0)], &[]);

    let before_it_is_told = scene.draw(&downpour(), Look::AHEAD);
    scene.generation += 1;
    let after = scene.draw(&downpour(), Look::AHEAD);

    assert!(
        lit(&before_it_is_told).len() > 200,
        "the map holds until the streamer says the world changed"
    );
    assert_eq!(lit(&after).len(), 0, "pixels of rain under the new roof");
}

#[test]
fn a_frame_that_works_out_no_rain_draws_none() {
    let Some(mut scene) = Scene::new("a_frame_that_works_out_no_rain_draws_none", &[]) else {
        return;
    };
    assert!(lit(&scene.draw(&downpour(), Look::AHEAD)).len() > 200);

    let unprepared = scene.draw_unprepared(Look::AHEAD);

    assert_eq!(lit(&unprepared).len(), 0, "last frame's rain again");
}

#[test]
fn no_rain_falls_under_water() {
    let Some(mut scene) = Scene::new("no_rain_falls_under_water", &[]) else {
        return;
    };
    scene.underwater = true;

    let draws = scene.prepared_draws(&downpour(), Look::AHEAD);
    let under_water = scene.draw(&downpour(), Look::AHEAD);

    assert_eq!(draws, Draws::NONE);
    assert_eq!(lit(&under_water).len(), 0, "rain under water");
    assert_eq!(scene.rain.map.window(), None, "and no map is drawn for it");
    scene.underwater = false;
    assert!(
        lit(&scene.draw(&downpour(), Look::AHEAD)).len() > 200,
        "it rains again at the surface"
    );
}
