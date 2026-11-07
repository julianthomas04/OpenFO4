//! The rapier side of carrying (`sim::carry`): the carried body stays an ordinary dynamic
//! body, driven each fixed tick by a velocity servo towards its hold pose, so the world can
//! push back on it. Every way of letting go runs through [`PhysicsContext::release`].

use glam::{Quat, Vec3};
use rapier3d::prelude::*;

use super::{PLAYER_DRIVEN_MAX_LINVEL, PhysicsContext};
use crate::sim::EntityId;
use crate::sim::carry::{self, CarriedInfo, GrabRefusal, ReleaseReason, Strain, View};
use crate::sim::dynamic_body::{MAX_DYN_ANGVEL, clamp_angvel, clamp_linvel};
use crate::sim::mirror::{dynamic_scene_query_filter, scene_query_filter};
use crate::utils::warn_once;

/// What carrying holds and what it restores on release.
#[derive(Default)]
pub(super) struct Carry {
    body: Option<CarriedBody>,
    last_release: Option<ReleaseReason>,
}

struct CarriedBody {
    entity: EntityId,
    handle: RigidBodyHandle,
    mass: f32,
    hold_distance: f32,
    /// The body's rotation in the camera frame (`View::rotation`), spin included.
    camera_relative: Quat,
    /// Body-local axis the mouse buttons spin about.
    spin_axis: Vec3,
    strain: Strain,
    sleep: SleepThresholds,
}

/// The body's own sleep thresholds, put back when it is let go.
#[derive(Copy, Clone)]
struct SleepThresholds {
    linear: f32,
    angular: f32,
}

impl SleepThresholds {
    /// Negative thresholds keep a body awake whatever its speed.
    const NEVER: Self = Self {
        linear: -1.0,
        angular: -1.0,
    };

    fn of(body: &RigidBody) -> Self {
        let activation = body.activation();
        Self {
            linear: activation.normalized_linear_threshold,
            angular: activation.angular_threshold,
        }
    }

    fn apply(self, body: &mut RigidBody) {
        let activation = body.activation_mut();
        activation.normalized_linear_threshold = self.linear;
        activation.angular_threshold = self.angular;
    }
}

impl PhysicsContext {
    /// Whether `entity` is a loose body simulating inside the physics radius.
    pub fn is_grabbable(&self, entity: EntityId) -> bool {
        self.grabbable_handle(entity).is_some()
    }

    fn grabbable_handle(&self, entity: EntityId) -> Option<RigidBodyHandle> {
        let body = self.dyn_bodies.get(&entity).filter(|body| body.active)?;
        self.world
            .bodies
            .get(body.handle)
            .filter(|rb| rb.is_dynamic())
            .map(|_| body.handle)
    }

    /// Starts carrying `entity` as seen from `eye` along `view`. Its hold distance, its
    /// rotation relative to the camera and its spin axis are fixed now.
    pub fn grab(&mut self, entity: EntityId, eye: Vec3, view: View) -> Result<(), GrabRefusal> {
        if self.carry.body.is_some() {
            return Err(GrabRefusal::AlreadyCarrying);
        }
        let handle = self
            .grabbable_handle(entity)
            .ok_or(GrabRefusal::NotDynamic)?;
        let rb = &mut self.world.bodies[handle];
        let mass = rb.mass();
        if !carry::can_grab(mass) {
            return Err(GrabRefusal::TooHeavy { mass });
        }
        let pose = *rb.position();
        let sleep = SleepThresholds::of(rb);
        SleepThresholds::NEVER.apply(rb);
        rb.wake_up(true);
        self.carry.body = Some(CarriedBody {
            entity,
            handle,
            mass,
            hold_distance: carry::hold_distance(eye.distance(pose.translation)),
            camera_relative: view.rotation().inverse() * pose.rotation,
            spin_axis: carry::spin_axis(pose.rotation, view.right()),
            strain: Strain::default(),
            sleep,
        });
        Ok(())
    }

