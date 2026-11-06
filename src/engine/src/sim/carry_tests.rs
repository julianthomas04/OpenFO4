//! Carrying, driven through `SimWorld` the way the render side drives it.

use std::f32::consts::FRAC_PI_2;

use glam::{Vec2, Vec3};

use super::carry::{View, throw_speed};
use super::physics::test_support::{BodyProbe, body_probe, box_shape};
use super::pipboy::PipBoyAction;
use super::test_world::{
    Prop, SPAWN_AT, crate_prop, hold, position, prop_box, prop_entity, spawned_at, tick, walking,
    world_with,
};
use super::{
    CarriedInfo, EntityId, GrabRefusal, PlayerInput, PlayerMovement, ReleaseReason,
    STANDING_CENTER_ABOVE_FEET, SimCommand, SimWorld, refr_entity_id,
};

/// Where the crate rests in most tests: on the ground, ahead of the player along +X.
const CRATE_AT: Vec3 = Vec3::new(206.0, 0.5, -100.0);
/// FO4's 9 units, in engine units: how much closer than at grab time the body is held.
const HOLD_INSET: f32 = 0.9;
const LIGHT_MASS: f32 = 10.0;

/// A crate of `mass` large enough (2 x 2 x 2) that the density clamp keeps its mass.
fn big_crate(at: Vec3, mass: f32) -> Prop {
    Prop {
        shape: box_shape(1.0),
        ..crate_prop(at, mass)
    }
}

fn crate_entity() -> EntityId {
    prop_entity(0)
}

fn crate_state(sim: &SimWorld) -> BodyProbe {
    body_probe(&sim.physics, crate_entity())
}

fn eye(sim: &SimWorld) -> Vec3 {
    sim.player_eye().expect("the player is placed")
}

/// The view from the player's eye onto `point`.
fn view_onto(sim: &SimWorld, point: Vec3) -> View {
    let to = point - eye(sim);
    View {
        yaw: to.z.atan2(to.x),
        pitch: to.y.atan2(to.x.hypot(to.z)),
    }
}

fn looking(view: View) -> PlayerInput {
    PlayerInput {
        yaw: view.yaw,
        pitch: view.pitch,
        ..PlayerInput::default()
    }
}

fn grab(sim: &mut SimWorld) {
    sim.apply(SimCommand::GrabBody {
        entity: crate_entity(),
    });
}

/// The player at `SPAWN_AT` in a world of `props` (the first is the crate), looking at the
/// crate and carrying it.
fn carrying(props: &[Prop]) -> SimWorld {
    let mut sim = spawned_at(world_with(props), SPAWN_AT);
    let view = view_onto(&sim, props[0].at);
    hold(&mut sim, looking(view));
    grab(&mut sim);
    assert_eq!(sim.take_grab_refusal(), None);
    assert_eq!(
        sim.carried().map(|carried| carried.entity),
        Some(crate_entity())
    );
    sim
}

fn light_crate_carried() -> SimWorld {
    carrying(&[crate_prop(CRATE_AT, LIGHT_MASS)])
}

#[test]
fn a_carried_crate_reaches_and_holds_its_distance_at_every_pitch() {
    let mut sim = light_crate_carried();
    let hold_distance = CRATE_AT.distance(eye(&sim)) - HOLD_INSET;
    let yaw = view_onto(&sim, CRATE_AT).yaw;
    for pitch in [-0.6, -0.2, 0.0, 0.3, 0.6] {
        let view = View { yaw, pitch };
        hold(&mut sim, looking(view));
        tick(&mut sim, 30);
        let offset = crate_state(&sim).translation - eye(&sim);
        assert!(
            (offset.length() - hold_distance).abs() < 0.05,
            "pitch {pitch}: held at {}, wanted {hold_distance}",
            offset.length()
        );
        assert!(
            offset.normalize().abs_diff_eq(view.forward(), 1e-2),
            "pitch {pitch}: {offset} is off the view"
        );
    }
    assert!(sim.carried().is_some());
}

