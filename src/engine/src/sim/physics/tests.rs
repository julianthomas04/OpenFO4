#![allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
//! Deterministic whole-pipeline scenarios on the real physics context: fixed
//! 1/60 s ticks, real rapier, real shape bake, synthetic content.

use glam::{Quat, Vec3};
use serde_json::json;

use super::test_support::{
    GROUND_CELL, NEIGHBOUR_CELL, bake, box_shape, box_shape_posed, ground_batch, object_group,
    part_placement, part_placement_posed, placement, run_ticks, terrain_mut, wake_context,
};
use super::*;
use crate::footsteps::events::SurfaceMaterial;
use crate::sim::collision::{CellColliderBatch, PartLocal, PartShape, TerrainCollision};

const CRATE_PATH: &str = "meshes\\crate.glb";
const CRATE_REF: &str = "0A0001:Fallout4.esm";
const CRATE_HALF_EXTENT: f32 = 0.5;
const CRATE_MASS: f32 = 2.0;
const DROP_HEIGHT: f32 = 6.0;
/// Loose bound for a resting box on terrain: rapier's allowed penetration is 0.001 u/m * 7 u/m.
const REST_HEIGHT_TOLERANCE: f32 = 0.05;
const TEN_SECONDS: u32 = 600;

fn crate_at(height: f32) -> (CellColliderBatch, BakedCellColliders) {
    let mut batch = ground_batch();
    batch.objects.push(object_group(
        CRATE_PATH,
        vec![placement(CRATE_REF, Vec3::new(100.0, height, -100.0))],
        true,
        Some(CRATE_MASS),
    ));
    let baked = bake(&batch, &[(CRATE_PATH, vec![box_shape(CRATE_HALF_EXTENT)])]);
    (batch, baked)
}

fn sleeping_context() -> PhysicsContext {
    let mut context = PhysicsContext::new();
    context.activation = ActivationMode::Sleep;
    context
}

fn context_with_crate(mut context: PhysicsContext, height: f32) -> PhysicsContext {
    let (batch, baked) = crate_at(height);
    context.add_cell(GROUND_CELL, &batch, Some(&baked));
    context.set_physics_center(GROUND_CELL, 2);
    context
}

fn the_crate(context: &PhysicsContext) -> &RigidBody {
    context
        .world
        .bodies
        .iter()
        .map(|(_, body)| body)
        .find(|body| body.is_dynamic() || body.is_fixed())
        .expect("the crate body")
}

fn crate_handle(context: &PhysicsContext) -> RigidBodyHandle {
    context
        .world
        .bodies
        .iter()
        .map(|(handle, _)| handle)
        .next()
        .expect("the crate body")
}

#[test]
fn a_dropped_crate_comes_to_rest_on_the_ground_and_falls_asleep() {
    let mut context = context_with_crate(wake_context(), DROP_HEIGHT);
    run_ticks(&mut context, TEN_SECONDS);
    let body = the_crate(&context);
    assert!(body.is_sleeping(), "settled clutter must sleep");
    let rest = body.translation();
    assert!(
        (rest.y - CRATE_HALF_EXTENT).abs() < REST_HEIGHT_TOLERANCE,
        "rests on its underside at {} u",
        rest.y
    );
    assert!(
        (rest.x - 100.0).abs() < 0.1 && (rest.z + 100.0).abs() < 0.1,
        "fell straight down"
    );
}

#[test]
fn a_settled_world_skips_the_pipeline_and_publishes_no_poses() {
    let mut context = context_with_crate(wake_context(), DROP_HEIGHT);
    run_ticks(&mut context, TEN_SECONDS);
    assert!(!context.needs_step());
    let mut poses = Vec::new();
    context.dynamic_poses(1.0, &mut poses);
    assert!(poses.is_empty());
}

#[test]
fn a_falling_crate_publishes_interpolated_poses() {
    let mut context = context_with_crate(wake_context(), DROP_HEIGHT);
    run_ticks(&mut context, 10);
    let mut poses = Vec::new();
    context.dynamic_poses(1.0, &mut poses);
    assert_eq!(poses.len(), 1);
    assert_eq!(poses[0].entity, refr_entity_id(CRATE_REF));
    assert!(poses[0].translation.y < DROP_HEIGHT);
}

#[test]
fn a_crate_falls_under_the_documented_dynamic_gravity() {
    // Free fall for 0.25 s from rest: d = g t^2 / 2 less damping, which only slows it (0.2/s).
    let mut context = context_with_crate(wake_context(), 40.0);
    run_ticks(&mut context, 15);
    let fallen = 40.0 - the_crate(&context).translation().y;
    let ideal = 0.5 * GRAVITY_DYN * 0.25 * 0.25;
    assert!(
        fallen < ideal && fallen > ideal * 0.9,
        "fell {fallen} u, undamped {ideal} u"
    );
}

#[test]
fn the_back_solved_mass_equals_the_manifest_mass() {
    let context = context_with_crate(wake_context(), DROP_HEIGHT);
    assert!((the_crate(&context).mass() - CRATE_MASS).abs() < 1e-3);
}

#[test]
fn clutter_activates_asleep_in_sleep_mode_and_hangs_until_disturbed() {
    let mut context = context_with_crate(sleeping_context(), DROP_HEIGHT);
    run_ticks(&mut context, 60);
    assert!(the_crate(&context).is_sleeping());
    assert_eq!(the_crate(&context).translation().y, DROP_HEIGHT);
    assert!(!context.needs_step());
}

#[test]
fn a_shove_wakes_sleeping_clutter() {
    let mut context = context_with_crate(sleeping_context(), DROP_HEIGHT);
    let handle = crate_handle(&context);
    context.shove_body(handle, Vec3::new(1.0, 0.0, 0.0), None);
    run_ticks(&mut context, 30);
    assert!(
        the_crate(&context).translation().y < DROP_HEIGHT,
        "it started to fall"
    );
}

#[test]
fn a_shove_cannot_exceed_the_push_speed_cap() {
    let mut context = context_with_crate(wake_context(), DROP_HEIGHT);
    run_ticks(&mut context, 1);
    let handle = crate_handle(&context);
    context.shove_body(handle, Vec3::new(1.0e9, 0.0, 0.0), None);
    let speed = context.world.bodies[handle].linvel().length();
    assert!((speed - PUSH_MAX_LINVEL).abs() < 1e-2, "speed {speed}");
}