    /// Sets the carried body's velocities so the next step brings it onto its hold pose in
    /// front of `eye`, after turning it by `spin` (see `carry::spin_step`). Releases it as
    /// [`ReleaseReason::TornAway`] once it has strained against the world for too long.
    pub fn drive_carried(&mut self, eye: Vec3, view: View, spin: i8, dt: f32) {
        let Some(carried) = self.carry.body.as_mut() else {
            return;
        };
        let obstacles = obstacle_normals(&self.world, carried.handle);
        let Some(rb) = self.world.bodies.get_mut(carried.handle) else {
            self.release(ReleaseReason::BodyLost);
            return;
        };
        let camera_relative =
            (carried.camera_relative * carry::spin_step(carried.spin_axis, spin, dt)).normalize();
        let rotation = (view.rotation() * camera_relative).normalize();
        let origin = carry::carry_target(eye, view, carried.hold_distance);
        let target = origin + rotation * rb.local_center_of_mass();
        if !(target.is_finite() && rotation.is_finite()) {
            warn_once("physics: non-finite carry target", || {
                format!(
                    "physics: carry target is not finite (eye {eye:?}, view {view:?}); \
                     the carried body is let go"
                )
            });
            self.release(ReleaseReason::InvalidHold);
            return;
        }
        carried.camera_relative = camera_relative;
        let center = rb.center_of_mass();
        if carried.strain.update(center.distance(target), dt) {
            self.release(ReleaseReason::TornAway);
            return;
        }
        let servo = carry::servo_linvel(center, target, dt);
        rb.set_linvel(carry::limit_push(servo, &obstacles), true);
        clamp_linvel(rb, PLAYER_DRIVEN_MAX_LINVEL, true);
        rb.set_angvel(carry::servo_angvel(*rb.rotation(), rotation, dt), true);
        clamp_angvel(rb, MAX_DYN_ANGVEL, true);
    }

    /// Lets go of the carried body at `velocity`, bounded like every player-driven body.
    /// False when nothing is carried.
    pub fn throw_carried(&mut self, velocity: Vec3) -> bool {
        let Some(handle) = self.carried_handle() else {
            return false;
        };
        if let Some(rb) = self.world.bodies.get_mut(handle) {
            rb.set_linvel(velocity, true);
            clamp_linvel(rb, PLAYER_DRIVEN_MAX_LINVEL, true);
        }
        self.release(ReleaseReason::Thrown)
    }

    /// Releases the carried body when it is one of `entities`.
    pub(super) fn release_if_carried(&mut self, entities: &[EntityId], reason: ReleaseReason) {
        if self
            .carried_entity()
            .is_some_and(|entity| entities.contains(&entity))
        {
            self.release(reason);
        }
    }

    /// The one way to let go: the body keeps its velocity, gets its sleep thresholds back and
    /// `reason` is kept for [`take_release`](Self::take_release). False when nothing is carried.
    pub fn release(&mut self, reason: ReleaseReason) -> bool {
        let Some(carried) = self.carry.body.take() else {
            return false;
        };
        if let Some(rb) = self.world.bodies.get_mut(carried.handle) {
            carried.sleep.apply(rb);
        } else {
            tracing::error!(
                "physics: carried body {:?} is gone at its release ({reason:?}); nothing to restore",
                carried.entity
            );
        }
        self.carry.last_release = Some(reason);
        true
    }

    pub fn carried(&self) -> Option<CarriedInfo> {
        self.carry.body.as_ref().map(|carried| CarriedInfo {
            entity: carried.entity,
            mass: carried.mass,
        })
    }

    /// Why the body carried last was let go, once per release.
    pub fn take_release(&mut self) -> Option<ReleaseReason> {
        self.carry.last_release.take()
    }

    /// Whether the carried body is heavy enough to hold the player to walking speed.
    pub(crate) fn carry_forces_walk(&self) -> bool {
        self.carry
            .body
            .as_ref()
            .is_some_and(|carried| carry::forces_walk(carried.mass))
    }

    pub(super) fn carried_entity(&self) -> Option<EntityId> {
        self.carry.body.as_ref().map(|carried| carried.entity)
    }

    pub(super) fn carried_handle(&self) -> Option<RigidBodyHandle> {
        self.carry.body.as_ref().map(|carried| carried.handle)
    }

    /// Scene query for the player's own contacts: the carried body is never in the way.
    pub(in crate::sim) fn player_scene_filter(&self) -> QueryFilter<'static> {
        self.without_carried(scene_query_filter())
    }

    /// Dynamic-body query for the player's own contacts: the carried body is never shoved,
    /// never blocks and never supports the player.
    pub(in crate::sim) fn player_dynamic_filter(&self) -> QueryFilter<'static> {
        self.without_carried(dynamic_scene_query_filter())
    }

    fn without_carried(&self, filter: QueryFilter<'static>) -> QueryFilter<'static> {
        QueryFilter {
            exclude_rigid_body: self.carried_handle(),
            ..filter
        }
    }
}