#[test]
fn the_carried_crate_keeps_its_rotation_relative_to_the_camera() {
    let mut sim = light_crate_carried();
    let start = view_onto(&sim, CRATE_AT);
    let turned = View {
        yaw: start.yaw + 0.8,
        ..start
    };
    hold(&mut sim, looking(turned));
    tick(&mut sim, 30);
    let expected = turned.rotation() * start.rotation().inverse();
    assert!(
        crate_state(&sim).rotation.angle_between(expected) < 0.01,
        "{:?}",
        crate_state(&sim).rotation
    );
}

#[test]
fn holding_a_mouse_button_spins_the_crate_at_the_measured_rate() {
    let mut sim = light_crate_carried();
    let view = view_onto(&sim, CRATE_AT);
    tick(&mut sim, 5);
    let before = crate_state(&sim).rotation;
    hold(
        &mut sim,
        PlayerInput {
            spin: 1,
            ..looking(view)
        },
    );
    tick(&mut sim, 30);
    hold(&mut sim, looking(view));
    tick(&mut sim, 30);
    let turned = crate_state(&sim).rotation * before.inverse();
    let (axis, angle) = turned.to_axis_angle();
    assert!(
        (angle.to_degrees() - 86.0).abs() < 1.0,
        "half a second turns 86 degrees, turned {}",
        angle.to_degrees()
    );
    assert!(
        axis.cross(view.right()).length() < 1e-2,
        "about the camera's right axis: {axis}"
    );
}

#[test]
fn a_wall_stops_the_carried_crate_and_pushing_into_it_never_tears_it_away() {
    let face = 211.5;
    let wall = prop_box(
        "meshes\\wall.glb",
        Vec3::new(face + 0.5, 10.0, -100.0),
        Vec3::new(0.5, 10.0, 50.0),
    );
    let mut sim = carrying(&[crate_prop(CRATE_AT, LIGHT_MASS), wall]);
    let view = view_onto(&sim, CRATE_AT);
    hold(
        &mut sim,
        PlayerInput {
            yaw: view.yaw,
            pitch: view.pitch,
            ..walking(view.yaw)
        },
    );
    for _ in 0..240 {
        tick(&mut sim, 1);
        let crate_x = crate_state(&sim).translation.x;
        assert!(crate_x + 0.5 < face + 0.05, "passed the wall: {crate_x}");
    }
    assert!(sim.carried().is_some(), "still held after four seconds");
    assert_eq!(sim.take_release(), None);
}

#[test]
fn a_crate_held_back_by_a_wall_is_torn_away_after_the_strain_lasts() {
    let face = 203.25;
    let wall = prop_box(
        "meshes\\wall.glb",
        Vec3::new(face - 0.25, 10.0, -100.0),
        Vec3::new(0.25, 10.0, 50.0),
    );
    let behind_wall = Vec3::new(205.5, 0.5, -100.0);
    let mut sim = carrying(&[crate_prop(behind_wall, LIGHT_MASS), wall]);
    let view = view_onto(&sim, behind_wall);
    hold(
        &mut sim,
        PlayerInput {
            yaw: view.yaw,
            pitch: view.pitch,
            move_dir: Vec2::new(0.0, -1.0),
            ..PlayerInput::default()
        },
    );
    let mut torn_at = None;
    for frame in 1..=180 {
        tick(&mut sim, 1);
        assert!(
            crate_state(&sim).translation.x - 0.5 > face - 0.05,
            "pulled through the wall"
        );
        if sim.carried().is_none() {
            torn_at = Some(frame);
            break;
        }
    }
    let torn_at = torn_at.expect("torn away");
    assert!(torn_at > 48, "not before the grace time: frame {torn_at}");
    assert_eq!(sim.take_release(), Some(ReleaseReason::TornAway));
}

#[test]
fn losing_sight_of_the_crate_does_not_drop_it() {
    let mut sim = light_crate_carried();
    let view = view_onto(&sim, CRATE_AT);
    hold(
        &mut sim,
        looking(View {
            yaw: view.yaw + std::f32::consts::PI,
            pitch: 0.0,
        }),
    );
    tick(&mut sim, 120);
    assert!(sim.carried().is_some());
}