#[test]
fn a_shove_below_the_cap_is_applied_in_full() {
    let mut context = context_with_crate(wake_context(), DROP_HEIGHT);
    run_ticks(&mut context, 1);
    let handle = crate_handle(&context);
    let before = context.world.bodies[handle].linvel();
    context.shove_body(handle, Vec3::new(CRATE_MASS * 10.0, 0.0, 0.0), None);
    let velocity = context.world.bodies[handle].linvel() - before;
    assert!(
        velocity.abs_diff_eq(Vec3::new(10.0, 0.0, 0.0), 1e-3),
        "{velocity}"
    );
}

#[test]
fn a_frozen_body_outside_the_radius_ignores_shoves() {
    let mut context = context_with_crate(wake_context(), DROP_HEIGHT);
    context.set_physics_center((50, 50), 1);
    let handle = crate_handle(&context);
    context.shove_body(handle, Vec3::new(1000.0, 0.0, 0.0), None);
    assert_eq!(context.world.bodies[handle].linvel(), Vec3::ZERO);
    assert_eq!(context.active_dynamic_count(), 0);
    assert_eq!(context.dynamic_body_count(), 1);
}

#[test]
fn leaving_the_radius_freezes_the_body_in_place_and_returning_reactivates_it() {
    let mut context = context_with_crate(wake_context(), DROP_HEIGHT);
    run_ticks(&mut context, 10);
    context.set_physics_center((50, 50), 1);
    let frozen_at = the_crate(&context).translation();
    run_ticks(&mut context, 30);
    assert_eq!(the_crate(&context).translation(), frozen_at);
    context.set_physics_center(GROUND_CELL, 2);
    run_ticks(&mut context, 30);
    assert!(the_crate(&context).translation().y < frozen_at.y);
}

#[test]
fn removing_the_cell_despawns_the_crate_and_the_ground() {
    let mut context = context_with_crate(wake_context(), DROP_HEIGHT);
    context.remove_cell(GROUND_CELL);
    assert_eq!(context.dynamic_body_count(), 0);
    assert_eq!(context.world.bodies.len(), 0);
    assert_eq!(context.collider_count(), 0);
    assert!(!context.spawn_ready(GROUND_CELL));
}

#[test]
fn a_dynamic_placement_without_a_form_key_stays_a_static_collider() {
    let mut batch = ground_batch();
    let mut keyless = placement(CRATE_REF, Vec3::new(100.0, 1.0, -100.0));
    keyless.ref_key = None;
    batch.objects.push(object_group(
        CRATE_PATH,
        vec![keyless],
        true,
        Some(CRATE_MASS),
    ));
    let baked = bake(&batch, &[(CRATE_PATH, vec![box_shape(CRATE_HALF_EXTENT)])]);
    let mut context = wake_context();
    context.add_cell(GROUND_CELL, &batch, Some(&baked));
    assert_eq!(context.dynamic_body_count(), 0);
    assert_eq!(context.collider_count(), 2, "terrain plus the static crate");
}

fn assert_non_finite_placement_is_skipped_and_stepping_survives(dynamic: bool) {
    let mut batch = ground_batch();
    let mut broken = placement(CRATE_REF, Vec3::new(100.0, 1.0, -100.0));
    broken.rotation = Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0);
    batch.objects.push(object_group(
        CRATE_PATH,
        vec![broken],
        dynamic,
        Some(CRATE_MASS),
    ));
    let baked = bake(&batch, &[(CRATE_PATH, vec![box_shape(CRATE_HALF_EXTENT)])]);
    let mut context = wake_context();
    context.add_cell(GROUND_CELL, &batch, Some(&baked));
    assert_eq!(context.dynamic_body_count(), 0);
    assert_eq!(context.collider_count(), 1, "terrain only");
    for _ in 0..3 {
        context.step();
    }
}

/// Regression: a collider with a NaN pose (a degenerate REFR scale decomposes to a NaN rotation)
/// has a NaN broad-phase centre, which made parry's binned BVH build panic out of bounds.
#[test]
fn a_dynamic_placement_with_a_non_finite_pose_is_skipped() {
    assert_non_finite_placement_is_skipped_and_stepping_survives(true);
}

#[test]
fn a_static_placement_with_a_non_finite_pose_is_skipped() {
    assert_non_finite_placement_is_skipped_and_stepping_survives(false);
}

#[test]
fn a_second_placement_with_the_same_form_key_does_not_spawn_a_second_body() {
    let mut batch = ground_batch();
    let twice = vec![
        placement(CRATE_REF, Vec3::new(100.0, 1.0, -100.0)),
        placement(CRATE_REF, Vec3::new(110.0, 1.0, -100.0)),
    ];
    batch
        .objects
        .push(object_group(CRATE_PATH, twice, true, Some(CRATE_MASS)));
    let baked = bake(&batch, &[(CRATE_PATH, vec![box_shape(CRATE_HALF_EXTENT)])]);
    let mut context = wake_context();
    context.add_cell(GROUND_CELL, &batch, Some(&baked));
    assert_eq!(context.dynamic_body_count(), 1);
}

const WEAPON_REF: &str = "0B0001:Fallout4.esm";
const RECEIVER_PATH: &str = "meshes\\weapons\\receiver.glb";
const GRIP_PATH: &str = "meshes\\weapons\\grip.glb";
const RECEIVER_MASS: f32 = 2.0;
const GRIP_MASS: f32 = 3.0;
const GRIP_OFFSET: Vec3 = Vec3::new(4.0, 0.0, 0.0);

fn weapon_shapes() -> Vec<(&'static str, Vec<PartShape>)> {
    vec![
        (RECEIVER_PATH, vec![box_shape(CRATE_HALF_EXTENT)]),
        (GRIP_PATH, vec![box_shape(CRATE_HALF_EXTENT)]),
    ]
}

/// A weapon REFR whose receiver and grip are separate colliding meshes; the
/// grip group is listed first to show the order does not matter.
fn weapon_batch(scale: f32, grip_dynamic: bool) -> CellColliderBatch {
    let at = Vec3::new(100.0, 1.0, -100.0);
    let scaled = |placement: Placement| Placement { scale, ..placement };
    let mut batch = ground_batch();
    batch.objects.push(object_group(
        GRIP_PATH,
        vec![scaled(part_placement(WEAPON_REF, at, GRIP_OFFSET))],
        grip_dynamic,
        Some(GRIP_MASS),
    ));
    batch.objects.push(object_group(
        RECEIVER_PATH,
        vec![scaled(part_placement(WEAPON_REF, at, Vec3::ZERO))],
        true,
        Some(RECEIVER_MASS),
    ));
    batch
}

fn weapon_context(batch: &CellColliderBatch) -> PhysicsContext {
    let baked = bake(batch, &weapon_shapes());
    let mut context = wake_context();
    context.add_cell(GROUND_CELL, batch, Some(&baked));
    context
}

