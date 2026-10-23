//! Pre-step audit of the collider set: names and removes whatever would poison the broad phase.
//!
//! A full collider scan runs whenever the set changed (`dirty`); every step also scans the bodies,
//! the only things that move between changes. Offenders are removed, not repaired, so one bad
//! REFR can never take the pipeline down.

use super::{PhysicsContext, unpack_path, unpack_ref};
use crate::sim::collider_user_data::unpack_grid;
use crate::sim::sound_pose::{aabb_is_sound, pose_is_sound};
use glam::Vec3;
use rapier3d::prelude::*;

#[derive(Debug, Clone, PartialEq)]
enum Defect {
    ColliderPose(Pose),
    ColliderAabb { mins: Vec3, maxs: Vec3 },
    BodyPose(Pose),
    BodyVelocity { linvel: Vec3, angvel: Vec3 },
    KinematicTarget(Pose),
}

impl Defect {
    fn label(&self) -> &'static str {
        match self {
            Self::ColliderPose(_) => "collider pose",
            Self::ColliderAabb { .. } => "collider aabb",
            Self::BodyPose(_) => "body pose",
            Self::BodyVelocity { .. } => "body velocity",
            Self::KinematicTarget(_) => "kinematic target pose",
        }
    }

    fn values(&self) -> String {
        match self {
            Self::ColliderPose(pose) | Self::BodyPose(pose) | Self::KinematicTarget(pose) => {
                format!("t={:?} q={:?}", pose.translation, pose.rotation)
            }
            Self::ColliderAabb { mins, maxs } => format!("mins={mins:?} maxs={maxs:?}"),
            Self::BodyVelocity { linvel, angvel } => {
                format!("linvel={linvel:?} angvel={angvel:?}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Finding {
    collider: Option<ColliderHandle>,
    body: Option<RigidBodyHandle>,
    defect: Defect,
}

fn scan_colliders(world: &PhysicsWorld, findings: &mut Vec<Finding>) {
    for (handle, collider) in world.colliders.iter() {
        let body = collider.parent();
        let mut report = |defect| {
            findings.push(Finding {
                collider: Some(handle),
                body,
                defect,
            });
        };
        let pose_ok = pose_is_sound(collider.position())
            && collider.position_wrt_parent().is_none_or(pose_is_sound);
        if !pose_ok {
            report(Defect::ColliderPose(*collider.position()));
            continue;
        }
        let aabb = collider.compute_aabb();
        if !aabb_is_sound(&aabb) {
            report(Defect::ColliderAabb {
                mins: aabb.mins,
                maxs: aabb.maxs,
            });
        }
    }
}

fn scan_bodies(world: &PhysicsWorld, findings: &mut Vec<Finding>) {
    for (handle, body) in world.bodies.iter() {
        let mut report = |defect| {
            findings.push(Finding {
                collider: None,
                body: Some(handle),
                defect,
            });
        };
        if !pose_is_sound(body.position()) {
            report(Defect::BodyPose(*body.position()));
        } else if body.is_kinematic() && !pose_is_sound(body.next_position()) {
            report(Defect::KinematicTarget(*body.next_position()));
        } else if !(body.linvel().is_finite() && body.angvel().is_finite()) {
            report(Defect::BodyVelocity {
                linvel: body.linvel(),
                angvel: body.angvel(),
            });
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BodyKind {
    Static,
    Dynamic,
    Kinematic,
    Ragdoll,
    Player,
}

impl BodyKind {
    fn label(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Dynamic => "dynamic",
            Self::Kinematic => "kinematic",
            Self::Ragdoll => "ragdoll",
            Self::Player => "player",
        }
    }
}

impl PhysicsContext {
    /// Scans before a pipeline step and quarantines every offender; returns how many it removed.
    pub(super) fn audit_before_step(&mut self) -> usize {
        let mut findings = Vec::new();
        if self.dirty {
            scan_colliders(&self.world, &mut findings);
        }
        scan_bodies(&self.world, &mut findings);
        for finding in &findings {
            self.quarantine(finding);
        }
        findings.len()
    }

    fn body_kind(&self, body: Option<RigidBodyHandle>) -> BodyKind {
        let Some(handle) = body else {
            return BodyKind::Static;
        };
        if self
            .player_mirror
            .as_ref()
            .is_some_and(|m| m.body() == handle)
        {
            BodyKind::Player
        } else if self.ragdolls.owns_body(handle) {
            BodyKind::Ragdoll
        } else if self.dyn_bodies.values().any(|db| db.handle == handle) {
            BodyKind::Dynamic
        } else if self
            .world
            .bodies
            .get(handle)
            .is_some_and(RigidBody::is_kinematic)
        {
            BodyKind::Kinematic
        } else {
            BodyKind::Static
        }
    }

    /// Mesh path, REFR and cell the collider's `user_data` attributes it to.
    fn attribution(&self, finding: &Finding) -> String {
        let collider = finding.collider.or_else(|| {
            let body = self.world.bodies.get(finding.body?)?;
            body.colliders().first().copied()
        });
        let Some((handle, collider)) =
            collider.and_then(|handle| Some((handle, self.world.colliders.get(handle)?)))
        else {
            return "mesh=? ref=? cell=?".to_owned();
        };
        let user_data = collider.user_data;
        let mesh = if self.terrain_colliders.contains(&handle) {
            "terrain"
        } else {
            self.paths
                .get(unpack_path(user_data))
                .map_or("?", |(path, _)| path)
        };
        let (form_id, plugin_slot) = unpack_ref(user_data);
        let plugin = usize::try_from(plugin_slot)
            .ok()
            .and_then(|slot| slot.checked_sub(1))
            .and_then(|index| self.plugin_registry.get(index));
        let reference = plugin.map_or_else(
            || "?".to_owned(),
            |plugin| format!("{form_id:06X}:{plugin}"),
        );
        format!(
            "mesh={mesh} ref={reference} cell={:?}",
            unpack_grid(user_data)
        )
    }

    fn quarantine(&mut self, finding: &Finding) {
        let alive = finding
            .collider
            .is_some_and(|handle| self.world.colliders.get(handle).is_some())
            || finding
                .body
                .is_some_and(|handle| self.world.bodies.get(handle).is_some());
        if !alive {
            return;
        }
        let kind = self.body_kind(finding.body);
        tracing::warn!(
            "physics: non-finite collider quarantined: {} kind={} {} {}",
            finding.defect.label(),
            kind.label(),
            self.attribution(finding),
            finding.defect.values()
        );
        let Some(body) = finding.body else {
            if let Some(collider) = finding.collider {
                self.terrain_colliders.remove(&collider);
                self.world.remove_collider(collider);
            }
            self.dirty = true;
            return;
        };
        match kind {
            BodyKind::Ragdoll if self.ragdolls.quarantine_body(&mut self.world, body) => {}
            BodyKind::Dynamic => {
                self.dyn_bodies.retain(|_, db| db.handle != body);
                self.world.remove_body(body);
            }
            BodyKind::Player => {
                self.player_mirror = None;
                self.world.remove_body(body);
            }
            BodyKind::Static | BodyKind::Kinematic | BodyKind::Ragdoll => {
                self.world.remove_body(body);
            }
        }
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::super::test_support::{
        GROUND_CELL, bake, box_shape, ground_batch, object_group, placement, run_ticks,
        wake_context,
    };
    use super::*;

    const CRATE_PATH: &str = "meshes/crate.glb";
    const CRATE_REF: &str = "0A0001:Fallout4.esm";

    fn nan_x() -> Vec3 {
        Vec3::new(f32::NAN, 0.0, 0.0)
    }

    fn context_with_dynamic_crate() -> PhysicsContext {
        let mut batch = ground_batch();
        batch.objects.push(object_group(
            CRATE_PATH,
            vec![placement(CRATE_REF, Vec3::new(100.0, 3.0, -100.0))],
            true,
            Some(2.0),
        ));
        let baked = bake(&batch, &[(CRATE_PATH, vec![box_shape(0.5)])]);
        let mut context = wake_context();
        context.add_cell(GROUND_CELL, &batch, Some(&baked));
        context.set_physics_center(GROUND_CELL, 2);
        context
    }

    fn crate_handle(context: &PhysicsContext) -> RigidBodyHandle {
        context
            .dyn_bodies
            .values()
            .next()
            .expect("the crate")
            .handle
    }

    #[test]
    fn a_clean_world_is_left_alone() {
        let mut context = context_with_dynamic_crate();
        let colliders = context.collider_count();
        assert_eq!(context.audit_before_step(), 0);
        assert_eq!(context.collider_count(), colliders);
    }

    #[test]
    fn a_static_collider_with_a_nan_pose_is_removed_and_the_step_survives() {
        let mut context = context_with_dynamic_crate();
        let before = context.collider_count();
        context.world.insert_collider(
            ColliderBuilder::cuboid(1.0, 1.0, 1.0).translation(nan_x()),
            None,
        );
        context.dirty = true;
        run_ticks(&mut context, 3);
        assert_eq!(context.collider_count(), before);
    }

    #[test]
    fn a_dynamic_body_with_a_nan_velocity_is_quarantined_with_its_entity() {
        let mut context = context_with_dynamic_crate();
        let handle = crate_handle(&context);
        context
            .world
            .bodies
            .get_mut(handle)
            .unwrap()
            .set_linvel(nan_x(), true);
        assert_eq!(context.audit_before_step(), 1);
        assert!(context.world.bodies.get(handle).is_none());
        assert_eq!(context.dynamic_body_count(), 0);
        run_ticks(&mut context, 3);
    }

    #[test]
    fn a_kinematic_body_aimed_at_nan_is_removed_before_the_step_applies_it() {
        let mut context = context_with_dynamic_crate();
        let body = context
            .world
            .insert_body(RigidBodyBuilder::kinematic_position_based());
        context
            .world
            .insert_collider(ColliderBuilder::cuboid(1.0, 1.0, 1.0), Some(body));
        context
            .world
            .bodies
            .get_mut(body)
            .unwrap()
            .set_next_kinematic_position(Pose::from_translation(nan_x()));
        context.dirty = true;
        context.step();
        assert!(context.world.bodies.get(body).is_none());
    }

    #[test]
    fn a_non_finite_player_mirror_update_is_ignored() {
        let mut context = context_with_dynamic_crate();
        context.sync_player_mirror(nan_x(), &Capsule::new_y(1.0, 0.5), true);
        assert!(context.player_mirror.is_none());
        context.sync_player_mirror(Vec3::ZERO, &Capsule::new_y(1.0, 0.5), true);
        context.sync_player_mirror(nan_x(), &Capsule::new_y(1.0, 0.5), false);
        assert_eq!(context.audit_before_step(), 0);
    }

    #[test]
    fn a_poisoned_mirror_body_is_dropped_and_rebuilt_on_the_next_sync() {
        let mut context = context_with_dynamic_crate();
        context.sync_player_mirror(Vec3::ZERO, &Capsule::new_y(1.0, 0.5), true);
        let body = context.player_mirror.as_ref().unwrap().body();
        context
            .world
            .bodies
            .get_mut(body)
            .unwrap()
            .set_translation(nan_x(), true);
        assert_eq!(context.audit_before_step(), 1);
        assert!(context.player_mirror.is_none());
        context.sync_player_mirror(Vec3::ZERO, &Capsule::new_y(1.0, 0.5), true);
        assert!(context.player_mirror.is_some());
    }

    #[test]
    fn a_shape_with_a_nan_local_pose_is_dropped_from_the_placement() {
        let shapes = std::sync::Arc::new(vec![crate::sim::bake::CachedShape {
            shape: SharedShape::cuboid(1.0, 1.0, 1.0),
            local_pose: Pose::from_translation(nan_x()),
            joint: None,
            material: 0,
        }]);
        assert!(super::super::without_unsound_shapes("x", shapes).is_empty());
    }
}