#[test]
fn the_carried_crate_never_sleeps_nor_freezes_outside_the_physics_radius() {
    let mut sim = light_crate_carried();
    sim.apply(SimCommand::SetPhysicsCenter {
        grid: (10, 10),
        radius: 0,
    });
    tick(&mut sim, 180);
    let state = crate_state(&sim);
    assert!(state.dynamic, "refrozen");
    assert!(!state.sleeping, "put to sleep");
    assert_eq!(sim.stats().active_dynamic, 1);
    assert!(sim.carried().is_some());
}

#[test]
fn a_dropped_crate_may_sleep_again() {
    let mut sim = light_crate_carried();
    tick(&mut sim, 10);
    sim.apply(SimCommand::DropCarried);
    tick(&mut sim, 300);
    assert!(crate_state(&sim).sleeping);
}

#[test]
fn unloading_the_crates_cell_releases_it() {
    let mut sim = light_crate_carried();
    tick(&mut sim, 10);
    sim.apply(SimCommand::RemoveCellColliders { grid: (0, 0) });
    assert_eq!(sim.carried(), None);
    assert_eq!(sim.take_release(), Some(ReleaseReason::CellUnloaded));
    assert_eq!(sim.stats().dyn_total, 0);
    tick(&mut sim, 5);
}

#[test]
fn a_teleport_releases_the_crate() {
    let mut sim = light_crate_carried();
    sim.apply(SimCommand::TeleportPlayer {
        pos: Vec3::new(50.0, 20.0, -50.0),
    });
    assert_eq!(sim.carried(), None);
    assert_eq!(sim.take_release(), Some(ReleaseReason::Teleported));
}

#[test]
fn opening_the_pipboy_releases_the_crate() {
    let mut sim = light_crate_carried();
    sim.apply(SimCommand::PipBoy(PipBoyAction::Open));
    assert_eq!(sim.carried(), None);
    assert_eq!(sim.take_release(), Some(ReleaseReason::MenuOpened));
    assert_eq!(sim.take_release(), None, "each release is reported once");
}

#[test]
fn a_drop_releases_the_crate_with_the_velocity_it_had() {
    let mut sim = light_crate_carried();
    let view = view_onto(&sim, CRATE_AT);
    hold(
        &mut sim,
        looking(View {
            yaw: view.yaw + 0.5,
            ..view
        }),
    );
    tick(&mut sim, 2);
    let moving = crate_state(&sim).linvel;
    assert!(moving.length() > 1.0, "{moving}");
    sim.apply(SimCommand::DropCarried);
    assert_eq!(sim.carried(), None);
    assert_eq!(sim.take_release(), Some(ReleaseReason::Dropped));
    assert_eq!(crate_state(&sim).linvel, moving);
}

#[test]
fn a_throw_launches_the_crate_along_the_view_at_the_charged_speed() {
    let mut sim = light_crate_carried();
    let view = view_onto(&sim, CRATE_AT);
    tick(&mut sim, 5);
    sim.apply(SimCommand::ThrowCarried { charge_secs: 0.8 });
    assert_eq!(sim.carried(), None);
    assert_eq!(sim.take_release(), Some(ReleaseReason::Thrown));
    let expected = view.forward() * throw_speed(0.8);
    assert!(
        crate_state(&sim).linvel.abs_diff_eq(expected, 1e-3),
        "{}",
        crate_state(&sim).linvel
    );
}

/// Carrying a light crate in `props` (first) and holding it level in front of the eye.
fn held_level(props: &[Prop]) -> SimWorld {
    let mut sim = carrying(props);
    let level = View {
        yaw: view_onto(&sim, CRATE_AT).yaw,
        pitch: 0.0,
    };
    hold(&mut sim, looking(level));
    tick(&mut sim, 30);
    sim
}

#[test]
fn a_crate_thrown_at_the_plateau_speed_does_not_pass_through_a_thin_wall() {
    let front = crate_state(&held_level(&[crate_prop(CRATE_AT, LIGHT_MASS)]))
        .translation
        .x
        + 0.5;
    // One discrete step at the plateau speed would carry the crate clean past this wall.
    let face = front + 0.6;
    let half_thickness = 0.05;
    let wall = prop_box(
        "meshes\\wall.glb",
        Vec3::new(face + half_thickness, 10.0, -100.0),
        Vec3::new(half_thickness, 10.0, 50.0),
    );
    let mut sim = held_level(&[crate_prop(CRATE_AT, LIGHT_MASS), wall]);
    assert!((crate_state(&sim).translation.x + 0.5 - front).abs() < 1e-3);
    sim.apply(SimCommand::ThrowCarried { charge_secs: 3.0 });
    assert!((crate_state(&sim).linvel.x - throw_speed(3.0)).abs() < 1e-3);
    for _ in 0..30 {
        tick(&mut sim, 1);
        let crate_x = crate_state(&sim).translation.x;
        assert!(crate_x + 0.5 < face + 0.05, "tunnelled: {crate_x}");
    }
}

