//! Aiming with a radius. The original picks with a volume around the crosshair line, not with the
//! line alone: measured, the rollover of an item shows up to the pick radius beside its outline.
//! This module composes the exact ray with a ball swept along it.
//!
//! The ball only looks for placed items. Everything else, a load door, a container, any other
//! activator, keeps the exact ray: the original was measured on an item only. The ball does not
//! extend the reach: what it touches counts only when the touched point is within reach of the
//! eye, as for the exact ray.

use glam::Vec3;
use rapier3d::parry::query::details::ShapeCastOptions;
use rapier3d::prelude::*;

use super::{PhysicsContext, RayProbe};
use crate::sim::RayHit;
use crate::sim::mirror::scene_query_filter;

/// What a collider is to the pick, as the caller (which knows items and activators) sees it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PickClass {
    /// A placed item: the ball finds it.
    Item,
    /// A door or an animated object: only the exact ray finds it, and it wins over any item.
    Activator,
    /// Anything else: only the exact ray finds it, and it hides items behind it.
    Solid,
}

/// The aim: a ray of `reach` from `origin` along the unit vector `dir`, widened to a ball of
/// `radius` for items. A `radius` of 0 is the exact ray.
#[derive(Copy, Clone, Debug)]
pub struct PickRay {
    pub origin: Vec3,
    pub dir: Vec3,
    pub reach: f32,
    pub radius: f32,
}

impl PhysicsContext {
    /// What the player aims at. The exact ray decides when it hits an item or an activator.
    /// Otherwise the ball swept along the ray finds the first item it touches within reach (the
    /// nearest along the ray: the one it meets first, wherever the other candidates sit around
    /// the axis), unless the exact ray hits solid geometry in front of that item: nearer than the
    /// item's nearest possible point, one radius before the ball's first touch. Within that
    /// radius a surface counts as the item's own support (a table top, a shelf, the floor), so
    /// an item resting on it stays aimed at when the crosshair lands just short of it.
    ///
    /// Costs the exact ray plus, only when the ray hit neither an item nor an activator, one
    /// ball sweep restricted to items.
    pub fn pick_attribution(
        &self,
        aim: PickRay,
        classify: impl Fn(&str) -> PickClass,
    ) -> Option<RayHit> {
        let exact = self.probe(aim.origin, aim.dir, aim.reach, scene_query_filter());
        let exact_decides = aim.radius <= 0.0
            || exact
                .as_ref()
                .and_then(|probe| probe.ref_id.as_deref())
                .is_some_and(|ref_id| classify(ref_id) != PickClass::Solid);
        if exact_decides {
            return exact.map(RayProbe::into_hit);
        }
        let swept = self.swept_item(aim, &classify);
        let item_is_visible = |item: &RayProbe| {
            exact
                .as_ref()
                .is_none_or(|solid| solid.dist >= item.dist - aim.radius)
        };
        match swept {
            Some(item) if item_is_visible(&item) => Some(item.into_hit()),
            _ => exact.map(RayProbe::into_hit),
        }
    }