/// Unit normals, out of the body of `handle` into what it touches, of its active contacts from
/// the last step with what cannot give way: static colliders, fixed and kinematic bodies.
fn obstacle_normals(world: &PhysicsWorld, handle: RigidBodyHandle) -> Vec<Vec3> {
    let Some(body) = world.bodies.get(handle) else {
        return Vec::new();
    };
    let immovable = |collider: ColliderHandle| {
        world
            .colliders
            .get(collider)
            .and_then(Collider::parent)
            .and_then(|parent| world.bodies.get(parent))
            .is_none_or(|parent| !parent.is_dynamic())
    };
    body.colliders()
        .iter()
        .flat_map(|&own| {
            world
                .contact_pairs_with(own)
                .filter_map(move |pair| {
                    let (other, outward) = if pair.collider1 == own {
                        (pair.collider2, 1.0)
                    } else {
                        (pair.collider1, -1.0)
                    };
                    immovable(other).then_some((pair, outward))
                })
                .flat_map(|(pair, outward)| {
                    pair.solver_manifolds()
                        .iter()
                        .filter(|manifold| !manifold.data.solver_contacts.is_empty())
                        .map(move |manifold| manifold.data.normal * outward)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::physics::test_support::{
        GROUND_CELL, bake, box_shape, ground_batch, object_group, placement, run_ticks,
        wake_context,
    };
    use crate::sim::refr_entity_id;

    const CRATE_PATH: &str = "meshes/crate.glb";
    const CRATE_REF: &str = "0A0001:Fallout4.esm";

    fn carrying_context() -> PhysicsContext {
        let mut batch = ground_batch();
        batch.objects.push(object_group(
            CRATE_PATH,
            vec![placement(CRATE_REF, Vec3::new(100.0, 0.5, -100.0))],
            true,
            Some(2.0),
        ));
        let baked = bake(&batch, &[(CRATE_PATH, vec![box_shape(0.5)])]);
        let mut context = wake_context();
        context.add_cell(GROUND_CELL, &batch, Some(&baked));
        context.set_physics_center(GROUND_CELL, 2);
        run_ticks(&mut context, 1);
        let view = View {
            yaw: 0.0,
            pitch: -0.5,
        };
        context
            .grab(
                refr_entity_id(CRATE_REF),
                Vec3::new(92.0, 5.0, -100.0),
                view,
            )
            .expect("grabbed");
        context
    }

    fn carried_body(context: &PhysicsContext) -> RigidBodyHandle {
        context.carried_handle().expect("carrying")
    }

    #[test]
    fn a_carried_body_quarantined_by_the_audit_is_released_as_lost() {
        let mut context = carrying_context();
        let handle = carried_body(&context);
        context.world.bodies[handle].set_linvel(Vec3::new(f32::NAN, 0.0, 0.0), true);
        assert_eq!(context.audit_before_step(), 1);
        assert_eq!(context.carried(), None);
        assert_eq!(context.take_release(), Some(ReleaseReason::BodyLost));
        assert!(context.player_dynamic_filter().exclude_rigid_body.is_none());
    }

    #[test]
    fn a_carried_body_that_vanished_is_released_as_lost_by_the_next_drive() {
        let mut context = carrying_context();
        let handle = carried_body(&context);
        context.world.remove_body(handle);
        let view = View {
            yaw: 0.0,
            pitch: 0.0,
        };
        context.drive_carried(Vec3::new(92.0, 5.0, -100.0), view, 0, crate::sim::FIXED_DT);
        assert_eq!(context.carried(), None);
        assert_eq!(context.take_release(), Some(ReleaseReason::BodyLost));
    }

    #[test]
    fn a_release_gives_the_body_its_sleep_thresholds_back() {
        let mut context = carrying_context();
        let handle = carried_body(&context);
        assert!(
            context.world.bodies[handle]
                .activation()
                .normalized_linear_threshold
                < 0.0
        );
        assert!(context.release(ReleaseReason::Dropped));
        let activation = context.world.bodies[handle].activation();
        assert!(activation.normalized_linear_threshold > 0.0);
        assert!(activation.angular_threshold > 0.0);
        assert!(
            !context.release(ReleaseReason::Dropped),
            "nothing left to release"
        );
    }
}
