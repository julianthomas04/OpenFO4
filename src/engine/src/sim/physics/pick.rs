//! Aiming with a radius. The original picks with a volume around the crosshair line, not with the
//! line alone: measured, the rollover of an item shows up to the pick radius beside its outline.
//! This module composes the exact ray with a ball swept along it.
//!
//! The ball only looks for placed items. Everything else, a load door, a container, any other
//! activator, keeps the exact ray: the original was measured on an item only. The ball does not
//! extend the reach: what it touches counts only when the touched point is within reach of the
//! eye, as for the exact ray.

use std::cell::RefCell;
use std::collections::HashMap;

use glam::Vec3;
use rapier3d::parry::query::details::ShapeCastOptions;
use rapier3d::prelude::*;

use super::{PhysicsContext, RayProbe};
use crate::sim::RayHit;
use crate::sim::collider_user_data::unpack_ref;
use crate::sim::mirror::scene_query_filter;
use crate::utils::warn_once;

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

impl PickRay {
    fn has_a_direction(&self) -> bool {
        self.origin.is_finite() && self.dir.is_finite() && self.dir.length_squared() > 0.0
    }
}

/// An item collider the ball touched, and where.
struct ItemTouch {
    handle: ColliderHandle,
    point: Vec3,
}

/// The class of the ref a collider belongs to, asked of the caller once per ref and without
/// building a key per collider: the key is written into one reused buffer.
struct RefClasses<'a> {
    physics: &'a PhysicsContext,
    classify: &'a dyn Fn(&str) -> PickClass,
    known: RefCell<HashMap<(u32, u32), PickClass>>,
    key: RefCell<String>,
}

impl<'a> RefClasses<'a> {
    fn new(physics: &'a PhysicsContext, classify: &'a dyn Fn(&str) -> PickClass) -> Self {
        Self {
            physics,
            classify,
            known: RefCell::default(),
            key: RefCell::default(),
        }
    }

    /// A collider without a ref is solid geometry, such as the terrain.
    fn of(&self, user_data: u128) -> PickClass {
        let ref_key = unpack_ref(user_data);
        if let Some(&class) = self.known.borrow().get(&ref_key) {
            return class;
        }
        let mut key = self.key.borrow_mut();
        key.clear();
        let class = if self.physics.write_ref_id(user_data, &mut key) {
            (self.classify)(&key)
        } else {
            PickClass::Solid
        };
        self.known.borrow_mut().insert(ref_key, class);
        class
    }
}

impl PhysicsContext {
    /// What the player aims at. The exact ray decides when it hits an item or an activator.
    /// Otherwise a ball swept along the ray looks for items: the ball centre travels no farther
    /// than the reach or the exact ray's solid hit, whichever is nearer, so it never passes
    /// through what the crosshair line stops at. Of the items the ball touches, the one met first
    /// along the ray counts when its touched point is in front of the eye, within reach, and
    /// seen from the eye: no solid, activator or terrain lies on the line from the eye to that
    /// point. Items and the carried body never block that line. When no item counts, the exact
    /// ray's hit is the aim.
    ///
    /// Costs the exact ray plus, only when the ray hit neither an item nor an activator, one ball
    /// sweep restricted to items per candidate tried (normally one), each with a visibility ray.
    pub fn pick_attribution(
        &self,
        aim: PickRay,
        classify: impl Fn(&str) -> PickClass,
    ) -> Option<RayHit> {
        if !aim.has_a_direction() {
            warn_once("pick-aim-without-direction", || {
                format!("pick: the aim {aim:?} has no usable origin and direction; it hits nothing")
            });
            return None;
        }
        let exact = self.probe(aim.origin, aim.dir, aim.reach, self.pick_filter());
        let exact_decides = aim.radius <= 0.0
            || exact
                .as_ref()
                .and_then(|probe| probe.ref_id.as_deref())
                .is_some_and(|ref_id| classify(ref_id) != PickClass::Solid);
        if exact_decides {
            return exact.map(RayProbe::into_hit);
        }
        let ball_travel = exact
            .as_ref()
            .map_or(aim.reach, |solid| solid.dist.min(aim.reach));
        let classes = RefClasses::new(self, &classify);
        self.first_counted_item(aim, ball_travel, &classes)
            .or_else(|| exact.map(RayProbe::into_hit))
    }