    /// The first item the ball touches along the ray. The carried body is no item: it must not
    /// pick itself. A ball that starts inside an item touches it at distance 0.
    fn swept_item(&self, aim: PickRay, classify: &impl Fn(&str) -> PickClass) -> Option<RayProbe> {
        let is_item = |_: ColliderHandle, collider: &Collider| {
            self.ref_id_of(collider.user_data)
                .is_some_and(|ref_id| classify(&ref_id) == PickClass::Item)
        };
        let filter = QueryFilter {
            exclude_rigid_body: self.carried_handle(),
            ..scene_query_filter()
        }
        .predicate(&is_item);
        let (handle, hit) = self.world.query_pipeline_with_filter(filter).cast_shape(
            &Pose::from_translation(aim.origin),
            aim.dir,
            &Ball::new(aim.radius),
            ShapeCastOptions {
                target_distance: 0.0,
                stop_at_penetration: true,
                max_time_of_impact: aim.reach,
                compute_impact_geometry_on_penetration: true,
            },
        )?;
        let touched_within_reach = aim.origin.distance(hit.witness1) <= aim.reach;
        touched_within_reach
            .then(|| self.attribute(handle, hit.time_of_impact))
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::carry::View;
    use crate::sim::collision::{ColShape, PartShape};
    use crate::sim::physics::test_support::{
        GROUND_CELL, bake, box_shape, ground_batch, object_group, pinned_group, placement,
        run_ticks, wake_context,
    };
    use crate::sim::refr_entity_id;

    const ITEM_PATH: &str = "meshes\\item.glb";
    const WALL_PATH: &str = "meshes\\wall.glb";
    const DOOR_PATH: &str = "meshes\\door.glb";
    const ITEM_HALF: f32 = 0.5;
    const WALL_HALF: f32 = 3.0;
    const RADIUS: f32 = 2.0;
    const REACH: f32 = 15.0;
    const EYE: Vec3 = Vec3::new(100.0, 5.0, -100.0);

    const ON_AXIS: &str = "0A0001:Fallout4.esm";
    const BESIDE: &str = "0A0002:Fallout4.esm";
    const FAR_BESIDE: &str = "0A0003:Fallout4.esm";
    const NEAR_ITEM: &str = "0A0004:Fallout4.esm";
    const WALL: &str = "0B0001:Fallout4.esm";
    const DOOR: &str = "0C0001:Fallout4.esm";

    fn classify(ref_id: &str) -> PickClass {
        match ref_id {
            WALL => PickClass::Solid,
            DOOR => PickClass::Activator,
            _ => PickClass::Item,
        }
    }

    fn at(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3::new(x, y, z)
    }

    /// Items stand in the world as pinned bodies, walls and doors as static boxes, on the ground.
    fn world(items: &[(&str, Vec3)], walls: &[Vec3], doors: &[Vec3]) -> PhysicsContext {
        let mut batch = ground_batch();
        if !items.is_empty() {
            batch.objects.push(pinned_group(
                ITEM_PATH,
                items.iter().map(|(r, p)| placement(r, *p)).collect(),
            ));
        }
        for (path, ref_id, positions) in [(WALL_PATH, WALL, walls), (DOOR_PATH, DOOR, doors)] {
            if !positions.is_empty() {
                let placements = positions.iter().map(|p| placement(ref_id, *p)).collect();
                batch
                    .objects
                    .push(object_group(path, placements, false, None));
            }
        }
        let baked = bake(
            &batch,
            &[
                (ITEM_PATH, vec![box_shape(ITEM_HALF)]),
                (WALL_PATH, vec![box_shape(WALL_HALF)]),
                (DOOR_PATH, vec![box_shape(ITEM_HALF)]),
            ],
        );
        let mut context = wake_context();
        context.add_cell(GROUND_CELL, &batch, Some(&baked));
        context.set_physics_center(GROUND_CELL, 2);
        run_ticks(&mut context, 1);
        context
    }

    fn aim(context: &PhysicsContext, dir: Vec3, radius: f32) -> Option<RayHit> {
        context.pick_attribution(
            PickRay {
                origin: EYE,
                dir: dir.normalize(),
                reach: REACH,
                radius,
            },
            classify,
        )
    }

    fn aimed_ref(hit: Option<RayHit>) -> Option<String> {
        hit?.ref_id
    }

    #[test]
    fn an_item_beside_the_ray_within_the_radius_is_aimed_at_and_one_outside_it_is_not() {
        let context = world(
            &[
                (BESIDE, at(108.0, 5.0, -102.2)),
                (FAR_BESIDE, at(108.0, 5.0, -90.0)),
            ],
            &[],
            &[],
        );
        assert_eq!(
            aimed_ref(aim(&context, Vec3::X, RADIUS)),
            Some(BESIDE.to_owned())
        );
        let context = world(&[(FAR_BESIDE, at(108.0, 5.0, -104.0))], &[], &[]);
        assert_eq!(aimed_ref(aim(&context, Vec3::X, RADIUS)), None);
    }

    #[test]
    fn a_radius_of_zero_gives_the_exact_ray_result_hit_or_miss() {
        let context = world(
            &[
                (ON_AXIS, at(110.0, 5.0, -100.0)),
                (BESIDE, at(108.0, 5.0, -102.2)),
            ],
            &[],
            &[],
        );
        for dir in [
            Vec3::X,
            Vec3::new(1.0, 0.0, -0.3),
            Vec3::new(1.0, -0.4, 0.0),
        ] {
            let exact = context.ray_attribution(EYE, dir.normalize(), REACH);
            let picked = aim(&context, dir, 0.0);
            assert_eq!(
                picked.as_ref().map(|hit| (&hit.ref_id, hit.dist.to_bits())),
                exact.as_ref().map(|hit| (&hit.ref_id, hit.dist.to_bits())),
                "{dir}"
            );
        }
    }

    #[test]
    fn an_item_on_the_exact_ray_beats_a_nearer_item_the_ball_touches() {
        let context = world(
            &[
                (ON_AXIS, at(110.0, 5.0, -100.0)),
                (NEAR_ITEM, at(105.0, 5.0, -101.8)),
            ],
            &[],
            &[],
        );
        assert_eq!(
            aimed_ref(aim(&context, Vec3::X, RADIUS)),
            Some(ON_AXIS.to_owned())
        );
    }

    #[test]
    fn of_two_items_in_the_ball_the_one_met_first_along_the_ray_is_aimed_at() {
        let context = world(
            &[
                (NEAR_ITEM, at(106.0, 5.0, -101.9)),
                (BESIDE, at(109.0, 5.0, -100.9)),
            ],
            &[],
            &[],
        );
        assert_eq!(
            aimed_ref(aim(&context, Vec3::X, RADIUS)),
            Some(NEAR_ITEM.to_owned())
        );
    }

    #[test]
    fn a_wall_in_front_hides_an_item_behind_it_and_the_wall_stays_the_aim() {
        let items = [(BESIDE, at(112.0, 5.0, -101.5))];
        let open = world(&items, &[], &[]);
        assert_eq!(
            aimed_ref(aim(&open, Vec3::X, RADIUS)),
            Some(BESIDE.to_owned())
        );
        let walled = world(&items, &[at(106.0, 5.0, -100.0)], &[]);
        assert_eq!(
            aimed_ref(aim(&walled, Vec3::X, RADIUS)),
            Some(WALL.to_owned())
        );
    }

    #[test]
    fn a_table_top_between_the_eye_and_an_item_behind_its_edge_hides_the_item() {
        let table = at(106.0, 1.5, -100.0);
        let item = (BESIDE, at(112.0, 3.5, -101.5));
        let context = world(&[item], &[table], &[]);
        let skimming = Vec3::new(1.0, -0.1, 0.0);
        assert_eq!(
            aimed_ref(aim(&context, skimming, RADIUS)),
            Some(WALL.to_owned())
        );
    }

    #[test]
    fn an_item_resting_on_the_floor_stays_aimed_at_when_the_crosshair_lands_just_short_of_it() {
        let context = world(&[(BESIDE, at(110.0, 0.5, -100.0))], &[], &[]);
        let short_of_it = Vec3::new(8.0, -5.0, 0.0);
        let hit = aim(&context, short_of_it, RADIUS).expect("a hit");
        assert_eq!(hit.ref_id, Some(BESIDE.to_owned()));
        let far_short = Vec3::new(3.0, -5.0, 0.0);
        let hit = aim(&context, far_short, RADIUS).expect("a hit");
        assert_eq!(hit.mesh_path, "terrain");
    }

    #[test]
    fn a_door_hit_by_the_exact_ray_wins_over_an_item_the_ball_touches() {
        let context = world(
            &[(NEAR_ITEM, at(105.0, 5.0, -101.8))],
            &[],
            &[at(110.0, 5.0, -100.0)],
        );
        assert_eq!(
            aimed_ref(aim(&context, Vec3::X, RADIUS)),
            Some(DOOR.to_owned())
        );
    }

    #[test]
    fn a_door_beside_the_ray_is_never_aimed_at_through_the_radius() {
        let context = world(&[], &[], &[at(108.0, 5.0, -102.2)]);
        assert_eq!(aimed_ref(aim(&context, Vec3::X, RADIUS)), None);
    }

    #[test]
    fn an_item_touching_the_start_of_the_ball_is_aimed_at_and_one_past_the_reach_is_not() {
        let context = world(&[(BESIDE, at(100.5, 5.0, -101.0))], &[], &[]);
        assert_eq!(
            aimed_ref(aim(&context, Vec3::X, RADIUS)),
            Some(BESIDE.to_owned())
        );
        let context = world(&[(BESIDE, at(130.0, 5.0, -101.0))], &[], &[]);
        assert_eq!(aimed_ref(aim(&context, Vec3::X, RADIUS)), None);
    }

    #[test]
    fn the_ball_does_not_extend_the_reach_along_the_aim() {
        let touched_inside = world(&[(BESIDE, at(115.2, 5.0, -101.7))], &[], &[]);
        assert_eq!(
            aimed_ref(aim(&touched_inside, Vec3::X, RADIUS)),
            Some(BESIDE.to_owned())
        );
        let touched_beyond = world(&[(BESIDE, at(116.2, 5.0, -101.7))], &[], &[]);
        assert_eq!(aimed_ref(aim(&touched_beyond, Vec3::X, RADIUS)), None);
    }

    #[test]
    fn an_eye_inside_a_wall_aims_at_the_wall_not_at_an_item_behind_it() {
        let context = world(
            &[(BESIDE, at(110.0, 5.0, -101.5))],
            &[at(100.0, 5.0, -100.0)],
            &[],
        );
        assert_eq!(
            aimed_ref(aim(&context, Vec3::X, RADIUS)),
            Some(WALL.to_owned())
        );
    }

    const CRATE_PATH: &str = "meshes\\crate.glb";
    const CRATE_REF: &str = "0D0001:Fallout4.esm";

    fn crate_context(carried: bool) -> PhysicsContext {
        let mut batch = ground_batch();
        batch.objects.push(object_group(
            CRATE_PATH,
            vec![placement(CRATE_REF, at(100.0, 0.5, -90.0))],
            true,
            Some(2.0),
        ));
        let baked = bake(&batch, &[(CRATE_PATH, vec![box_shape(ITEM_HALF)])]);
        let mut context = wake_context();
        context.add_cell(GROUND_CELL, &batch, Some(&baked));
        context.set_physics_center(GROUND_CELL, 2);
        run_ticks(&mut context, 1);
        if carried {
            let view = View {
                yaw: 0.0,
                pitch: -0.5,
            };
            context
                .grab(refr_entity_id(CRATE_REF), at(94.0, 3.0, -90.0), view)
                .expect("grabbed");
        }
        context
    }

    #[test]
    fn the_carried_body_is_no_item_to_the_ball() {
        let sweep = |context: &PhysicsContext| {
            context.pick_attribution(
                PickRay {
                    origin: at(94.0, 2.4, -90.0),
                    dir: Vec3::X,
                    reach: REACH,
                    radius: RADIUS,
                },
                |_| PickClass::Item,
            )
        };
        assert_eq!(
            aimed_ref(sweep(&crate_context(false))),
            Some(CRATE_REF.to_owned())
        );
        assert_eq!(aimed_ref(sweep(&crate_context(true))), None);
    }

    fn rounded_item_context(radius: f32) -> PhysicsContext {
        let corners: Vec<Vec3> = [-1.0_f32, 1.0]
            .into_iter()
            .flat_map(|x| {
                [-1.0_f32, 1.0]
                    .into_iter()
                    .flat_map(move |y| [-1.0_f32, 1.0].map(move |z| Vec3::new(x, y, z) * 0.3))
            })
            .collect();
        let mut batch = ground_batch();
        batch.objects.push(pinned_group(
            ITEM_PATH,
            vec![placement(BESIDE, at(108.0, 5.0, -102.2))],
        ));
        let shape = PartShape {
            shape: ColShape::Convex {
                center: Vec3::ZERO,
                rotation: glam::Quat::IDENTITY,
                vertices: corners,
                radius,
            },
            node: None,
            joint: None,
            material: 0,
        };
        let baked = bake(&batch, &[(ITEM_PATH, vec![shape])]);
        let mut context = wake_context();
        context.add_cell(GROUND_CELL, &batch, Some(&baked));
        context.set_physics_center(GROUND_CELL, 2);
        run_ticks(&mut context, 1);
        context
    }

    #[test]
    fn the_ball_touches_the_grown_surface_of_a_rounded_hull_and_not_the_bare_one() {
        let reach_of_ball = 1.6;
        let aimed = |context: &PhysicsContext| aimed_ref(aim(context, Vec3::X, reach_of_ball));
        assert_eq!(aimed(&rounded_item_context(0.4)), Some(BESIDE.to_owned()));
        assert_eq!(aimed(&rounded_item_context(0.0)), None);
    }
}