fn travelled_in_one_second(sim: &mut SimWorld, sprint: bool) -> f32 {
    let view = view_onto(sim, CRATE_AT);
    hold(
        sim,
        PlayerInput {
            sprint,
            pitch: view.pitch,
            ..walking(view.yaw)
        },
    );
    let start = position(sim);
    tick(sim, 60);
    (position(sim) - start).x
}

#[test]
fn a_heavy_carry_forces_walking_speed_even_while_sprinting() {
    let walk = PlayerMovement::default().walk_speed;
    for sprint in [false, true] {
        let mut sim = carrying(&[big_crate(CRATE_AT + Vec3::Y * 0.5, 100.0)]);
        let travelled = travelled_in_one_second(&mut sim, sprint);
        assert!(
            (travelled - walk).abs() < 0.5,
            "sprint {sprint}: {travelled}"
        );
        assert!(sim.carried().is_some());
    }
}

#[test]
fn a_light_carry_leaves_running_and_sprinting_unchanged() {
    let movement = PlayerMovement::default();
    for (sprint, speed) in [(false, movement.run_speed), (true, movement.sprint_speed)] {
        let mut sim = light_crate_carried();
        let travelled = travelled_in_one_second(&mut sim, sprint);
        assert!(
            (travelled - speed).abs() < 0.5,
            "sprint {sprint}: {travelled}"
        );
    }
}

#[test]
fn the_player_may_jump_while_carrying_a_heavy_crate() {
    let mut sim = carrying(&[big_crate(CRATE_AT + Vec3::Y * 0.5, 100.0)]);
    let ground = position(&sim).y;
    let view = view_onto(&sim, CRATE_AT);
    hold(
        &mut sim,
        PlayerInput {
            jump: true,
            ..looking(view)
        },
    );
    tick(&mut sim, 20);
    assert!(position(&sim).y > ground + 3.0, "{}", position(&sim).y);
    assert!(sim.carried().is_some());
}

#[test]
fn a_crate_held_under_the_feet_neither_lifts_nor_shoves_the_player() {
    let feet_level = Vec3::new(200.0, 0.5, -100.0);
    let empty = spawned_at(world_with(&[]), SPAWN_AT);
    let eye_height = eye(&empty).y - feet_level.y;
    let grab_distance = eye_height + HOLD_INSET;
    let beside = feet_level + Vec3::X * (grab_distance.powi(2) - eye_height.powi(2)).sqrt();
    let mut sim = carrying(&[crate_prop(beside, LIGHT_MASS)]);
    let start = position(&sim);
    hold(
        &mut sim,
        looking(View {
            yaw: 0.0,
            pitch: -FRAC_PI_2,
        }),
    );
    tick(&mut sim, 60);
    assert!(
        crate_state(&sim).translation.distance(feet_level) < 0.2,
        "the crate is under the feet: {}",
        crate_state(&sim).translation
    );
    assert!(
        position(&sim).abs_diff_eq(start, 0.02),
        "moved from {start} to {}",
        position(&sim)
    );
    assert!(sim.player_debug().grounded);
}

