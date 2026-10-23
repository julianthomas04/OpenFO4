//! Kinematic rigid bodies for animated parts (door leaves, container lids).
//! A placement's shapes that follow an animated joint ride one
//! `KinematicPositionBased` body per joint; the renderer's animation tick
//! commands each body's world pose and rapier sweeps it there, pushing the
//! player and clutter instead of teleporting through them.

use std::collections::HashMap;

use glam::{Quat, Vec3};
use rapier3d::prelude::*;

use super::bake::CachedShape;
use super::collider_user_data::ColliderTag;
use super::commands::KinematicPose;
use super::sound_pose::{pose_is_sound, vec_is_sound};
use super::{CellGrid, EntityId};

/// Steps owed after a pose command: one for the body to reach its target, one
/// for the broad phase and contact pairs to see the new position.
const SETTLE_STEPS: u8 = 2;

#[derive(Debug, thiserror::Error)]
pub(crate) enum KinematicError {
    #[error("kinematic body for entity {entity:?} joint {joint} is missing from the physics world")]
    BodyMissing { entity: EntityId, joint: u16 },
    #[error("non-finite kinematic pose for entity {entity:?} joint {joint}")]
    NonFinitePose { entity: EntityId, joint: u16 },
}

struct PartBody {
    joint: u16,
    handle: RigidBodyHandle,
}

#[derive(Default)]
pub(crate) struct KinematicBodies {
    by_entity: HashMap<EntityId, Vec<PartBody>>,
    /// Entities each cell owns, so `remove_cell` despawns their bodies.
    by_cell: HashMap<CellGrid, Vec<EntityId>>,
    /// Last pose commanded per (entity, joint), so bodies built after the
    /// command (their cell reloaded) start where the part visually is. Kept
    /// until the renderer releases the entity.
    last_poses: HashMap<(EntityId, u16), (Vec3, Quat)>,
    steps_owed: u8,
}

impl KinematicBodies {
    /// Builds the entity's bodies (one per distinct joint among `shapes`) at
    /// `refr_pose` and attaches every shape as a collider. Returns the number
    /// of bodies the entity now owns.
    pub fn attach(
        &mut self,
        world: &mut PhysicsWorld,
        grid: CellGrid,
        entity: EntityId,
        refr_pose: Pose,
        shapes: &[&CachedShape],
        tag: &ColliderTag,
    ) -> usize {
        let bodies = self.by_entity.entry(entity).or_default();
        for cached in shapes {
            let Some(joint) = cached.joint else { continue };
            let handle = if let Some(part) = bodies.iter().find(|part| part.joint == joint) {
                part.handle
            } else {
                let start_pose = self
                    .last_poses
                    .get(&(entity, joint))
                    .map_or(refr_pose, |&(translation, rotation)| {
                        Pose::from_parts(translation, rotation)
                    });
                if !pose_is_sound(&start_pose) {
                    crate::utils::warn_once(
                        &format!("kinematic: non-finite start pose {entity:?} joint {joint}"),
                        || {
                            format!(
                                "kinematic: entity {entity:?} joint {joint} has a non-finite start pose; its animated colliders are skipped"
                            )
                        },
                    );
                    continue;
                }
                let handle = world
                    .insert_body(RigidBodyBuilder::kinematic_position_based().pose(start_pose));
                bodies.push(PartBody { joint, handle });
                handle
            };
            let collider = ColliderBuilder::new(cached.shape.clone())
                .position(cached.local_pose)
                .user_data(tag.user_data(cached.material));
            world.insert_collider(collider, Some(handle));
        }
        let count = bodies.len();
        if count == 0 {
            self.by_entity.remove(&entity);
        } else {
            let owned = self.by_cell.entry(grid).or_default();
            if !owned.contains(&entity) {
                owned.push(entity);
            }
        }
        count
    }

    /// Aims each body at its commanded pose for the next step. Poses for
    /// entities without bodies (collision ring smaller than the render ring)
    /// are expected and skipped.
    pub fn set_poses(&mut self, world: &mut PhysicsWorld, poses: &[KinematicPose]) {
        for pose in poses {
            match self.set_pose(world, pose) {
                Ok(true) => self.steps_owed = SETTLE_STEPS,
                Ok(false) => {}
                Err(error) => {
                    crate::utils::warn_once(&format!("kinematic:{error}"), || error.to_string());
                }
            }
        }
    }