    /// The query filter of every pick query: the carried body is no part of the world to the
    /// aim, it must neither pick nor hide anything.
    fn pick_filter(&self) -> QueryFilter<'_> {
        QueryFilter {
            exclude_rigid_body: self.carried_handle(),
            ..scene_query_filter()
        }
    }

    /// The first item along the ray that counts, trying the next one when the nearest does not.
    fn first_counted_item(
        &self,
        aim: PickRay,
        ball_travel: f32,
        classes: &RefClasses<'_>,
    ) -> Option<RayHit> {
        let mut rejected = Vec::new();
        loop {
            let touch = self.nearest_item_touch(aim, ball_travel, classes, &rejected)?;
            if self.touch_counts(aim, touch.point, classes) {
                let eye_distance = aim.origin.distance(touch.point);
                return self
                    .attribute(touch.handle, eye_distance)
                    .map(RayProbe::into_hit);
            }
            rejected.push(touch.handle);
        }
    }

    /// The item the ball, centre travelling `ball_travel` along the ray, meets first. A ball that
    /// starts inside an item touches it at distance 0.
    fn nearest_item_touch(
        &self,
        aim: PickRay,
        ball_travel: f32,
        classes: &RefClasses<'_>,
        rejected: &[ColliderHandle],
    ) -> Option<ItemTouch> {
        let is_candidate = |handle: ColliderHandle, collider: &Collider| {
            !rejected.contains(&handle) && classes.of(collider.user_data) == PickClass::Item
        };
        let filter = self.pick_filter().predicate(&is_candidate);
        let (handle, hit) = self.world.query_pipeline_with_filter(filter).cast_shape(
            &Pose::from_translation(aim.origin),
            aim.dir,
            &Ball::new(aim.radius),
            ShapeCastOptions {
                target_distance: 0.0,
                stop_at_penetration: true,
                max_time_of_impact: ball_travel,
                compute_impact_geometry_on_penetration: true,
            },
        )?;
        let touched_point = hit.witness1;
        Some(ItemTouch {
            handle,
            point: touched_point,
        })
    }

    fn touch_counts(&self, aim: PickRay, touched_point: Vec3, classes: &RefClasses<'_>) -> bool {
        let to_touched = touched_point - aim.origin;
        let distance = to_touched.length();
        to_touched.dot(aim.dir) > 0.0
            && distance <= aim.reach
            && self.is_seen_from(aim.origin, to_touched / distance, distance, classes)
    }

    /// Whether nothing but items lies on the line from `origin` along `dir` for `distance`.
    fn is_seen_from(
        &self,
        origin: Vec3,
        dir: Vec3,
        distance: f32,
        classes: &RefClasses<'_>,
    ) -> bool {
        let is_obstacle = |_: ColliderHandle, collider: &Collider| {
            classes.of(collider.user_data) != PickClass::Item
        };
        let filter = self.pick_filter().predicate(&is_obstacle);
        self.world
            .cast_ray(&Ray::new(origin, dir), distance, true, filter)
            .is_none()
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
    const THIN_WALL_PATH: &str = "meshes\\thin_wall.glb";
    const ITEM_HALF: f32 = 0.5;
    const WALL_HALF: f32 = 3.0;
    const THIN_WALL_HALF: f32 = 0.5;
    const RADIUS: f32 = 2.0;
    const REACH: f32 = 15.0;
    const EYE: Vec3 = Vec3::new(100.0, 5.0, -100.0);

    const ON_AXIS: &str = "0A0001:Fallout4.esm";
    const BESIDE: &str = "0A0002:Fallout4.esm";
    const FAR_BESIDE: &str = "0A0003:Fallout4.esm";
    const NEAR_ITEM: &str = "0A0004:Fallout4.esm";
    const BEHIND_EYE: &str = "0A0005:Fallout4.esm";
    const OTHER_SIDE: &str = "0A0006:Fallout4.esm";
    const WALL: &str = "0B0001:Fallout4.esm";
    const THIN_WALL: &str = "0B0002:Fallout4.esm";
    const DOOR: &str = "0C0001:Fallout4.esm";

    fn classify(ref_id: &str) -> PickClass {
        match ref_id {
            WALL | THIN_WALL => PickClass::Solid,
            DOOR => PickClass::Activator,
            _ => PickClass::Item,
        }
    }

    fn at(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3::new(x, y, z)
    }

    /// Items stand in the world as pinned bodies, walls and doors as static boxes, on the ground.
    fn world(items: &[(&str, Vec3)], walls: &[Vec3], doors: &[Vec3]) -> PhysicsContext {
        world_with_thin_walls(items, walls, &[], doors)
    }

    fn world_with_thin_walls(
        items: &[(&str, Vec3)],
        walls: &[Vec3],
        thin_walls: &[Vec3],
        doors: &[Vec3],
    ) -> PhysicsContext {
        let mut batch = ground_batch();
        if !items.is_empty() {
            batch.objects.push(pinned_group(
                ITEM_PATH,
                items.iter().map(|(r, p)| placement(r, *p)).collect(),
            ));
        }
        for (path, ref_id, positions) in [
            (WALL_PATH, WALL, walls),
            (THIN_WALL_PATH, THIN_WALL, thin_walls),
            (DOOR_PATH, DOOR, doors),
        ] {
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
                (THIN_WALL_PATH, vec![box_shape(THIN_WALL_HALF)]),
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
        aim_with_reach(context, dir, radius, REACH)
    }

    fn aim_with_reach(
        context: &PhysicsContext,
        dir: Vec3,
        radius: f32,
        reach: f32,
    ) -> Option<RayHit> {
        context.pick_attribution(
            PickRay {
                origin: EYE,
                dir: dir.normalize(),
                reach,
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
    fn an_item_overlapping_the_start_of_the_ball_in_front_is_aimed_at_and_one_past_the_reach_is_not()
     {
        let context = world(&[(BESIDE, at(101.5, 5.0, -101.0))], &[], &[]);
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
    fn an_item_touched_exactly_at_the_reach_is_aimed_at_and_one_just_beyond_is_not() {
        let context = world(&[(BESIDE, at(110.0, 5.0, -101.7))], &[], &[]);
        let touched_at = aim(&context, Vec3::X, RADIUS).expect("a hit").dist;
        assert_eq!(
            aimed_ref(aim_with_reach(&context, Vec3::X, RADIUS, touched_at)),
            Some(BESIDE.to_owned())
        );
        let just_short = f32::from_bits(touched_at.to_bits() - 1);
        assert_eq!(
            aimed_ref(aim_with_reach(&context, Vec3::X, RADIUS, just_short)),
            None
        );
    }

    #[test]
    fn the_distance_of_an_item_the_ball_touches_is_from_the_eye_to_the_touched_point() {
        let item_face_x = 109.5;
        let context = world(&[(BESIDE, at(110.0, 5.0, -101.7))], &[], &[]);
        let hit = aim(&context, Vec3::X, RADIUS).expect("a hit");
        assert!(
            hit.dist >= item_face_x - EYE.x && hit.dist < REACH,
            "{}",
            hit.dist
        );
    }

    #[test]
    fn a_solid_thinner_than_the_radius_in_front_of_an_item_hides_it() {
        let item = (BESIDE, at(104.5, 5.0, -101.0));
        let open = world(&[item], &[], &[]);
        assert_eq!(
            aimed_ref(aim(&open, Vec3::X, RADIUS)),
            Some(BESIDE.to_owned())
        );
        let behind_a_pane = world_with_thin_walls(&[item], &[], &[at(103.0, 5.0, -100.0)], &[]);
        assert_eq!(
            aimed_ref(aim(&behind_a_pane, Vec3::X, RADIUS)),
            Some(THIN_WALL.to_owned())
        );
    }

    #[test]
    fn an_item_hidden_behind_a_thin_solid_does_not_hide_a_visible_item_the_ball_touches_later() {
        let context = world_with_thin_walls(
            &[
                (BESIDE, at(104.5, 5.0, -101.0)),
                (OTHER_SIDE, at(104.5, 5.0, -98.4)),
            ],
            &[],
            &[at(103.0, 5.0, -100.0)],
            &[],
        );
        assert_eq!(
            aimed_ref(aim(&context, Vec3::X, RADIUS)),
            Some(OTHER_SIDE.to_owned())
        );
    }

    #[test]
    fn an_item_beside_the_wall_the_ray_hits_is_aimed_at_when_the_eye_sees_it() {
        let wall_edge_at_z = -100.5;
        let wall = at(106.0, 5.0, wall_edge_at_z + WALL_HALF);
        let context = world(&[(BESIDE, at(102.5, 5.0, -102.0))], &[wall], &[]);
        assert_eq!(
            aimed_ref(aim(&context, Vec3::X, RADIUS)),
            Some(BESIDE.to_owned())
        );
    }

    #[test]
    fn the_ball_never_travels_past_the_solid_the_ray_hits() {
        let wall_edge_at_z = -100.5;
        let wall = at(106.0, 5.0, wall_edge_at_z + WALL_HALF);
        let beside_the_wall_beyond_its_face = at(105.5, 5.0, -102.0);
        let context = world(&[(BESIDE, beside_the_wall_beyond_its_face)], &[wall], &[]);
        assert_eq!(
            aimed_ref(aim(&context, Vec3::X, RADIUS)),
            Some(WALL.to_owned())
        );
    }

    #[test]
    fn an_item_overlapping_the_start_beside_or_behind_the_eye_never_counts() {
        let behind = (BEHIND_EYE, at(98.5, 5.0, -101.0));
        let in_front = (NEAR_ITEM, at(105.0, 5.0, -101.8));
        let both = world(&[behind, in_front], &[], &[]);
        assert_eq!(
            aimed_ref(aim(&both, Vec3::X, RADIUS)),
            Some(NEAR_ITEM.to_owned())
        );
        let alone = world(&[behind], &[], &[]);
        assert_eq!(aimed_ref(aim(&alone, Vec3::X, RADIUS)), None);
    }

    #[test]
    fn an_aim_without_a_usable_direction_hits_nothing() {
        let context = world(&[(BESIDE, at(101.5, 5.0, -101.0))], &[], &[]);
        for dir in [Vec3::ZERO, Vec3::NAN, Vec3::INFINITY] {
            let hit = context.pick_attribution(
                PickRay {
                    origin: EYE,
                    dir,
                    reach: REACH,
                    radius: RADIUS,
                },
                classify,
            );
            assert!(hit.is_none(), "{dir}");
        }
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
        let ball_radius = 1.6;
        let aimed = |context: &PhysicsContext| aimed_ref(aim(context, Vec3::X, ball_radius));
        assert_eq!(aimed(&rounded_item_context(0.4)), Some(BESIDE.to_owned()));
        assert_eq!(aimed(&rounded_item_context(0.0)), None);
    }
}