#[test]
fn a_player_standing_on_a_crate_is_never_lifted_by_carrying_it() {
    let crate_at = Vec3::new(200.0, 1.0, -100.0);
    let mut sim = spawned_at(
        world_with(&[big_crate(crate_at, 50.0)]),
        Vec3::new(200.0, 8.0, -100.0),
    );
    let on_crate = position(&sim);
    assert!(
        (on_crate.y - (2.0 + STANDING_CENTER_ABOVE_FEET)).abs() < 0.1,
        "the player starts on the crate: {on_crate}"
    );
    hold(
        &mut sim,
        looking(View {
            yaw: 0.0,
            pitch: -FRAC_PI_2,
        }),
    );
    grab(&mut sim);
    assert!(sim.carried().is_some());
    let mut highest = on_crate.y;
    for _ in 0..120 {
        tick(&mut sim, 1);
        highest = highest.max(position(&sim).y);
    }
    assert!(highest <= on_crate.y + 1e-3, "lifted to {highest}");
    let end = sim.player_debug();
    assert!(end.grounded);
    assert!(
        (end.pos.y - STANDING_CENTER_ABOVE_FEET).abs() < 0.1,
        "stands on the ground: {}",
        end.pos.y
    );
}

#[test]
fn a_grab_is_refused_for_a_body_heavier_than_the_limit() {
    let mut sim = spawned_at(world_with(&[big_crate(CRATE_AT, 120.0)]), SPAWN_AT);
    assert!(sim.is_grabbable(crate_entity()), "mass aside it is loose");
    grab(&mut sim);
    let Some(GrabRefusal::TooHeavy { mass }) = sim.take_grab_refusal() else {
        panic!("not refused as too heavy");
    };
    assert!((mass - 120.0).abs() < 0.1, "{mass}");
    assert_eq!(sim.carried(), None);
    assert_eq!(
        sim.take_grab_refusal(),
        None,
        "each refusal is reported once"
    );
}

#[test]
fn the_carried_mass_is_the_havok_mass() {
    let sim = carrying(&[big_crate(CRATE_AT, 100.0)]);
    let Some(CarriedInfo { mass, .. }) = sim.carried() else {
        panic!("carrying");
    };
    assert!((mass - 100.0).abs() < 0.1, "{mass}");
}

#[test]
fn a_second_grab_is_refused_while_carrying() {
    let mut sim = light_crate_carried();
    grab(&mut sim);
    assert_eq!(sim.take_grab_refusal(), Some(GrabRefusal::AlreadyCarrying));
    assert!(sim.carried().is_some());
}

#[test]
fn statics_and_unknown_entities_are_not_grabbable() {
    let pillar = prop_box(
        "meshes\\pillar.glb",
        Vec3::new(206.0, 2.0, -100.0),
        Vec3::splat(1.0),
    );
    let mut sim = spawned_at(world_with(&[pillar]), SPAWN_AT);
    for entity in [prop_entity(0), refr_entity_id("FFFFFF:Nowhere.esm")] {
        assert!(!sim.is_grabbable(entity));
        sim.apply(SimCommand::GrabBody { entity });
        assert_eq!(sim.take_grab_refusal(), Some(GrabRefusal::NotDynamic));
    }
}

#[test]
fn a_crate_frozen_outside_the_radius_is_not_grabbable() {
    let mut sim = spawned_at(world_with(&[crate_prop(CRATE_AT, LIGHT_MASS)]), SPAWN_AT);
    sim.apply(SimCommand::SetPhysicsCenter {
        grid: (10, 10),
        radius: 0,
    });
    assert!(!sim.is_grabbable(crate_entity()));
    grab(&mut sim);
    assert_eq!(sim.take_grab_refusal(), Some(GrabRefusal::NotDynamic));
}

#[test]
fn nothing_is_grabbed_before_the_player_is_placed() {
    let mut sim = world_with(&[crate_prop(CRATE_AT, LIGHT_MASS)]);
    grab(&mut sim);
    assert_eq!(sim.take_grab_refusal(), Some(GrabRefusal::PlayerNotPlaced));
}

#[test]
fn dropping_or_throwing_with_empty_hands_releases_nothing() {
    let mut sim = spawned_at(world_with(&[crate_prop(CRATE_AT, LIGHT_MASS)]), SPAWN_AT);
    let before = crate_state(&sim).linvel;
    sim.apply(SimCommand::DropCarried);
    sim.apply(SimCommand::ThrowCarried { charge_secs: 1.0 });
    assert_eq!(sim.take_release(), None);
    assert_eq!(crate_state(&sim).linvel, before);
}