    fn set_pose(
        &mut self,
        world: &mut PhysicsWorld,
        pose: &KinematicPose,
    ) -> Result<bool, KinematicError> {
        let KinematicPose {
            entity,
            joint,
            translation,
            rotation,
        } = *pose;
        let usable = vec_is_sound(translation)
            && rotation.is_finite()
            && rotation.length_squared() > f32::EPSILON;
        if !usable {
            return Err(KinematicError::NonFinitePose { entity, joint });
        }
        let rotation = rotation.normalize();
        self.last_poses
            .insert((entity, joint), (translation, rotation));
        let Some(part) = self
            .by_entity
            .get(&entity)
            .and_then(|parts| parts.iter().find(|p| p.joint == joint))
        else {
            return Ok(false);
        };
        let body = world
            .bodies
            .get_mut(part.handle)
            .ok_or(KinematicError::BodyMissing { entity, joint })?;
        body.set_next_kinematic_position(Pose::from_parts(translation, rotation));
        Ok(true)
    }

    /// Forgets the remembered poses of entities the renderer no longer animates.
    pub fn release_entities(&mut self, entities: &[EntityId]) {
        self.last_poses
            .retain(|(entity, _), _| !entities.contains(entity));
    }

    pub fn release_all(&mut self) {
        self.last_poses.clear();
    }

    /// Despawns the cell's bodies (and their colliders); true if any existed.
    pub fn remove_cell(&mut self, world: &mut PhysicsWorld, grid: CellGrid) -> bool {
        let Some(entities) = self.by_cell.remove(&grid) else {
            return false;
        };
        for entity in entities {
            for part in self.by_entity.remove(&entity).unwrap_or_default() {
                world.remove_body(part.handle);
            }
        }
        true
    }

    pub fn needs_step(&self) -> bool {
        self.steps_owed > 0
    }

