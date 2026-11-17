//! Whole-`SimWorld` fixtures for tests that move the player through a tiny synthetic cell.

use std::sync::Arc;

use glam::{Quat, Vec2, Vec3};

use super::collision::{CellColliderBatch, ColShape, ObjectColliderGroup, PartShape};
use super::commands::{PlayerInput, SimCommand};
use super::physics::test_support::{bake, box_shape, ground_batch, object_group, placement};
use super::{EyeHeights, FIXED_DT, PlayerMovement, SimWorld, WorldClock};

pub const SPAWN_AT: Vec3 = Vec3::new(200.0, 3.0, -100.0);

pub struct Prop {
    pub path: &'static str,
    pub at: Vec3,
    pub shape: PartShape,
    pub dynamic: bool,
    pub mass: Option<f32>,
    pub pinned: bool,
}

pub fn prop_box(path: &'static str, at: Vec3, half: Vec3) -> Prop {
    Prop {
        path,
        at,
        shape: PartShape {
            shape: ColShape::Box {
                center: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                half_extents: half,
            },
            node: None,
            joint: None,
            material: 0,
        },
        dynamic: false,
        mass: None,
        pinned: false,
    }
}

/// A dynamic 1 x 1 x 1 crate centred at `at`.
pub fn crate_prop(at: Vec3, mass: f32) -> Prop {
    Prop {
        path: "meshes\\crate.glb",
        at,
        shape: box_shape(0.5),
        dynamic: true,
        mass: Some(mass),
        pinned: false,
    }
}

/// A static 1 x 1 x 1 item centred at `at` that the sim tracks as a fixed body.
pub fn pinned_prop(at: Vec3) -> Prop {
    Prop {
        pinned: true,
        ..prop_box("meshes\\item.glb", at, Vec3::splat(0.5))
    }
}

/// The entity id of the `index`-th prop handed to [`world_with`] or [`world_in`].
pub fn prop_entity(index: usize) -> super::EntityId {
    super::refr_entity_id(&prop_ref_key(index))
}

fn prop_ref_key(index: usize) -> String {
    format!("{:06X}:Fallout4.esm", 0x0A_0000 + index)
}

/// A flat world with `props` registered in cell (0, 0).
pub fn world_with(props: &[Prop]) -> SimWorld {
    world_in(ground_batch(), props)
}

pub fn world_in(batch: CellColliderBatch, props: &[Prop]) -> SimWorld {
    world_in_with_eye_heights(batch, props, EyeHeights::default())
}

/// A flat world without props whose player sees from `eye_heights`.
pub fn flat_world_with_eye_heights(eye_heights: EyeHeights) -> SimWorld {
    world_in_with_eye_heights(ground_batch(), &[], eye_heights)
}

fn world_in_with_eye_heights(
    mut batch: CellColliderBatch,
    props: &[Prop],
    eye_heights: EyeHeights,
) -> SimWorld {
    for (index, prop) in props.iter().enumerate() {
        batch.objects.push(ObjectColliderGroup {
            pinned: prop.pinned,
            ..object_group(
                prop.path,
                vec![placement(&prop_ref_key(index), prop.at)],
                prop.dynamic,
                prop.mass,
            )
        });
    }
    let shapes: Vec<(&str, Vec<PartShape>)> = props
        .iter()
        .map(|prop| (prop.path, vec![prop.shape.clone()]))
        .collect();
    let baked = bake(&batch, &shapes);
    let mut sim = SimWorld::new(
        PlayerMovement::default(),
        eye_heights,
        WorldClock::fixture(),
    );
    sim.apply(SimCommand::AddCellColliders {
        grid: (0, 0),
        batch: Arc::new(batch),
        baked: Some(baked),
    });
    sim.apply(SimCommand::SetPhysicsCenter {
        grid: (0, 0),
        radius: 2,
    });
    sim
}

/// Adds a dynamic 1 x 1 x 1 crate centred at `at` to cell (0, 0) as a dropped item enters the
/// world: awake from its first step.
pub fn drop_crate(sim: &mut SimWorld, at: Vec3) -> super::EntityId {
    const DROPPED_REF: &str = "00000A:*dropped";
    let batch = CellColliderBatch {
        objects: vec![object_group(
            "meshes\\crate.glb",
            vec![placement(DROPPED_REF, at)],
            true,
            Some(2.0),
        )],
        ..CellColliderBatch::default()
    };
    let baked = bake(&batch, &[("meshes\\crate.glb", vec![box_shape(0.5)])]);
    sim.apply(SimCommand::AddWorldObjects {
        grid: (0, 0),
        batch: Arc::new(batch),
        baked: Some(baked),
    });
    super::refr_entity_id(DROPPED_REF)
}

pub fn flat_world() -> SimWorld {
    world_with(&[])
}

pub fn tick(sim: &mut SimWorld, ticks: u32) {
    for _ in 0..ticks {
        sim.tick(FIXED_DT);
    }
}

pub fn spawned_at(mut sim: SimWorld, at: Vec3) -> SimWorld {
    sim.apply(SimCommand::SpawnPlayer { pos: at });
    tick(&mut sim, 2);
    sim
}

pub fn hold(sim: &mut SimWorld, input: PlayerInput) {
    sim.apply(SimCommand::SetPlayerInput(input));
}

pub fn running(yaw: f32) -> PlayerInput {
    PlayerInput {
        move_dir: Vec2::new(0.0, 1.0),
        yaw,
        ..PlayerInput::default()
    }
}

pub fn position(sim: &SimWorld) -> Vec3 {
    sim.player_debug().pos
}