#[test]
fn the_hold_distance_does_not_change_while_running_or_sprinting() {
    for sprint in [false, true] {
        let mut sim = light_crate_carried();
        let hold_distance = crate_state(&sim).translation.distance(sim.player.eye()) - HOLD_INSET;
        let view = view_onto(&sim, CRATE_AT);
        hold(
            &mut sim,
            PlayerInput {
                sprint,
                pitch: view.pitch,
                ..walking(view.yaw)
            },
        );
        for frame in 1..=60 {
            tick(&mut sim, 1);
            let held = crate_state(&sim).translation.distance(sim.player.eye());
            if frame > 2 {
                assert!(
                    (held - hold_distance).abs() < 0.05,
                    "sprint {sprint}, frame {frame}: held at {held}, wanted {hold_distance}"
                );
            }
        }
    }
}

#[test]
fn a_grab_is_refused_while_the_pipboy_is_open() {
    let mut sim = spawned_at(world_with(&[crate_prop(CRATE_AT, LIGHT_MASS)]), SPAWN_AT);
    sim.apply(SimCommand::PipBoy(PipBoyAction::Open));
    grab(&mut sim);
    assert_eq!(sim.take_grab_refusal(), Some(GrabRefusal::SimulationPaused));
    assert_eq!(sim.carried(), None);
}

#[test]
fn a_grab_is_refused_while_a_load_holds_the_simulation() {
    let mut sim = spawned_at(world_with(&[crate_prop(CRATE_AT, LIGHT_MASS)]), SPAWN_AT);
    sim.set_fold_hold(true);
    grab(&mut sim);
    assert_eq!(sim.take_grab_refusal(), Some(GrabRefusal::SimulationPaused));
    assert_eq!(sim.carried(), None);
}

#[test]
fn a_non_finite_view_lets_go_of_the_crate() {
    let mut sim = light_crate_carried();
    hold(
        &mut sim,
        looking(View {
            yaw: f32::NAN,
            pitch: 0.0,
        }),
    );
    tick(&mut sim, 1);
    assert_eq!(sim.carried(), None);
    assert_eq!(sim.take_release(), Some(ReleaseReason::InvalidHold));
}

/// (min, max) of each axis over `points`.
fn spread(points: &[Vec3]) -> Vec3 {
    let min = points.iter().copied().reduce(Vec3::min).expect("samples");
    let max = points.iter().copied().reduce(Vec3::max).expect("samples");
    max - min
}

#[test]
fn walking_the_carried_crate_into_a_wall_brings_the_player_up_to_the_wall_with_it_still_held() {
    let face = 220.0;
    let wall = prop_box(
        "meshes/wall.glb",
        Vec3::new(face + 0.5, 10.0, -100.0),
        Vec3::new(0.5, 10.0, 50.0),
    );
    let mut sim = carrying(&[crate_prop(CRATE_AT, LIGHT_MASS), wall]);
    let start = view_onto(&sim, CRATE_AT);
    let hold_distance = crate_state(&sim).translation.distance(sim.player.eye()) - HOLD_INSET;
    let chest_height = 6.5 - eye(&sim).y;
    let at_chest = View {
        yaw: start.yaw,
        pitch: (chest_height / hold_distance).asin(),
    };
    let standing = position(&sim).y;
    hold(
        &mut sim,
        PlayerInput {
            pitch: at_chest.pitch,
            ..walking(at_chest.yaw)
        },
    );
    tick(&mut sim, 180);
    let mut players = Vec::new();
    let mut crates = Vec::new();
    for _ in 0..120 {
        tick(&mut sim, 1);
        players.push(position(&sim));
        crates.push(crate_state(&sim).translation);
    }
    let player = *players.last().expect("samples");
    assert!(sim.carried().is_some(), "still held");
    assert_eq!(sim.take_release(), None);
    let short = face - player.x;
    assert!(
        (short - super::player::RADIUS).abs() < 0.1,
        "the player's capsule ends at the wall: centre {short} from the face"
    );
    assert!(
        players.iter().all(|at| (at.y - standing).abs() < 0.1),
        "the squeezed crate never lifts the player"
    );
    let rattle = spread(&crates);
    assert!(
        rattle.x < 0.02 && rattle.max_element() < 0.1,
        "the crate rests against the wall: {rattle}"
    );
}