    pub fn step_done(&mut self) {
        self.steps_owed = self.steps_owed.saturating_sub(1);
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use super::*;

    const CELL: CellGrid = (3, -2);
    const DOOR: EntityId = EntityId(42);

    fn door_shape(joint: Option<u16>) -> CachedShape {
        CachedShape {
            shape: SharedShape::cuboid(1.0, 2.0, 0.1),
            local_pose: Pose::IDENTITY,
            joint,
            material: 0,
        }
    }

    fn pose_command(entity: EntityId, joint: u16, translation: Vec3) -> KinematicPose {
        KinematicPose {
            entity,
            joint,
            translation,
            rotation: Quat::IDENTITY,
        }
    }

    fn attach(
        bodies: &mut KinematicBodies,
        world: &mut PhysicsWorld,
        shapes: &[CachedShape],
        refr: Vec3,
    ) -> usize {
        let tag = ColliderTag::new(0, shapes.iter().map(|shape| shape.material), |material| {
            material + 1
        });
        let refs: Vec<&CachedShape> = shapes.iter().collect();
        bodies.attach(world, CELL, DOOR, Pose::from_translation(refr), &refs, &tag)
    }

    #[test]
    fn each_collider_carries_the_slot_of_its_own_material() {
        use super::super::collider_user_data::unpack_path;
        let mut world = PhysicsWorld::new();
        let mut bodies = KinematicBodies::default();
        let shapes = [
            CachedShape {
                material: 7,
                ..door_shape(Some(1))
            },
            CachedShape {
                material: 9,
                ..door_shape(Some(1))
            },
        ];
        attach(&mut bodies, &mut world, &shapes, Vec3::ZERO);
        let mut slots: Vec<u32> = world
            .colliders
            .iter()
            .map(|(_, collider)| unpack_path(collider.user_data))
            .collect();
        slots.sort_unstable();
        assert_eq!(slots, [8, 10]);
    }

    fn only_body_position(world: &PhysicsWorld) -> Vec3 {
        let (_, body) = world.bodies.iter().next().expect("one body");
        body.translation()
    }

    #[test]
    fn attach_builds_one_body_per_distinct_joint() {
        let mut world = PhysicsWorld::new();
        let mut bodies = KinematicBodies::default();
        let shapes = [
            door_shape(Some(1)),
            door_shape(Some(1)),
            door_shape(Some(2)),
        ];
        assert_eq!(attach(&mut bodies, &mut world, &shapes, Vec3::ZERO), 2);
        assert_eq!(world.bodies.len(), 2);
        assert_eq!(world.colliders.len(), 3);
    }

    #[test]
    fn attach_ignores_shapes_that_follow_no_joint() {
        let mut world = PhysicsWorld::new();
        let mut bodies = KinematicBodies::default();
        assert_eq!(
            attach(&mut bodies, &mut world, &[door_shape(None)], Vec3::ZERO),
            0
        );
        assert_eq!(world.bodies.len(), 0);
        assert!(
            !bodies.remove_cell(&mut world, CELL),
            "an entity without bodies owns nothing"
        );
    }

    #[test]
    fn a_pose_command_moves_the_body_to_the_target_on_the_next_step() {
        let mut world = PhysicsWorld::new();
        let mut bodies = KinematicBodies::default();
        attach(
            &mut bodies,
            &mut world,
            &[door_shape(Some(1))],
            Vec3::new(1.0, 0.0, 0.0),
        );
        bodies.set_poses(
            &mut world,
            &[pose_command(DOOR, 1, Vec3::new(4.0, 1.0, 0.0))],
        );
        world.step();
        assert!(only_body_position(&world).abs_diff_eq(Vec3::new(4.0, 1.0, 0.0), 1e-4));
    }

    #[test]
    fn a_pose_command_owes_exactly_two_settle_steps() {
        let mut world = PhysicsWorld::new();
        let mut bodies = KinematicBodies::default();
        attach(&mut bodies, &mut world, &[door_shape(Some(1))], Vec3::ZERO);
        assert!(!bodies.needs_step());
        bodies.set_poses(&mut world, &[pose_command(DOOR, 1, Vec3::X)]);
        assert!(bodies.needs_step());
        bodies.step_done();
        assert!(bodies.needs_step());
        bodies.step_done();
        assert!(!bodies.needs_step());
        bodies.step_done();
        assert!(!bodies.needs_step(), "saturates instead of underflowing");
    }

    #[test]
    fn poses_for_entities_without_bodies_are_skipped_without_owing_steps() {
        let mut world = PhysicsWorld::new();
        let mut bodies = KinematicBodies::default();
        bodies.set_poses(&mut world, &[pose_command(DOOR, 1, Vec3::X)]);
        assert!(!bodies.needs_step());
    }

    #[test]
    fn a_zero_rotation_is_rejected_and_a_scaled_one_is_normalised() {
        let mut world = PhysicsWorld::new();
        let mut bodies = KinematicBodies::default();
        attach(&mut bodies, &mut world, &[door_shape(Some(1))], Vec3::ZERO);
        let command = |rotation| KinematicPose {
            rotation,
            ..pose_command(DOOR, 1, Vec3::X)
        };
        bodies.set_poses(&mut world, &[command(Quat::from_xyzw(0.0, 0.0, 0.0, 0.0))]);
        assert!(!bodies.needs_step(), "a zero quaternion cannot be aimed at");
        bodies.set_poses(&mut world, &[command(Quat::from_xyzw(0.0, 0.0, 0.0, 3.0))]);
        world.step();
        let (_, body) = world.bodies.iter().next().expect("one body");
        assert!(body.rotation().is_normalized());
    }

    #[test]
    fn a_part_with_a_non_finite_start_pose_gets_no_body() {
        let mut world = PhysicsWorld::new();
        let mut bodies = KinematicBodies::default();
        let nan = Vec3::new(f32::NAN, 0.0, 0.0);
        assert_eq!(
            attach(&mut bodies, &mut world, &[door_shape(Some(1))], nan),
            0
        );
        assert_eq!(world.bodies.len(), 0);
        assert_eq!(world.colliders.len(), 0);
    }

    #[test]
    fn non_finite_poses_are_rejected_and_leave_the_body_where_it_was() {
        let mut world = PhysicsWorld::new();
        let mut bodies = KinematicBodies::default();
        attach(
            &mut bodies,
            &mut world,
            &[door_shape(Some(1))],
            Vec3::new(2.0, 0.0, 0.0),
        );
        bodies.set_poses(
            &mut world,
            &[pose_command(DOOR, 1, Vec3::new(f32::NAN, 0.0, 0.0))],
        );
        world.step();
        assert!(!bodies.needs_step());
        assert!(only_body_position(&world).abs_diff_eq(Vec3::new(2.0, 0.0, 0.0), 1e-6));
    }

    #[test]
    fn bodies_built_after_a_pose_command_start_at_the_remembered_pose() {
        let mut world = PhysicsWorld::new();
        let mut bodies = KinematicBodies::default();
        bodies.set_poses(
            &mut world,
            &[pose_command(DOOR, 1, Vec3::new(7.0, 0.0, 0.0))],
        );
        attach(&mut bodies, &mut world, &[door_shape(Some(1))], Vec3::ZERO);
        assert!(only_body_position(&world).abs_diff_eq(Vec3::new(7.0, 0.0, 0.0), 1e-6));
    }

    #[test]
    fn releasing_an_entity_forgets_its_remembered_pose() {
        let mut world = PhysicsWorld::new();
        let mut bodies = KinematicBodies::default();
        bodies.set_poses(
            &mut world,
            &[pose_command(DOOR, 1, Vec3::new(7.0, 0.0, 0.0))],
        );
        bodies.release_entities(&[DOOR]);
        attach(
            &mut bodies,
            &mut world,
            &[door_shape(Some(1))],
            Vec3::new(1.0, 0.0, 0.0),
        );
        assert!(only_body_position(&world).abs_diff_eq(Vec3::new(1.0, 0.0, 0.0), 1e-6));
    }

    #[test]
    fn releasing_everything_forgets_every_remembered_pose() {
        let mut world = PhysicsWorld::new();
        let mut bodies = KinematicBodies::default();
        bodies.set_poses(
            &mut world,
            &[pose_command(DOOR, 1, Vec3::new(7.0, 0.0, 0.0))],
        );
        bodies.release_all();
        attach(&mut bodies, &mut world, &[door_shape(Some(1))], Vec3::ZERO);
        assert!(only_body_position(&world).abs_diff_eq(Vec3::ZERO, 1e-6));
    }

    #[test]
    fn releasing_another_entity_keeps_the_remembered_pose() {
        let mut world = PhysicsWorld::new();
        let mut bodies = KinematicBodies::default();
        bodies.set_poses(
            &mut world,
            &[pose_command(DOOR, 1, Vec3::new(7.0, 0.0, 0.0))],
        );
        bodies.release_entities(&[EntityId(99)]);
        attach(&mut bodies, &mut world, &[door_shape(Some(1))], Vec3::ZERO);
        assert!(only_body_position(&world).abs_diff_eq(Vec3::new(7.0, 0.0, 0.0), 1e-6));
    }

    #[test]
    fn removing_a_cell_despawns_its_bodies_and_colliders() {
        let mut world = PhysicsWorld::new();
        let mut bodies = KinematicBodies::default();
        attach(
            &mut bodies,
            &mut world,
            &[door_shape(Some(1)), door_shape(Some(2))],
            Vec3::ZERO,
        );
        assert!(bodies.remove_cell(&mut world, CELL));
        assert_eq!(world.bodies.len(), 0);
        assert_eq!(world.colliders.len(), 0);
        assert!(
            !bodies.remove_cell(&mut world, CELL),
            "second removal finds nothing"
        );
    }

    #[test]
    fn a_closing_door_pushes_a_dynamic_body_instead_of_passing_through_it() {
        let mut world = PhysicsWorld::new();
        world.gravity = Vec3::ZERO;
        let mut bodies = KinematicBodies::default();
        attach(
            &mut bodies,
            &mut world,
            &[door_shape(Some(1))],
            Vec3::new(-5.0, 0.0, 0.0),
        );
        let crate_body =
            world.insert_body(RigidBodyBuilder::dynamic().translation(Vec3::new(0.0, 0.0, 0.0)));
        world.insert_collider(ColliderBuilder::cuboid(0.5, 0.5, 0.5), Some(crate_body));
        for step in 1..=60 {
            let x = -5.0 + step as f32 * 0.1;
            bodies.set_poses(&mut world, &[pose_command(DOOR, 1, Vec3::new(x, 0.0, 0.0))]);
            world.step();
        }
        assert!(
            world.bodies[crate_body].translation().x > 0.5,
            "the door swept 1 unit past the crate's start; it must have been shoved"
        );
    }
}