fn body_collider_offsets(context: &PhysicsContext) -> Vec<Vec3> {
    let (_, body) = context.world.bodies.iter().next().expect("the weapon body");
    let mut offsets: Vec<Vec3> = body
        .colliders()
        .iter()
        .map(|handle| {
            context.world.colliders[*handle]
                .position_wrt_parent()
                .unwrap()
                .translation
        })
        .collect();
    offsets.sort_by(|a, b| a.x.total_cmp(&b.x));
    offsets
}

#[test]
fn the_parts_of_a_weapon_form_one_body_with_the_summed_mass() {
    let context = weapon_context(&weapon_batch(1.0, true));
    assert_eq!(context.dynamic_body_count(), 1);
    let body = the_crate(&context);
    assert_eq!(body.colliders().len(), 2);
    assert!((body.mass() - (RECEIVER_MASS + GRIP_MASS)).abs() < 1e-3);
}

#[test]
fn part_colliders_sit_at_their_offset_scaled_by_the_refr() {
    let context = weapon_context(&weapon_batch(2.0, true));
    let offsets = body_collider_offsets(&context);
    assert_eq!(offsets.len(), 2);
    assert!(offsets[0].length() < 1e-5);
    assert!((offsets[1] - GRIP_OFFSET * 2.0).length() < 1e-5);
}

#[test]
fn a_part_without_its_own_dynamic_flag_still_joins_the_weapon_body() {
    let context = weapon_context(&weapon_batch(1.0, false));
    assert_eq!(context.dynamic_body_count(), 1);
    assert_eq!(the_crate(&context).colliders().len(), 2);
}

#[test]
fn a_plain_placement_sharing_a_weapon_form_key_is_still_a_duplicate() {
    let mut batch = weapon_batch(1.0, true);
    batch.objects.push(object_group(
        CRATE_PATH,
        vec![placement(WEAPON_REF, Vec3::new(120.0, 1.0, -100.0))],
        true,
        Some(CRATE_MASS),
    ));
    let mut shapes = weapon_shapes();
    shapes.push((CRATE_PATH, vec![box_shape(CRATE_HALF_EXTENT)]));
    let baked = bake(&batch, &shapes);
    let mut context = wake_context();
    context.add_cell(GROUND_CELL, &batch, Some(&baked));
    assert_eq!(context.dynamic_body_count(), 1);
    assert_eq!(the_crate(&context).colliders().len(), 2);
}

#[test]
fn a_weapon_without_any_dynamic_part_is_static_at_the_part_offsets() {
    let mut batch = weapon_batch(1.0, false);
    batch.objects[1].dynamic = false;
    let mut context = weapon_context(&batch);
    context.step();
    assert_eq!(context.dynamic_body_count(), 0);
    let hit = context.raycast_down(Vec3::new(100.0 + GRIP_OFFSET.x, 10.0, -100.0), 20.0);
    assert!(
        hit.is_some_and(|height| (height - 1.5).abs() < 1e-3),
        "{hit:?}"
    );
}

#[test]
fn a_plain_body_created_first_does_not_take_in_a_weapon_part_of_the_same_form_key() {
    let at = Vec3::new(100.0, 1.0, -100.0);
    let mut batch = ground_batch();
    batch.objects.push(object_group(
        CRATE_PATH,
        vec![placement(WEAPON_REF, at)],
        true,
        Some(CRATE_MASS),
    ));
    batch.objects.push(object_group(
        GRIP_PATH,
        vec![part_placement(WEAPON_REF, at, GRIP_OFFSET)],
        true,
        Some(GRIP_MASS),
    ));
    let mut shapes = weapon_shapes();
    shapes.push((CRATE_PATH, vec![box_shape(CRATE_HALF_EXTENT)]));
    let baked = bake(&batch, &shapes);
    let mut context = wake_context();
    context.add_cell(GROUND_CELL, &batch, Some(&baked));
    assert_eq!(context.dynamic_body_count(), 1);
    assert_eq!(the_crate(&context).colliders().len(), 1);
    assert!((the_crate(&context).mass() - CRATE_MASS).abs() < 1e-3);
}

fn quarter_turn_about_y() -> Quat {
    Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
}

fn assert_same_rotation(actual: Quat, expected: Quat) {
    assert!(
        actual.dot(expected).abs() > 1.0 - 1e-5,
        "{actual:?} != {expected:?}"
    );
}

