//! Wake-only kinematic mirror of a character-controller capsule. The controller
//! has no rigid body, so rapier's islands never see the character; the mirror
//! gives the narrow phase a contact start to wake sleeping clutter on. Solver
//! groups match nothing, so it never pushes (shoving stays the controller's
//! impulse path), and [`scene_query_filter`] hides it from every scene query.

use glam::Vec3;
use rapier3d::prelude::*;

/// Collision group only mirrors belong to; scene queries filter it out.
const MIRROR_GROUP: Group = Group::GROUP_32;

/// Contacts only start within the prediction distance (~0.014), less than the
/// controller's skin gap, so the mirror radius is inflated to still touch.
const WAKE_MARGIN: f32 = 0.1;

/// Every scene query must use this so the mirror never hits its own character.
pub(crate) fn scene_query_filter() -> QueryFilter<'static> {
    QueryFilter::default().groups(InteractionGroups::new(
        Group::ALL,
        Group::ALL.difference(MIRROR_GROUP),
        InteractionTestMode::And,
    ))
}

/// Scene query over everything the character walks on and slides along:
/// dynamic bodies are left to the shove pass (`dynamic_contact`).
pub(crate) fn static_scene_query_filter() -> QueryFilter<'static> {
    let mut filter = scene_query_filter();
    filter.flags |= QueryFilterFlags::EXCLUDE_DYNAMIC;
    filter
}

/// Scene query over dynamic bodies only.
pub(crate) fn dynamic_scene_query_filter() -> QueryFilter<'static> {
    let mut filter = scene_query_filter();
    filter.flags |= QueryFilterFlags::ONLY_DYNAMIC;
    filter
}

pub(crate) struct KinematicMirror {
    body: RigidBodyHandle,
    collider: ColliderHandle,
    target: Vec3,
    shape_source: Capsule,
}

impl KinematicMirror {
    pub fn new(world: &mut PhysicsWorld, center: Vec3, capsule: &Capsule) -> Self {
        let body =
            world.insert_body(RigidBodyBuilder::kinematic_position_based().translation(center));
        let collider = world.insert_collider(
            ColliderBuilder::new(inflated(capsule))
                .collision_groups(InteractionGroups::new(
                    MIRROR_GROUP,
                    Group::ALL,
                    InteractionTestMode::And,
                ))
                .solver_groups(InteractionGroups::none()),
            Some(body),
        );
        Self {
            body,
            collider,
            target: center,
            shape_source: *capsule,
        }
    }

    pub fn body(&self) -> RigidBodyHandle {
        self.body
    }

    /// Instant placement (spawn, teleport): no sweep through the world.
    pub fn teleport(&mut self, world: &mut PhysicsWorld, center: Vec3, capsule: &Capsule) {
        self.place(world, center, capsule, |body| {
            body.set_translation(center, true);
        });
    }

    /// Aim at `center` for the next step; false when nothing changed.
    pub fn follow(&mut self, world: &mut PhysicsWorld, center: Vec3, capsule: &Capsule) -> bool {
        if center == self.target && !self.shape_differs(capsule) {
            return false;
        }
        self.place(world, center, capsule, |body| {
            body.set_next_kinematic_translation(center);
        });
        true
    }

    /// Dynamic bodies in active contact with the mirror.
    pub fn touched_bodies(&self, world: &PhysicsWorld) -> Vec<RigidBodyHandle> {
        world
            .contact_pairs_with(self.collider)
            .filter(|pair| pair.has_any_active_contact())
            .map(|pair| {
                if pair.collider1 == self.collider {
                    pair.collider2
                } else {
                    pair.collider1
                }
            })
            .filter_map(|other| world.colliders.get(other)?.parent())
            .collect()
    }

    fn place(
        &mut self,
        world: &mut PhysicsWorld,
        center: Vec3,
        capsule: &Capsule,
        move_body: impl FnOnce(&mut RigidBody),
    ) {
        if self.shape_differs(capsule) {
            if let Some(collider) = world.colliders.get_mut(self.collider) {
                collider.set_shape(inflated(capsule));
            } else {
                tracing::error!("physics: player mirror collider is missing");
            }
            self.shape_source = *capsule;
        }
        if let Some(body) = world.bodies.get_mut(self.body) {
            move_body(body);
        } else {
            tracing::error!("physics: player mirror body is missing");
        }
        self.target = center;
    }

    #[allow(
        clippy::float_cmp,
        reason = "change detection: the shape source is a verbatim copy of the last capsule"
    )]
    fn shape_differs(&self, capsule: &Capsule) -> bool {
        capsule.radius != self.shape_source.radius
            || capsule.half_height() != self.shape_source.half_height()
    }
}

