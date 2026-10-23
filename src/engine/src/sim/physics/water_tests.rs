#![allow(clippy::float_cmp, reason = "gravity scale is set to exact constants")]
//! FO4 water behaviour on a real rapier world: a crate in a water
//! plane over the test ground (ground top at y = 0, crate half extent 0.5).

use std::sync::Arc;

use glam::Vec3;

use super::test_support::{
    GROUND_CELL, bake, box_shape, ground_batch, object_group, placement, wake_context,
};
use super::*;
use crate::sim::water_physics::infinite_plane;

const CRATE_PATH: &str = "meshes\\crate.glb";
const CRATE_REF: &str = "0A0001:Fallout4.esm";
const CRATE_START: Vec3 = Vec3::new(100.0, 1.0, -100.0);
const WATER_HEIGHT: f32 = 10.0;
const FIVE_SECONDS: u32 = 300;

const SINKING: u32 = 0x51_4E;
const FLOATING: u32 = 0xF1_0A;
const UNLISTED: u32 = 0xDEAD;

fn crate_context(water_height: Option<f32>) -> PhysicsContext {
    crate_context_with(water_height, None)
}

/// A crate whose sidecar carries `water`, in a world whose MATT table lists a sinking (-1.0) and a
/// floating (+1.0) material.
fn crate_context_with(water_height: Option<f32>, water: Option<WaterBodyProps>) -> PhysicsContext {
    let mut batch = ground_batch();
    let mut group = object_group(
        CRATE_PATH,
        vec![placement(CRATE_REF, CRATE_START)],
        true,
        Some(2.0),
    );
    group.water = water.map(Arc::new);
    batch.objects.push(group);
    let baked = bake(&batch, &[(CRATE_PATH, vec![box_shape(0.5)])]);
    let mut context = wake_context();
    context.set_material_buoyancy_table(Arc::new(MaterialBuoyancy::new([
        (SINKING, -1.0),
        (FLOATING, 1.0),
    ])));
    context.add_cell(GROUND_CELL, &batch, Some(&baked));
    context.set_physics_center(GROUND_CELL, 2);
    context.set_water_query(water_height.map(infinite_plane));
    context
}

fn made_of(layer: u8, materials: &[(u32, u32)]) -> WaterBodyProps {
    WaterBodyProps {
        layer,
        part: 0,
        materials: materials.to_vec(),
    }
}

fn run(context: &mut PhysicsContext, ticks: u32) {
    for _ in 0..ticks {
        context.update_water(CRATE_START);
        if context.needs_step() {
            context.step();
        }
    }
}

fn crate_body(context: &PhysicsContext) -> &RigidBody {
    let handle = context.dyn_bodies[&refr_entity_id(CRATE_REF)].handle;
    &context.world.bodies[handle]
}

#[test]
fn a_neutral_crate_rises_from_the_bottom_and_settles_at_the_surface() {
    let mut context = crate_context(Some(WATER_HEIGHT));
    run(&mut context, FIVE_SECONDS);
    let body = crate_body(&context);
    assert!(
        (body.translation().y - WATER_HEIGHT).abs() < 0.5,
        "floats at the waterline, y = {}",
        body.translation().y
    );
    assert!(body.linvel().length() < 3.0, "settled: {}", body.linvel());
    assert_eq!(body.gravity_scale(), 0.1, "water gravity factor");
}

#[test]
fn a_negative_buoyancy_crate_sinks_to_the_ground() {
    let mut context = crate_context(Some(WATER_HEIGHT));
    context.set_material_buoyancy(refr_entity_id(CRATE_REF), -1.0);
    run(&mut context, FIVE_SECONDS);
    let body = crate_body(&context);
    assert!(body.translation().y < 1.0, "y = {}", body.translation().y);
}

#[test]
fn a_crate_of_a_sinking_material_sinks_to_the_ground() {
    let mut context = crate_context_with(Some(WATER_HEIGHT), Some(made_of(4, &[(SINKING, 1)])));
    run(&mut context, FIVE_SECONDS);
    assert!(crate_body(&context).translation().y < 1.0);
}

#[test]
fn a_crate_of_a_floating_material_rises_to_the_surface() {
    let mut context = crate_context_with(Some(WATER_HEIGHT), Some(made_of(4, &[(FLOATING, 1)])));
    run(&mut context, FIVE_SECONDS);
    assert!((crate_body(&context).translation().y - WATER_HEIGHT).abs() < 0.5);
}

#[test]
fn the_buoyancy_is_the_mean_over_the_shapes_materials() {
    // (-1 + 1) / 2 = 0: no lift at all, so the crate sinks like gravity-factor-0.1 ballast.
    let mut context = crate_context_with(
        Some(WATER_HEIGHT),
        Some(made_of(4, &[(SINKING, 1), (FLOATING, 1)])),
    );
    run(&mut context, FIVE_SECONDS);
    assert!(crate_body(&context).translation().y < 1.0);
}

#[test]
fn a_material_without_a_matt_record_contributes_no_buoyancy() {
    let mut context = crate_context_with(Some(WATER_HEIGHT), Some(made_of(4, &[(UNLISTED, 1)])));
    run(&mut context, FIVE_SECONDS);
    assert!(crate_body(&context).translation().y < 1.0);
}

#[test]
fn a_body_on_an_untracked_layer_is_left_alone_by_the_water() {
    let mut context = crate_context_with(Some(WATER_HEIGHT), Some(made_of(2, &[(FLOATING, 1)])));
    run(&mut context, 30);
    let body = crate_body(&context);
    assert_eq!(body.gravity_scale(), 1.0);
    assert!(body.translation().y < 1.0, "it just falls");
}

#[test]
fn the_explicit_buoyancy_override_beats_the_materials() {
    let mut context = crate_context_with(Some(WATER_HEIGHT), Some(made_of(4, &[(FLOATING, 1)])));
    context.set_material_buoyancy(refr_entity_id(CRATE_REF), -1.0);
    run(&mut context, FIVE_SECONDS);
    assert!(crate_body(&context).translation().y < 1.0);
}

#[test]
fn leaving_the_water_volume_restores_full_gravity() {
    let mut context = crate_context(Some(WATER_HEIGHT));
    run(&mut context, 30);
    assert_eq!(crate_body(&context).gravity_scale(), 0.1);
    context.set_water_query(Some(Box::new(|_| None)));
    run(&mut context, 1);
    assert_eq!(crate_body(&context).gravity_scale(), 1.0);
}

#[test]
fn a_dry_world_is_never_woken_and_goes_idle() {
    let mut context = crate_context(Some(-100.0));
    run(&mut context, 600);
    assert!(!context.needs_step(), "settled dry clutter sleeps");
    assert_eq!(crate_body(&context).gravity_scale(), 1.0);
}

#[test]
fn a_sleeping_crate_in_water_is_woken() {
    let mut context = crate_context(None);
    run(&mut context, 600);
    assert!(!context.needs_step(), "asleep on the ground");
    context.set_water_query(Some(infinite_plane(WATER_HEIGHT)));
    context.update_water(CRATE_START);
    assert!(context.needs_step());
}