fn posed_part_batch(
    grip_dynamic: bool,
) -> (CellColliderBatch, Vec<(&'static str, Vec<PartShape>)>) {
    let at = Vec3::new(100.0, 1.0, -100.0);
    let part = PartLocal {
        translation: GRIP_OFFSET,
        rotation: quarter_turn_about_y(),
    };
    let mut batch = ground_batch();
    batch.objects.push(object_group(
        GRIP_PATH,
        vec![part_placement_posed(WEAPON_REF, at, part)],
        grip_dynamic,
        Some(GRIP_MASS),
    ));
    let shapes = vec![(
        GRIP_PATH,
        vec![box_shape_posed(
            CRATE_HALF_EXTENT,
            Vec3::new(0.5, 0.0, 0.0),
            Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
        )],
    )];
    (batch, shapes)
}

fn expected_collider_pose() -> (Vec3, Quat) {
    let part_rotation = quarter_turn_about_y();
    (
        GRIP_OFFSET + part_rotation * Vec3::new(0.5, 0.0, 0.0),
        part_rotation * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
    )
}

#[test]
fn a_dynamic_part_collider_pose_is_the_part_pose_times_the_shape_pose() {
    let (batch, shapes) = posed_part_batch(true);
    let baked = bake(&batch, &shapes);
    let mut context = wake_context();
    context.add_cell(GROUND_CELL, &batch, Some(&baked));
    let (_, body) = context.world.bodies.iter().next().expect("the weapon body");
    let handle = body.colliders()[0];
    let pose = context.world.colliders[handle]
        .position_wrt_parent()
        .unwrap();
    let (translation, rotation) = expected_collider_pose();
    assert!((pose.translation - translation).length() < 1e-4, "{pose:?}");
    assert_same_rotation(pose.rotation, rotation);
}

#[test]
fn a_static_part_collider_pose_is_refr_times_part_times_the_shape_pose() {
    let (batch, shapes) = posed_part_batch(false);
    let baked = bake(&batch, &shapes);
    let mut context = wake_context();
    context.add_cell(GROUND_CELL, &batch, Some(&baked));
    let world_pose = context
        .world
        .colliders
        .iter()
        .filter(|(_, collider)| collider.parent().is_none())
        .map(|(_, collider)| *collider.position())
        .find(|pose| (pose.translation.x - 100.0).abs() < 50.0 && pose.translation.y > 0.5)
        .expect("the static part collider");
    let (translation, rotation) = expected_collider_pose();
    let expected = Vec3::new(100.0, 1.0, -100.0) + translation;
    assert!(
        (world_pose.translation - expected).length() < 1e-4,
        "{world_pose:?}"
    );
    assert_same_rotation(world_pose.rotation, rotation);
}

#[test]
fn a_compound_body_created_by_a_static_part_takes_the_water_of_the_dynamic_part() {
    let at = Vec3::new(100.0, 1.0, -100.0);
    let water = Arc::new(WaterBodyProps {
        layer: 7,
        part: 0,
        materials: vec![(1, 1)],
    });
    let mut batch = ground_batch();
    let mut receiver = object_group(
        RECEIVER_PATH,
        vec![part_placement(WEAPON_REF, at, Vec3::ZERO)],
        true,
        Some(RECEIVER_MASS),
    );
    receiver.water = Some(Arc::clone(&water));
    batch.objects.push(receiver);
    batch.objects.push(object_group(
        GRIP_PATH,
        vec![part_placement(WEAPON_REF, at, GRIP_OFFSET)],
        false,
        None,
    ));
    let baked = bake(&batch, &[(GRIP_PATH, vec![box_shape(CRATE_HALF_EXTENT)])]);
    let mut context = wake_context();
    context.add_cell(GROUND_CELL, &batch, Some(&baked));
    assert_eq!(context.dynamic_body_count(), 1);
    let body = context.dyn_bodies.get(&refr_entity_id(WEAPON_REF)).unwrap();
    assert_eq!(body.water_props.as_deref(), Some(&*water));
}

#[test]
fn a_joint_part_of_a_static_weapon_part_rides_a_body_at_the_part_pose() {
    let at = Vec3::new(100.0, 1.0, -100.0);
    let mut batch = ground_batch();
    batch.objects.push(object_group(
        GRIP_PATH,
        vec![part_placement(WEAPON_REF, at, GRIP_OFFSET)],
        false,
        None,
    ));
    let mut hinged = box_shape(CRATE_HALF_EXTENT);
    hinged.joint = Some(0);
    let baked = bake(&batch, &[(GRIP_PATH, vec![hinged])]);
    let mut context = wake_context();
    context.add_cell(GROUND_CELL, &batch, Some(&baked));
    let body = context
        .world
        .bodies
        .iter()
        .map(|(_, body)| body)
        .find(|body| body.is_kinematic())
        .expect("the joint body");
    assert!((body.translation() - (at + GRIP_OFFSET)).length() < 1e-4);
}

#[test]
fn a_static_world_needs_no_steps_once_its_colliders_are_folded() {
    let mut context = wake_context();
    context.add_cell(GROUND_CELL, &ground_batch(), None);
    assert!(
        context.needs_step(),
        "new colliders must reach the broad phase"
    );
    context.step();
    assert!(!context.needs_step());
}

#[test]
fn a_cell_whose_terrain_indexes_past_its_vertices_gets_no_ground_and_does_not_panic() {
    let mut batch = ground_batch();
    terrain_mut(&mut batch).indices[1] = [0, 2, 17];
    let mut context = wake_context();
    context.add_cell(GROUND_CELL, &batch, None);
    assert!(!context.spawn_ready(GROUND_CELL));
}

#[test]
fn spawn_readiness_follows_the_installed_ground_of_the_cell() {
    let mut context = wake_context();
    assert!(!context.spawn_ready(GROUND_CELL));
    context.add_cell(GROUND_CELL, &ground_batch(), None);
    assert!(context.spawn_ready(GROUND_CELL));
    assert!(!context.spawn_ready(NEIGHBOUR_CELL));
    context.set_interior_mode(true);
    assert!(
        context.spawn_ready((99, 99)),
        "interior mode only asks for an installed cell"
    );
}

#[test]
fn a_cell_without_terrain_is_ready_once_installed() {
    let mut context = wake_context();
    let landless = CellColliderBatch {
        terrain: TerrainCollision::Absent,
        objects: Vec::new(),
    };
    context.add_cell(GROUND_CELL, &landless, None);
    assert!(context.spawn_ready(GROUND_CELL));
    assert!(!context.spawn_ready(NEIGHBOUR_CELL));
}

#[test]
fn a_cell_whose_terrain_failed_to_build_is_never_ready() {
    let mut context = wake_context();
    let failed = CellColliderBatch {
        terrain: TerrainCollision::Failed,
        objects: Vec::new(),
    };
    context.add_cell(GROUND_CELL, &failed, None);
    assert!(!context.spawn_ready(GROUND_CELL));
}

#[test]
fn removing_a_cell_withdraws_its_readiness() {
    let mut context = wake_context();
    context.add_cell(GROUND_CELL, &CellColliderBatch::default(), None);
    context.remove_cell(GROUND_CELL);
    assert!(!context.spawn_ready(GROUND_CELL));
}

#[test]
fn interior_readiness_needs_an_installed_cell_but_ignores_the_grid() {
    let mut context = wake_context();
    context.set_interior_mode(true);
    assert!(!context.spawn_ready((99, 99)));
    context.add_cell(GROUND_CELL, &CellColliderBatch::default(), None);
    assert!(context.spawn_ready((99, 99)));
}

#[test]
fn a_ray_straight_down_finds_the_ground_height() {
    let mut context = wake_context();
    context.add_cell(GROUND_CELL, &ground_batch(), None);
    context.step();
    let ground_y = context
        .raycast_down(Vec3::new(100.0, 25.0, -100.0), 1000.0)
        .unwrap();
    assert!(
        ground_y.abs() < 1e-3,
        "the ground is at height 0, got {ground_y}"
    );
    assert!(
        context
            .raycast_down(Vec3::new(100.0, 25.0, 5000.0), 1000.0)
            .is_none()
    );
}

#[test]
fn rays_report_the_mesh_and_form_key_of_the_collider_they_hit() {
    let mut context = context_with_crate(wake_context(), 1.0);
    context.step();
    let hit = context
        .ray_attribution(Vec3::new(100.0, 10.0, -100.0), Vec3::NEG_Y, 100.0)
        .unwrap();
    assert_eq!(hit.mesh_path, CRATE_PATH);
    assert_eq!(hit.ref_id.as_deref(), Some(CRATE_REF));
    assert!((hit.dist - 8.5).abs() < 0.05, "dist {}", hit.dist);
    let terrain = context
        .ray_attribution(Vec3::new(300.0, 10.0, -300.0), Vec3::NEG_Y, 100.0)
        .unwrap();
    assert_eq!(terrain.mesh_path, "terrain");
    assert!(terrain.ref_id.is_none());
}

mod ragdoll {
    use std::path::Path;

    use super::*;
    use crate::sim::EntityId;
    use crate::sim::ragdoll::RagdollPoses;

    const SKELETON_PATH: &str = "meshes\\skeleton.glb";
    const SKELETON_REF: &str = "0B0001:Fallout4.esm";
    const SKELETON_AT: Vec3 = Vec3::new(100.0, 0.0, -500.0);
    const BONE_COUNT: usize = 3;
    const BONE_RADIUS: f32 = 0.5;
    const TWIST_LIMIT: f32 = 0.5;
    const PLANE_LIMIT: f32 = 0.5;
    const CONE_LIMIT: f32 = 0.8;
    /// The joints are soft springs (30 Hz): measured overshoot is under 0.05 rad.
    const LIMIT_SLACK: f32 = 0.1;
    /// Anchors of two bones joined at a pivot drift apart through that softness (measured peak 0.094 u).
    const ANCHOR_SLACK: f32 = 0.15;

    fn frame(x: f32, y: f32, z: f32) -> serde_json::Value {
        json!({ "t": [x, y, z], "r": [0.0, 0.0, 0.0, 1.0] })
    }

    fn bone(
        node: &str,
        y: f32,
        node_parent: Option<usize>,
        water: Option<&serde_json::Value>,
    ) -> serde_json::Value {
        let mut bone = json!({
            "node": node, "mass": 2.0, "anchor": false, "damping": [0.1, 0.1],
            "rest": frame(0.0, y, 0.0),
            "shapes": [{ "type": "sphere", "radius": BONE_RADIUS }],
            "node_parent": node_parent,
        });
        if let Some(water) = water {
            bone["water"] = water.clone();
        }
        bone
    }

    fn joint(child: usize, parent: usize) -> serde_json::Value {
        json!({
            "type": "ragdoll", "child": child, "parent": parent,
            "frame_child": frame(0.0, -0.5, 0.0), "frame_parent": frame(0.0, 0.5, 0.0),
            "twist": [-TWIST_LIMIT, TWIST_LIMIT], "plane": [-PLANE_LIMIT, PLANE_LIMIT],
            "cone": CONE_LIMIT, "friction": 0.0,
        })
    }

    fn write_chain_sidecar(dir: &Path, water: Option<&serde_json::Value>) -> String {
        let sidecar = json!({
            "version": 2,
            "bodies": [
                bone("Pelvis", 3.0, None, water),
                bone("Spine", 4.0, Some(0), water),
                bone("Head", 5.0, Some(1), water),
            ],
            "constraints": [joint(1, 0), joint(2, 1)],
        });
        let path = dir.join("skeleton.col.json");
        std::fs::write(&path, serde_json::to_vec(&sidecar).unwrap()).unwrap();
        path.to_str().unwrap().to_owned()
    }

    struct Scene {
        context: PhysicsContext,
        batch: CellColliderBatch,
        baked: BakedCellColliders,
        entity: EntityId,
        _dir: tempfile::TempDir,
    }

    impl Scene {
        fn new(with_ground: bool) -> Self {
            Self::with_bone_water(with_ground, None)
        }

        /// Every bone carries `water` as its sidecar `water` object (none when `None`).
        fn with_bone_water(with_ground: bool, water: Option<&serde_json::Value>) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let sidecar = write_chain_sidecar(dir.path(), water);
            let mut batch = CellColliderBatch::default();
            let mut group = object_group(
                SKELETON_PATH,
                vec![placement(SKELETON_REF, SKELETON_AT)],
                true,
                Some(6.0),
            );
            group.ragdoll_sidecar = Some(sidecar);
            batch.objects.push(group);
            let baked = bake(&batch, &[(SKELETON_PATH, vec![box_shape(1.0)])]);
            let mut context = wake_context();
            if with_ground {
                context.add_cell(GROUND_CELL, &ground_batch(), None);
            }
            context.add_cell(NEIGHBOUR_CELL, &batch, Some(&baked));
            context.set_physics_center(NEIGHBOUR_CELL, 1);
            Self {
                context,
                batch,
                baked,
                entity: refr_entity_id(SKELETON_REF),
                _dir: dir,
            }
        }

        fn folded(with_ground: bool) -> Self {
            let mut scene = Self::new(with_ground);
            run_ticks(&mut scene.context, 10);
            scene
        }

        fn bones(&self) -> Vec<&RigidBody> {
            self.context
                .world
                .bodies
                .iter()
                .map(|(_, body)| body)
                .filter(|body| body.user_data == u128::from(self.entity.0))
                .collect()
        }

        fn positions(&self) -> Vec<Vec3> {
            self.bones().iter().map(|body| body.translation()).collect()
        }

        fn is_asleep(&self) -> bool {
            self.bones().iter().all(|body| body.is_sleeping())
        }
    }

    /// Respawned bodies reuse freed arena slots, so their iteration order is not spec order.
    fn sorted_by_height(mut positions: Vec<Vec3>) -> Vec<Vec3> {
        positions.sort_by(|a, b| a.y.total_cmp(&b.y));
        positions
    }

    fn twist_angle(relative: Quat) -> f32 {
        2.0 * relative.x.atan2(relative.w)
    }

    fn assert_within_joint_limits(parent: &RigidBody, child: &RigidBody, tick: u32) {
        let relative = parent.rotation().inverse() * child.rotation();
        let twist = twist_angle(relative);
        let axis = relative * Vec3::X;
        let tilt_toward_y = axis.y.atan2(axis.x);
        let tilt_toward_z = axis.z.atan2(axis.x);
        assert!(
            twist.abs() <= TWIST_LIMIT + LIMIT_SLACK,
            "tick {tick}: twist {twist}"
        );
        assert!(
            tilt_toward_y.abs() <= PLANE_LIMIT + LIMIT_SLACK,
            "tick {tick}: plane {tilt_toward_y}"
        );
        assert!(
            tilt_toward_z.abs() <= CONE_LIMIT + LIMIT_SLACK,
            "tick {tick}: cone {tilt_toward_z}"
        );
    }

    fn assert_joint_pivots_meet(parent: &RigidBody, child: &RigidBody, tick: u32) {
        let parent_pivot = parent.position() * Vec3::new(0.0, 0.5, 0.0);
        let child_pivot = child.position() * Vec3::new(0.0, -0.5, 0.0);
        let error = parent_pivot.distance(child_pivot);
        assert!(error <= ANCHOR_SLACK, "tick {tick}: pivots {error} u apart");
    }

    #[test]
    fn a_frozen_ragdoll_spawns_asleep_and_costs_no_steps() {
        let scene = Scene::folded(true);
        assert_eq!(scene.bones().len(), BONE_COUNT);
        assert!(scene.is_asleep());
        assert!(!scene.context.needs_step());
        let expected = [3.0, 4.0, 5.0];
        for (position, height) in scene.positions().iter().zip(expected) {
            assert!(
                position.abs_diff_eq(Vec3::new(100.0, height, -500.0), 1e-3),
                "{position}"
            );
        }
    }

    #[test]
    fn a_disturbed_ragdoll_collapses_stays_finite_and_within_joint_limits() {
        let mut scene = Scene::folded(true);
        scene.context.world.wake_up_all(true);
        let mut tick = 0;
        while scene.context.needs_step() && tick < 3 * TEN_SECONDS {
            scene.context.step();
            tick += 1;
            let bones = scene.bones();
            for body in &bones {
                assert!(
                    body.translation().is_finite() && body.linvel().is_finite(),
                    "tick {tick}"
                );
            }
            for (parent, child) in [(0, 1), (1, 2)] {
                assert_within_joint_limits(bones[parent], bones[child], tick);
                assert_joint_pivots_meet(bones[parent], bones[child], tick);
            }
        }
        assert!(scene.is_asleep(), "still simulating after {tick} ticks");
        for position in scene.positions() {
            assert!(
                position.y > BONE_RADIUS * 0.6 && position.y < 5.0,
                "bone at {position} did not settle on the ground"
            );
        }
    }

    #[test]
    fn a_ragdoll_only_sleeps_after_staying_inside_the_envelope_for_the_whole_window() {
        let settings = RagdollSettings::default();
        let window_ticks = (settings.window_seconds / FIXED_DT) as usize;
        let mut scene = Scene::folded(true);
        scene.context.world.wake_up_all(true);
        let mut history: Vec<Vec<Vec3>> = Vec::new();
        while scene.context.needs_step() && history.len() < 3 * TEN_SECONDS as usize {
            scene.context.step();
            history.push(scene.positions());
        }
        assert!(scene.is_asleep());
        assert!(
            history.len() > window_ticks,
            "it cannot sleep before one window has passed"
        );
        let last = &history[history.len() - window_ticks..];
        for bone in 0..BONE_COUNT {
            let travelled = last
                .iter()
                .map(|positions| positions[bone])
                .fold((Vec3::MAX, Vec3::MIN), |(low, high), p| {
                    (low.min(p), high.max(p))
                });
            let range = (travelled.1 - travelled.0).length();
            assert!(
                range <= settings.envelope + 0.01,
                "bone {bone} moved {range} u inside the last {} s before sleeping",
                settings.window_seconds
            );
        }
    }

    #[test]
    fn a_ragdoll_falling_without_ground_reverts_to_static_collision_after_eight_seconds() {
        let mut scene = Scene::folded(false);
        scene.context.world.wake_up_all(true);
        // 8 s of contact-free awake time is 16 half-second checks; allow a few ticks of slack for the check cadence.
        run_ticks(&mut scene.context, 7 * 60 + 30);
        assert_eq!(scene.bones().len(), BONE_COUNT, "still simulating at 7.5 s");
        run_ticks(&mut scene.context, 90);
        assert!(scene.bones().is_empty(), "reverted by 9 s");
        assert_eq!(
            scene.context.collider_count(),
            1,
            "the legacy collider is back"
        );
        let mut poses = RagdollPoses::default();
        scene.context.ragdoll_poses(&mut poses);
        assert_eq!(poses.released, [scene.entity]);
    }

    #[test]
    fn a_legacy_collider_reinserted_by_the_ragdoll_revert_is_audited_before_the_next_step() {
        let mut scene = Scene::folded(false);
        scene.context.world.wake_up_all(true);
        scene
            .context
            .ragdolls
            .set_refr_translation(scene.entity, Vec3::splat(f32::NAN));
        let mut ticks = 0;
        while !scene.bones().is_empty() && ticks < 9 * 60 {
            scene.context.step();
            ticks += 1;
        }
        assert!(scene.bones().is_empty(), "reverted by 9 s");
        assert_eq!(
            scene.context.collider_count(),
            1,
            "the legacy collider is back"
        );
        assert!(scene.context.dirty, "the revert flags the set as changed");
        scene.context.step();
        assert_eq!(
            scene.context.collider_count(),
            0,
            "the unsound legacy collider was quarantined"
        );
    }

    #[test]
    fn a_ragdoll_resting_on_the_ground_is_never_reverted() {
        let mut scene = Scene::folded(true);
        scene.context.world.wake_up_all(true);
        run_ticks(&mut scene.context, 20 * 60);
        assert_eq!(scene.bones().len(), BONE_COUNT);
    }

    #[test]
    fn a_moved_ragdoll_respawns_where_it_lay_when_its_cell_returns() {
        let mut scene = Scene::folded(true);
        scene.context.world.wake_up_all(true);
        run_ticks(&mut scene.context, 3 * TEN_SECONDS);
        let settled = sorted_by_height(scene.positions());
        scene.context.remove_cell(NEIGHBOUR_CELL);
        assert!(scene.bones().is_empty());
        scene
            .context
            .add_cell(NEIGHBOUR_CELL, &scene.batch, Some(&scene.baked));
        let mut poses = RagdollPoses::default();
        scene.context.ragdoll_poses(&mut poses);
        run_ticks(&mut scene.context, 10);
        assert_eq!(
            poses.entries.len(),
            1,
            "the renderer is told to skin it again"
        );
        assert_eq!(poses.entries[0].entity, scene.entity);
        for (restored, before) in sorted_by_height(scene.positions()).iter().zip(&settled) {
            assert!(
                restored.abs_diff_eq(*before, 1e-4),
                "{restored} vs {before}"
            );
        }
        assert!(scene.is_asleep(), "it was asleep when its cell left");
    }

    #[test]
    fn a_ragdoll_that_never_moved_leaves_no_remembered_pose() {
        let mut scene = Scene::folded(true);
        scene.context.remove_cell(NEIGHBOUR_CELL);
        scene
            .context
            .add_cell(NEIGHBOUR_CELL, &scene.batch, Some(&scene.baked));
        assert!(!scene.context.ragdoll_has_pose(scene.entity));
    }

    #[test]
    fn a_ragdoll_whose_sidecar_did_not_bake_stays_static() {
        let scene = Scene::new(true);
        let mut without_ragdolls = scene.baked.clone();
        without_ragdolls.ragdolls.clear();
        let mut context = wake_context();
        context.add_cell(NEIGHBOUR_CELL, &scene.batch, Some(&without_ragdolls));
        context.set_physics_center(NEIGHBOUR_CELL, 1);
        run_ticks(&mut context, 10);
        assert_eq!(context.world.bodies.len(), 0);
        assert_eq!(context.collider_count(), 1);
    }

    const BONE_MATERIAL: u32 = 0xB0_0E;
    const WATER_HEIGHT: f32 = 100.0;

    fn bone_water(layer: u8, part: u8) -> serde_json::Value {
        json!({ "layer": layer, "part": part, "materials": [[BONE_MATERIAL, 1]] })
    }

    fn water_scene(water: Option<&serde_json::Value>) -> Scene {
        let mut scene = Scene::with_bone_water(true, water);
        scene
            .context
            .set_material_buoyancy_table(Arc::new(MaterialBuoyancy::new([(BONE_MATERIAL, 1.0)])));
        scene
            .context
            .set_water_query(Some(crate::sim::water_physics::infinite_plane(
                WATER_HEIGHT,
            )));
        scene
    }

    fn run_in_water(scene: &mut Scene, ticks: u32) {
        for _ in 0..ticks {
            scene.context.update_water(SKELETON_AT);
            if scene.context.needs_step() {
                scene.context.step();
            }
        }
    }

    fn gravity_scales(scene: &Scene) -> Vec<f32> {
        scene
            .bones()
            .iter()
            .map(|body| body.gravity_scale())
            .collect()
    }

    #[test]
    fn a_folding_ragdoll_keeps_the_fold_s_gravity_scale_and_joins_the_water_once_folded() {
        let mut scene = water_scene(Some(&bone_water(10, 0)));
        scene.context.update_water(SKELETON_AT);
        assert_eq!(
            gravity_scales(&scene),
            [0.0; BONE_COUNT],
            "the fold runs gravity-free; the water pass leaves it alone"
        );
        run_in_water(&mut scene, 10);
        assert_eq!(gravity_scales(&scene), [0.1; BONE_COUNT]);
    }

    #[test]
    fn a_ragdoll_in_water_floats_up_from_its_resting_height() {
        let mut scene = water_scene(Some(&bone_water(10, 0)));
        run_in_water(&mut scene, 120);
        let heights = scene.positions().iter().map(|p| p.y).collect::<Vec<_>>();
        assert!(
            heights.iter().all(|y| *y > 5.5),
            "buoyant bones rise above their spawn heights {heights:?}"
        );
    }

    #[test]
    fn ragdoll_bones_on_an_untracked_layer_are_ignored_by_the_water() {
        let mut scene = water_scene(Some(&bone_water(19, 0)));
        run_in_water(&mut scene, 10);
        assert_eq!(gravity_scales(&scene), [1.0; BONE_COUNT]);
    }

    #[test]
    fn ragdoll_bones_without_water_data_are_ignored_by_the_water() {
        let mut scene = water_scene(None);
        run_in_water(&mut scene, 10);
        assert_eq!(gravity_scales(&scene), [1.0; BONE_COUNT]);
    }

    #[test]
    fn leaving_the_water_restores_full_gravity_on_ragdoll_bones() {
        let mut scene = water_scene(Some(&bone_water(10, 0)));
        run_in_water(&mut scene, 10);
        scene.context.set_water_query(Some(Box::new(|_| None)));
        run_in_water(&mut scene, 1);
        assert_eq!(gravity_scales(&scene), [1.0; BONE_COUNT]);
    }

    #[test]
    fn disabling_the_model_reverts_live_ragdolls_to_static_colliders() {
        let mut scene = Scene::folded(true);
        scene.context.disable_ragdoll_model(SKELETON_PATH);
        assert!(scene.bones().is_empty());
        scene.context.step();
        scene.context.set_physics_center(NEIGHBOUR_CELL, 1);
        run_ticks(&mut scene.context, 10);
        assert!(
            scene.bones().is_empty(),
            "never a ragdoll again this session"
        );
    }
}

