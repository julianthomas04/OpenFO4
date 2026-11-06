//! Whole-`SimWorld` fixtures for tests that move the player through a tiny synthetic cell.

use std::sync::Arc;

use glam::{Quat, Vec2, Vec3};

use super::collision::{CellColliderBatch, ColShape, PartShape};
use super::commands::{PlayerInput, SimCommand};
use super::physics::test_support::{bake, box_shape, ground_batch, object_group, placement};
use super::{FIXED_DT, PlayerMovement, SimWorld, WorldClock};

pub const SPAWN_AT: Vec3 = Vec3::new(200.0, 3.0, -100.0);

pub struct Prop {
    pub path: &'static str,
    pub at: Vec3,
    pub shape: PartShape,
    pub dynamic: bool,
    pub mass: Option<f32>,
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

pub fn world_in(mut batch: CellColliderBatch, props: &[Prop]) -> SimWorld {
    for (index, prop) in props.iter().enumerate() {
        batch.objects.push(object_group(
            prop.path,
            vec![placement(&prop_ref_key(index), prop.at)],
            prop.dynamic,
            prop.mass,
        ));
    }
    let shapes: Vec<(&str, Vec<PartShape>)> = props
        .iter()
        .map(|prop| (prop.path, vec![prop.shape.clone()]))
        .collect();
    let baked = bake(&batch, &shapes);
    let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
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

pub fn walking(yaw: f32) -> PlayerInput {
    PlayerInput {
        move_dir: Vec2::new(0.0, 1.0),
        yaw,
        ..PlayerInput::default()
    }
}

pub fn position(sim: &SimWorld) -> Vec3 {
    sim.player_debug().pos
}