fn inflated(capsule: &Capsule) -> SharedShape {
    SharedShape::new(Capsule::new_y(
        capsule.half_height(),
        capsule.radius + WAKE_MARGIN,
    ))
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use super::*;

    fn capsule() -> Capsule {
        Capsule::new_y(1.0, 0.5)
    }

    fn world_without_gravity() -> PhysicsWorld {
        let mut world = PhysicsWorld::new();
        world.gravity = Vec3::ZERO;
        world
    }

    fn crate_at(
        world: &mut PhysicsWorld,
        body: RigidBodyBuilder,
        position: Vec3,
    ) -> RigidBodyHandle {
        let handle = world.insert_body(body.translation(position));
        world.insert_collider(ColliderBuilder::cuboid(0.5, 0.5, 0.5), Some(handle));
        handle
    }

    fn hits(world: &PhysicsWorld, filter: QueryFilter<'static>, at: Vec3) -> usize {
        world
            .query_pipeline_with_filter(filter)
            .intersect_shape(Pose::from_translation(at), &Ball::new(0.1))
            .count()
    }

    #[test]
    fn scene_queries_never_see_the_mirror() {
        let mut world = world_without_gravity();
        let _mirror = KinematicMirror::new(&mut world, Vec3::ZERO, &capsule());
        world.step();
        assert_eq!(
            hits(&world, QueryFilter::default(), Vec3::ZERO),
            1,
            "unfiltered query finds it"
        );
        assert_eq!(hits(&world, scene_query_filter(), Vec3::ZERO), 0);
    }

    #[test]
    fn scene_queries_still_see_ordinary_colliders_overlapping_the_mirror() {
        let mut world = world_without_gravity();
        let _mirror = KinematicMirror::new(&mut world, Vec3::ZERO, &capsule());
        crate_at(&mut world, RigidBodyBuilder::fixed(), Vec3::ZERO);
        world.step();
        assert_eq!(hits(&world, scene_query_filter(), Vec3::ZERO), 1);
    }

    #[test]
    fn static_and_dynamic_filters_split_bodies_by_type() {
        let mut world = world_without_gravity();
        crate_at(
            &mut world,
            RigidBodyBuilder::fixed(),
            Vec3::new(-5.0, 0.0, 0.0),
        );
        crate_at(
            &mut world,
            RigidBodyBuilder::dynamic(),
            Vec3::new(5.0, 0.0, 0.0),
        );
        world.step();
        let fixed_at = Vec3::new(-5.0, 0.0, 0.0);
        let dynamic_at = Vec3::new(5.0, 0.0, 0.0);
        assert_eq!(hits(&world, static_scene_query_filter(), fixed_at), 1);
        assert_eq!(hits(&world, static_scene_query_filter(), dynamic_at), 0);
        assert_eq!(hits(&world, dynamic_scene_query_filter(), dynamic_at), 1);
        assert_eq!(hits(&world, dynamic_scene_query_filter(), fixed_at), 0);
    }

    #[test]
    fn follow_reports_no_change_for_an_identical_target() {
        let mut world = world_without_gravity();
        let mut mirror = KinematicMirror::new(&mut world, Vec3::ZERO, &capsule());
        assert!(!mirror.follow(&mut world, Vec3::ZERO, &capsule()));
        assert!(mirror.follow(&mut world, Vec3::X, &capsule()));
        assert!(!mirror.follow(&mut world, Vec3::X, &capsule()));
    }

    #[test]
    fn follow_reports_a_change_when_only_the_capsule_shape_changed() {
        let mut world = world_without_gravity();
        let mut mirror = KinematicMirror::new(&mut world, Vec3::ZERO, &capsule());
        assert!(mirror.follow(&mut world, Vec3::ZERO, &Capsule::new_y(0.4, 0.5)));
    }

    #[test]
    fn teleport_places_the_body_immediately() {
        let mut world = world_without_gravity();
        let mut mirror = KinematicMirror::new(&mut world, Vec3::ZERO, &capsule());
        mirror.teleport(&mut world, Vec3::new(10.0, 2.0, -3.0), &capsule());
        let (_, body) = world.bodies.iter().next().unwrap();
        assert_eq!(body.translation(), Vec3::new(10.0, 2.0, -3.0));
    }

    #[test]
    fn a_followed_target_is_reached_by_the_next_step() {
        let mut world = world_without_gravity();
        let mut mirror = KinematicMirror::new(&mut world, Vec3::ZERO, &capsule());
        mirror.follow(&mut world, Vec3::new(3.0, 0.0, 0.0), &capsule());
        world.step();
        let (_, body) = world.bodies.iter().next().unwrap();
        assert!(
            body.translation()
                .abs_diff_eq(Vec3::new(3.0, 0.0, 0.0), 1e-5)
        );
    }

    #[test]
    fn touched_bodies_lists_the_dynamic_body_in_contact_and_not_a_distant_one() {
        let mut world = world_without_gravity();
        let mirror = KinematicMirror::new(&mut world, Vec3::ZERO, &capsule());
        let near = crate_at(
            &mut world,
            RigidBodyBuilder::dynamic(),
            Vec3::new(0.8, 0.0, 0.0),
        );
        let far = crate_at(
            &mut world,
            RigidBodyBuilder::dynamic(),
            Vec3::new(20.0, 0.0, 0.0),
        );
        world.step();
        let touched = mirror.touched_bodies(&world);
        assert!(touched.contains(&near));
        assert!(!touched.contains(&far));
    }

    #[test]
    fn the_mirror_never_pushes_the_body_it_touches() {
        let mut world = world_without_gravity();
        let _mirror = KinematicMirror::new(&mut world, Vec3::ZERO, &capsule());
        let touching = crate_at(
            &mut world,
            RigidBodyBuilder::dynamic(),
            Vec3::new(0.8, 0.0, 0.0),
        );
        for _ in 0..30 {
            world.step();
        }
        assert!(
            world.bodies[touching]
                .translation()
                .abs_diff_eq(Vec3::new(0.8, 0.0, 0.0), 1e-4)
        );
        assert!(world.bodies[touching].linvel().length() < 1e-4);
    }
}