mod ground_surface {
    use super::*;

    const SLAB_PATH: &str = "meshes\\slab.glb";
    const SLAB_REF: &str = "0A0002:Fallout4.esm";
    const DIRT: u32 = 0xD127;
    const CONCRETE: u32 = 0xC0C3;

    fn material_box(centre_x: f32, material: u32) -> PartShape {
        PartShape {
            material,
            ..box_shape_posed(1.0, Vec3::new(centre_x, 0.0, 0.0), Quat::IDENTITY)
        }
    }

    fn context_with_slab(parts: Vec<PartShape>, dynamic: bool) -> PhysicsContext {
        let mut batch = ground_batch();
        batch.objects.push(object_group(
            SLAB_PATH,
            vec![placement(SLAB_REF, Vec3::new(100.0, 0.0, -100.0))],
            dynamic,
            dynamic.then_some(CRATE_MASS),
        ));
        let baked = bake(&batch, &[(SLAB_PATH, parts)]);
        let mut context = sleeping_context();
        context.add_cell(GROUND_CELL, &batch, Some(&baked));
        context.step();
        context
    }

    fn surface_at(context: &PhysicsContext, x: f32, z: f32) -> SurfaceMaterial {
        context.ground_surface(Vec3::new(x, 10.0, z), 100.0)
    }

    #[test]
    fn each_material_of_one_mesh_is_reported_where_the_ray_hits_it() {
        let context = context_with_slab(
            vec![
                material_box(-2.0, DIRT),
                material_box(2.0, CONCRETE),
                material_box(-2.0, DIRT),
            ],
            false,
        );
        assert_eq!(
            surface_at(&context, 98.0, -100.0),
            SurfaceMaterial::Hash(DIRT)
        );
        assert_eq!(
            surface_at(&context, 102.0, -100.0),
            SurfaceMaterial::Hash(CONCRETE)
        );
    }

    #[test]
    fn rays_still_name_the_mesh_whatever_its_material() {
        let context = context_with_slab(vec![material_box(0.0, CONCRETE)], false);
        let hit = context
            .ray_attribution(Vec3::new(100.0, 10.0, -100.0), Vec3::NEG_Y, 100.0)
            .unwrap();
        assert_eq!(hit.mesh_path, SLAB_PATH);
        assert_eq!(hit.ref_id.as_deref(), Some(SLAB_REF));
    }

    #[test]
    fn terrain_reports_the_hit_point() {
        let context = context_with_slab(vec![material_box(0.0, CONCRETE)], false);
        let SurfaceMaterial::Terrain { point } = surface_at(&context, 300.0, -300.0) else {
            panic!("expected terrain");
        };
        assert!((point[0] - 300.0).abs() < 1e-3 && (point[2] + 300.0).abs() < 1e-3);
        assert!(point[1].abs() < 1e-3, "ground is at 0, got {}", point[1]);
    }

    #[test]
    fn a_collider_without_material_and_a_miss_report_none() {
        let context = context_with_slab(vec![material_box(0.0, 0)], false);
        assert_eq!(surface_at(&context, 100.0, -100.0), SurfaceMaterial::None);
        assert_eq!(
            context.ground_surface(Vec3::new(100.0, 10.0, -100.0), 1.0),
            SurfaceMaterial::None,
            "ground out of reach"
        );
    }

    #[test]
    fn dynamic_bodies_report_the_material_their_colliders_carry() {
        let with_material = context_with_slab(vec![material_box(0.0, DIRT)], true);
        assert_eq!(
            surface_at(&with_material, 100.0, -100.0),
            SurfaceMaterial::Hash(DIRT)
        );
        let without = context_with_slab(vec![material_box(0.0, 0)], true);
        assert_eq!(surface_at(&without, 100.0, -100.0), SurfaceMaterial::None);
    }

    #[test]
    fn a_moving_platform_reports_the_material_of_its_shapes() {
        let mut platform = material_box(0.0, CONCRETE);
        platform.joint = Some(0);
        let context = context_with_slab(vec![platform], false);
        assert!(
            context
                .world
                .bodies
                .iter()
                .any(|(_, body)| body.is_kinematic()),
            "the shape rides a kinematic body"
        );
        assert_eq!(
            surface_at(&context, 100.0, -100.0),
            SurfaceMaterial::Hash(CONCRETE)
        );
    }

    fn collider_materials(context: &PhysicsContext, body: &RigidBody) -> Vec<u32> {
        let mut materials: Vec<u32> = body
            .colliders()
            .iter()
            .map(|handle| {
                let slot = unpack_path(context.world.colliders[*handle].user_data);
                context.paths.get(slot).expect("an attributed collider").1
            })
            .collect();
        materials.sort_unstable();
        materials
    }

    #[test]
    fn each_collider_of_a_dynamic_body_carries_the_slot_of_its_material() {
        let context = context_with_slab(
            vec![material_box(-2.0, DIRT), material_box(2.0, CONCRETE)],
            true,
        );
        let body = the_crate(&context);
        assert_eq!(
            collider_materials(&context, body),
            [DIRT.min(CONCRETE), DIRT.max(CONCRETE)]
        );
    }

    #[test]
    fn the_parts_of_a_compound_body_each_carry_the_slot_of_their_material() {
        let batch = weapon_batch(1.0, true);
        let baked = bake(
            &batch,
            &[
                (RECEIVER_PATH, vec![material_box(0.0, DIRT)]),
                (GRIP_PATH, vec![material_box(0.0, CONCRETE)]),
            ],
        );
        let mut context = wake_context();
        context.add_cell(GROUND_CELL, &batch, Some(&baked));
        assert_eq!(context.dynamic_body_count(), 1);
        assert_eq!(
            collider_materials(&context, the_crate(&context)),
            [DIRT.min(CONCRETE), DIRT.max(CONCRETE)]
        );
    }

    #[test]
    fn a_placement_of_a_ragdoll_capable_mesh_keeps_its_shapes_materials_before_it_wakes() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = dir.path().join("body.col.json");
        std::fs::write(
            &sidecar,
            serde_json::to_vec(&json!({
                "version": 2,
                "bodies": [{
                    "node": "Pelvis", "mass": 2.0, "anchor": false, "damping": [0.1, 0.1],
                    "rest": { "t": [0.0, 3.0, 0.0], "r": [0.0, 0.0, 0.0, 1.0] },
                    "shapes": [{ "type": "sphere", "radius": 0.5 }],
                    "node_parent": null,
                }],
                "constraints": [],
            }))
            .unwrap(),
        )
        .unwrap();
        let mut batch = ground_batch();
        let mut group = object_group(
            SLAB_PATH,
            vec![placement(SLAB_REF, Vec3::new(100.0, 0.0, -100.0))],
            true,
            Some(CRATE_MASS),
        );
        group.ragdoll_sidecar = Some(sidecar.to_str().unwrap().to_owned());
        batch.objects.push(group);
        let baked = bake(&batch, &[(SLAB_PATH, vec![material_box(0.0, CONCRETE)])]);
        let mut context = sleeping_context();
        context.add_cell(GROUND_CELL, &batch, Some(&baked));
        context.step();
        assert_eq!(
            surface_at(&context, 100.0, -100.0),
            SurfaceMaterial::Hash(CONCRETE)
        );
    }

    #[test]
    fn a_collider_that_is_neither_terrain_nor_attributed_reports_no_material_and_warns_once() {
        let log = crate::audio::test_support::SharedBuffer::default();
        let _guard = log.capture("openfo4=warn");
        let mut context = context_with_slab(vec![material_box(0.0, DIRT)], false);
        context.world.insert_collider(
            ColliderBuilder::cuboid(1.0, 1.0, 1.0)
                .translation(Vec3::new(500.0, 0.0, -500.0))
                .user_data(0),
            None,
        );
        context.step();
        for _ in 0..3 {
            assert_eq!(surface_at(&context, 500.0, -500.0), SurfaceMaterial::None);
        }
        let logged = log.drain();
        assert_eq!(logged.lines().count(), 1, "{logged}");
        assert!(
            logged.contains("neither terrain nor attributed"),
            "{logged}"
        );
    }

    #[test]
    fn a_collider_with_an_unknown_path_slot_reports_no_material_and_warns_once() {
        const UNKNOWN_SLOT: u32 = 4_000_017;
        let log = crate::audio::test_support::SharedBuffer::default();
        let _guard = log.capture("openfo4=warn");
        let mut context = context_with_slab(vec![material_box(0.0, DIRT)], false);
        context.world.insert_collider(
            ColliderBuilder::cuboid(1.0, 1.0, 1.0)
                .translation(Vec3::new(500.0, 0.0, -500.0))
                .user_data(pack_path(UNKNOWN_SLOT)),
            None,
        );
        context.step();
        for _ in 0..3 {
            assert_eq!(surface_at(&context, 500.0, -500.0), SurfaceMaterial::None);
        }
        let logged = log.drain();
        assert_eq!(logged.lines().count(), 1, "{logged}");
        assert!(logged.contains(&UNKNOWN_SLOT.to_string()), "{logged}");
    }

    #[test]
    fn only_the_terrain_collider_is_terrain_and_it_stops_being_so_with_its_cell() {
        let mut context = context_with_slab(vec![material_box(0.0, DIRT)], false);
        assert!(matches!(
            surface_at(&context, 300.0, -300.0),
            SurfaceMaterial::Terrain { .. }
        ));
        assert_eq!(context.terrain_colliders.len(), 1);
        context.remove_cell(GROUND_CELL);
        assert!(context.terrain_colliders.is_empty());
    }
}
