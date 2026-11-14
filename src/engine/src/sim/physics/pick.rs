//! Aiming with a radius. The original picks with a volume around the crosshair line, not with the
//! line alone: measured, the rollover of an item or a door shows up to the pick radius beside the
//! outline of its collision.
//!
//! The exact ray decides when it hits an item or an activator. Otherwise every item and activator
//! the ball swept along the ray touches is a candidate when its touched point is in front of the
//! eye, within reach and seen from the eye, and the candidate whose touched point lies nearest
//! the ray wins (measured on item pairs at two depths and a door end, 2026-11-13/14). Solids never
//! stop the ball: they only hide from the eye what lies behind them.

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

const NO_USABLE_AIM: &str = "pick-aim-without-direction";

/// What a collider is to the pick, as the caller (which knows items and activators) sees it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PickClass {
    /// A placed item: the ball finds it.
    Item,
    /// A door or an animated object: the ball finds it like an item, and it hides what lies
    /// behind it from the eye.
    Activator,
    /// Anything else: the ball never finds it, and it hides what lies behind it from the eye.
    Solid,
}

impl PickClass {
    fn is_pickable(self) -> bool {
        match self {
            Self::Item | Self::Activator => true,
            Self::Solid => false,
        }
    }
}

/// The aim: a ray of `reach` from `origin` along `dir` (any length above 0), widened to a ball of
/// `radius` for items and activators. A `radius` of 0 is the exact ray. The `reach` and `radius`
/// come from `PickSettings`, which guarantees a finite reach above 0 and a finite radius of 0 or
/// more; `origin` and `dir` come from the camera and are checked on entry.
#[derive(Copy, Clone, Debug)]
pub struct PickRay {
    pub origin: Vec3,
    pub dir: Vec3,
    pub reach: f32,
    pub radius: f32,
}

/// A [`PickRay`] that passed [`PickRay::validated`]: a finite origin and a unit direction.
#[derive(Copy, Clone, Debug)]
struct ValidAim {
    origin: Vec3,
    dir: Vec3,
    reach: f32,
    radius: f32,
}

impl PickRay {
    /// The one entry check of an aim: `None` for a non-finite origin or direction and for a zero
    /// direction. The direction is normalised here and nowhere else.
    fn validated(self) -> Option<ValidAim> {
        debug_assert!(
            self.reach.is_finite() && self.reach > 0.0,
            "the reach of {self:?} breaks the guarantee of PickSettings"
        );
        debug_assert!(
            self.radius.is_finite() && self.radius >= 0.0,
            "the radius of {self:?} breaks the guarantee of PickSettings"
        );
        let dir = self.dir.try_normalize()?;
        self.origin.is_finite().then_some(ValidAim {
            origin: self.origin,
            dir,
            reach: self.reach,
            radius: self.radius,
        })
    }
}

impl ValidAim {
    /// Where the ball centre stops: the reach along the aim.
    fn far_end(&self) -> Vec3 {
        self.origin + self.dir * self.reach
    }

    /// How far a touched point is from the eye for the reach: straight from the eye. Measured,
    /// the far item of a pair is lost where its touched point passes the reach from the eye, not
    /// along the aim (2026-11-14).
    fn reach_distance(&self, point: Vec3) -> f32 {
        self.origin.distance(point)
    }

    /// How far `point` lies from the line of the aim.
    fn offset_from_aim(&self, point: Vec3) -> f32 {
        (point - self.origin).cross(self.dir).length()
    }
}

/// An item or activator in the ball whose touched point counts, and how far that point lies from
/// the line of the aim.
#[derive(Copy, Clone, Debug)]
struct Candidate {
    handle: ColliderHandle,
    touched_point: Vec3,
    offset: f32,
}

impl Candidate {
    fn new(handle: ColliderHandle, touched_point: Vec3, aim: &ValidAim) -> Self {
        Self {
            handle,
            touched_point,
            offset: aim.offset_from_aim(touched_point),
        }
    }

    /// The touched point nearer the line of the aim wins; at an equal offset the one nearer the
    /// eye does.
    fn beats(&self, other: &Self, aim: &ValidAim) -> bool {
        self.offset
            .total_cmp(&other.offset)
            .then_with(|| {
                aim.origin
                    .distance(self.touched_point)
                    .total_cmp(&aim.origin.distance(other.touched_point))
            })
            .is_lt()
    }
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
    /// Otherwise every item and activator a ball swept along the whole reach of the ray touches
    /// is a candidate when its touched point is in front of the eye, within reach, and seen from
    /// the eye: no solid, activator or terrain other than itself lies on the line from the eye
    /// to that point. Items and the carried body never block that line. Of the candidates, the
    /// one whose touched point lies nearest the line of the ray wins. When there is none, the
    /// exact ray's hit is the aim.
    ///
    /// Costs the exact ray plus, only when the ray hit neither an item nor an activator, one query
    /// of the swept ball's volume restricted to items and activators, and for each collider in it
    /// one ball sweep and one visibility ray.
    pub fn pick_attribution(
        &self,
        aim: PickRay,
        classify: impl Fn(&str) -> PickClass,
    ) -> Option<RayHit> {
        let Some(aim) = aim.validated() else {
            warn_once(NO_USABLE_AIM, || {
                format!("pick: the aim {aim:?} has no usable origin and direction; it hits nothing")
            });
            return None;
        };
        let classes = RefClasses::new(self, &classify);
        let exact = self.probe(aim.origin, aim.dir, aim.reach, self.pick_filter());
        let exact_decides = aim.radius <= 0.0
            || exact
                .as_ref()
                .is_some_and(|probe| classes.of(probe.user_data).is_pickable());
        if exact_decides {
            return exact.map(RayProbe::into_hit);
        }
        self.best_candidate(aim, &classes)
            .and_then(|candidate| {
                let eye_distance = aim.origin.distance(candidate.touched_point);
                self.attribute(candidate.handle, eye_distance)
            })
            .or(exact)
            .map(RayProbe::into_hit)
    }

    /// The query filter of every pick query: the carried body is no part of the world to the
    /// aim, it must neither pick nor hide anything.
    fn pick_filter(&self) -> QueryFilter<'_> {
        QueryFilter {
            exclude_rigid_body: self.carried_handle(),
            ..scene_query_filter()
        }
    }

    /// Of the items and activators in the volume the ball sweeps, the candidate that wins.
    fn best_candidate(&self, aim: ValidAim, classes: &RefClasses<'_>) -> Option<Candidate> {
        let swept_ball = Capsule::new(aim.origin, aim.far_end(), aim.radius);
        let is_pickable =
            |_: ColliderHandle, collider: &Collider| classes.of(collider.user_data).is_pickable();
        let queries = self
            .world
            .query_pipeline_with_filter(self.pick_filter().predicate(&is_pickable));
        let mut best: Option<Candidate> = None;
        for (handle, collider) in queries.intersect_shape(Pose::IDENTITY, &swept_ball) {
            let Some(touched_point) = self.ball_touch(aim, handle) else {
                continue;
            };
            if !self.touch_counts(aim, touched_point, collider, classes) {
                continue;
            }
            let candidate = Candidate::new(handle, touched_point, &aim);
            if best.is_none_or(|best| candidate.beats(&best, &aim)) {
                best = Some(candidate);
            }
        }
        best
    }

    /// Where the ball, its centre travelling the reach along the aim, first touches the collider
    /// `handle`. A ball that starts inside it touches it at once.
    fn ball_touch(&self, aim: ValidAim, handle: ColliderHandle) -> Option<Vec3> {
        let is_this_collider = |other: ColliderHandle, _: &Collider| other == handle;
        let filter = self.pick_filter().predicate(&is_this_collider);
        let (_, hit) = self.world.query_pipeline_with_filter(filter).cast_shape(
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
        Some(hit.witness1)
    }

    fn touch_counts(
        &self,
        aim: ValidAim,
        touched_point: Vec3,
        touched: &Collider,
        classes: &RefClasses<'_>,
    ) -> bool {
        (touched_point - aim.origin).dot(aim.dir) > 0.0
            && aim.reach_distance(touched_point) <= aim.reach
            && self.is_seen_from(aim.origin, touched_point, touched, classes)
    }

    /// Whether nothing but items and the ref of `seen` lies on the line from `origin` to `point`,
    /// which differ.
    fn is_seen_from(
        &self,
        origin: Vec3,
        point: Vec3,
        seen: &Collider,
        classes: &RefClasses<'_>,
    ) -> bool {
        let seen_ref = unpack_ref(seen.user_data);
        let is_obstacle = |_: ColliderHandle, collider: &Collider| {
            classes.of(collider.user_data) != PickClass::Item
                && unpack_ref(collider.user_data) != seen_ref
        };
        let filter = self.pick_filter().predicate(&is_obstacle);
        let to_point = point - origin;
        let distance = to_point.length();
        self.world
            .cast_ray(
                &Ray::new(origin, to_point / distance),
                distance,
                true,
                filter,
            )
            .is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::db::collision_layers::{
        CollisionLayerRow, PlayerCollisionLayers, layer_flags,
    };
    use crate::bethesda_space::UNITS_TO_ENGINE;
    use crate::sim::carry::View;
    use crate::sim::collision::{CellColliderBatch, ColShape, PartShape, parse_sidecar_bytes};
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
    const SLIVER: &str = "0B0003:Fallout4.esm";
    const DOOR: &str = "0C0001:Fallout4.esm";
    const DOOR_PANEL: &str = "0C0002:Fallout4.esm";
    const TOASTER: &str = "0A0007:Fallout4.esm";
    const TEDDY: &str = "0A0008:Fallout4.esm";
    const TABLE: &str = "0B0004:Fallout4.esm";
    const GLASS: &str = "0B0005:Fallout4.esm";

    fn classify(ref_id: &str) -> PickClass {
        match ref_id {
            WALL | THIN_WALL | SLIVER | TABLE | GLASS | CRATE_REF => PickClass::Solid,
            DOOR | DOOR_PANEL => PickClass::Activator,
            _ => PickClass::Item,
        }
    }

    fn at(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3::new(x, y, z)
    }

    /// The cell of `batch` with `shapes`, loaded and stepped once.
    fn settled(batch: &CellColliderBatch, shapes: &[(&str, Vec<PartShape>)]) -> PhysicsContext {
        let baked = bake(batch, shapes);
        let mut context = wake_context();
        context.add_cell(GROUND_CELL, batch, Some(&baked));
        context.set_physics_center(GROUND_CELL, 2);
        run_ticks(&mut context, 1);
        context
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
        settled(
            &batch,
            &[
                (ITEM_PATH, vec![box_shape(ITEM_HALF)]),
                (WALL_PATH, vec![box_shape(WALL_HALF)]),
                (THIN_WALL_PATH, vec![box_shape(THIN_WALL_HALF)]),
                (DOOR_PATH, vec![box_shape(ITEM_HALF)]),
            ],
        )
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
        aim_from(context, EYE, dir, radius, reach)
    }

    fn aim_from(
        context: &PhysicsContext,
        origin: Vec3,
        dir: Vec3,
        radius: f32,
        reach: f32,
    ) -> Option<RayHit> {
        context.pick_attribution(
            PickRay {
                origin,
                dir,
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
    fn a_door_beside_the_ray_within_the_radius_is_aimed_at_like_an_item_and_one_outside_it_is_not()
    {
        let within = world(&[], &[], &[at(108.0, 5.0, -102.2)]);
        assert_eq!(
            aimed_ref(aim(&within, Vec3::X, RADIUS)),
            Some(DOOR.to_owned())
        );
        let outside = world(&[], &[], &[at(108.0, 5.0, -104.0)]);
        assert_eq!(aimed_ref(aim(&outside, Vec3::X, RADIUS)), None);
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
    fn an_item_hidden_behind_a_thin_solid_leaves_a_visible_item_in_the_ball_aimed_at() {
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

    /// W1 (2026-11-13/14): an item clear of a wall's end stayed named while the exact ray already
    /// met the wall, as long as the eye saw its touched point. A ball stopped at the wall would
    /// have lost it about 2 degrees early.
    #[test]
    fn an_item_beside_the_wall_the_ray_hits_beyond_its_face_is_aimed_at_while_its_touched_point_is_seen()
     {
        let wall_edge_at_z = -100.5;
        let wall = at(106.0, 5.0, wall_edge_at_z + WALL_HALF);
        let beside_the_wall_beyond_its_face = at(105.5, 5.0, -102.0);
        let context = world(&[(BESIDE, beside_the_wall_beyond_its_face)], &[wall], &[]);
        let exact = aim(&context, Vec3::X, 0.0).expect("the exact ray hits the wall");
        assert_eq!(exact.ref_id.as_deref(), Some(WALL));
        let wall_hit = EYE + Vec3::X * exact.dist;
        assert!(
            box_distance(
                wall_hit,
                beside_the_wall_beyond_its_face,
                Vec3::splat(ITEM_HALF)
            ) > RADIUS,
            "a ball stopped at the wall never reaches the item"
        );
        assert_eq!(
            aimed_ref(aim(&context, Vec3::X, RADIUS)),
            Some(BESIDE.to_owned())
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
    fn an_aim_without_a_usable_origin_or_direction_hits_nothing_and_warns_once() {
        let context = world(&[(BESIDE, at(101.5, 5.0, -101.0))], &[], &[]);
        let unusable = [
            (EYE, Vec3::ZERO),
            (EYE, Vec3::NAN),
            (EYE, Vec3::INFINITY),
            (Vec3::NAN, Vec3::X),
            (Vec3::INFINITY, Vec3::X),
            (Vec3::new(EYE.x, f32::NEG_INFINITY, EYE.z), Vec3::X),
        ];
        for (origin, dir) in unusable {
            crate::utils::forget_once(NO_USABLE_AIM);
            let log = crate::audio::test_support::SharedBuffer::default();
            let guard = log.capture("warn");
            let aim = PickRay {
                origin,
                dir,
                reach: REACH,
                radius: RADIUS,
            };
            let hits = [
                context.pick_attribution(aim, classify),
                context.pick_attribution(aim, classify),
            ];
            drop(guard);
            assert!(hits.iter().all(Option::is_none), "{origin} {dir}");
            let warnings = log.drain();
            assert_eq!(warnings.lines().count(), 1, "{origin} {dir}: {warnings}");
        }
    }

    #[test]
    fn a_direction_of_any_length_aims_like_its_unit_vector() {
        let context = world(&[(BESIDE, at(108.0, 5.0, -102.2))], &[], &[]);
        let unit = aim(&context, Vec3::X, RADIUS).expect("a hit");
        for length in [0.25, 7.0] {
            let scaled = aim(&context, Vec3::X * length, RADIUS).expect("a hit");
            assert_eq!(scaled.ref_id, unit.ref_id, "{length}");
            assert!((scaled.dist - unit.dist).abs() < 1e-5, "{length}");
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
        let mut context = settled(&batch, &[(CRATE_PATH, vec![box_shape(ITEM_HALF)])]);
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
        settled(&batch, &[(ITEM_PATH, vec![shape])])
    }

    #[test]
    fn the_ball_touches_the_grown_surface_of_a_rounded_hull_and_not_the_bare_one() {
        let ball_radius = 1.6;
        let aimed = |context: &PhysicsContext| aimed_ref(aim(context, Vec3::X, ball_radius));
        assert_eq!(aimed(&rounded_item_context(0.4)), Some(BESIDE.to_owned()));
        assert_eq!(aimed(&rounded_item_context(0.0)), None);
    }

    const CARRIED_HALF: f32 = 0.25;
    const SLIVER_HALF: f32 = 0.05;
    const SLIVER_PATH: &str = "meshes\\sliver.glb";
    const DOOR_SLAB_PATH: &str = "meshes\\door_slab.glb";
    const TABLE_PATH: &str = "meshes\\table.glb";
    const SIDEWAYS_FROM_AXIS: f32 = 1.9;
    const AHEAD_OF_EYE: f32 = 2.0;
    const SLIVER_FRACTION_OF_THE_WAY: f32 = 0.9;
    const _: () = assert!(SIDEWAYS_FROM_AXIS < RADIUS);

    #[derive(Copy, Clone)]
    struct Block {
        path: &'static str,
        ref_id: &'static str,
        half_extents: Vec3,
        at: Vec3,
    }

    fn cuboid_shape(half_extents: Vec3) -> PartShape {
        PartShape {
            shape: ColShape::Box {
                center: Vec3::ZERO,
                rotation: glam::Quat::IDENTITY,
                half_extents,
            },
            node: None,
            joint: None,
            material: 0,
        }
    }

    #[derive(Copy, Clone)]
    struct Crate {
        at: Vec3,
        carried: bool,
    }

    fn world_with_blocks(
        items: &[(&str, Vec3)],
        blocks: &[Block],
        crate_in_the_world: Option<Crate>,
    ) -> PhysicsContext {
        let mut batch = ground_batch();
        let mut shapes = vec![(ITEM_PATH, vec![box_shape(ITEM_HALF)])];
        if !items.is_empty() {
            batch.objects.push(pinned_group(
                ITEM_PATH,
                items.iter().map(|(r, p)| placement(r, *p)).collect(),
            ));
        }
        for block in blocks {
            batch.objects.push(object_group(
                block.path,
                vec![placement(block.ref_id, block.at)],
                false,
                None,
            ));
            shapes.push((block.path, vec![cuboid_shape(block.half_extents)]));
        }
        if let Some(crate_in_the_world) = crate_in_the_world {
            batch.objects.push(object_group(
                CRATE_PATH,
                vec![placement(CRATE_REF, crate_in_the_world.at)],
                true,
                Some(2.0),
            ));
            shapes.push((CRATE_PATH, vec![box_shape(CARRIED_HALF)]));
        }
        let mut context = settled(&batch, &shapes);
        if crate_in_the_world.is_some_and(|crate_in_the_world| crate_in_the_world.carried) {
            let view = View {
                yaw: 0.0,
                pitch: -0.5,
            };
            context
                .grab(refr_entity_id(CRATE_REF), EYE, view)
                .expect("grabbed");
        }
        context
    }

    /// An item whose corner nearest the axis is `corner`: its centre lies half an item beyond it,
    /// away from the axis (the aim looks along +x, the items sit on the -z side).
    fn item_at_corner(ref_id: &'static str, corner: Vec3) -> (&'static str, Vec3) {
        (ref_id, corner + Vec3::new(ITEM_HALF, 0.0, -ITEM_HALF))
    }

    /// The corner of the item the ball touches first: inside the radius sideways, ahead of the eye.
    fn touched_corner() -> Vec3 {
        at(EYE.x + AHEAD_OF_EYE, EYE.y, EYE.z - SIDEWAYS_FROM_AXIS)
    }

    fn halfway_to_the_touched_corner() -> Vec3 {
        EYE.lerp(touched_corner(), 0.5)
    }

    /// What the aim along +x reaches when the touched item has `blocks` and `crate_in_the_world`
    /// around it.
    fn aimed_at_the_touched_item_with(
        blocks: &[Block],
        crate_in_the_world: Option<Crate>,
    ) -> Option<String> {
        let item = item_at_corner(BESIDE, touched_corner());
        let context = world_with_blocks(&[item], blocks, crate_in_the_world);
        aimed_ref(aim(&context, Vec3::X, RADIUS))
    }

    #[test]
    fn a_carried_body_between_the_eye_and_an_item_the_ball_touches_does_not_hide_it() {
        let crate_between = |carried| {
            Some(Crate {
                at: halfway_to_the_touched_corner(),
                carried,
            })
        };
        assert_eq!(
            aimed_at_the_touched_item_with(&[], crate_between(false)),
            None
        );
        assert_eq!(
            aimed_at_the_touched_item_with(&[], crate_between(true)),
            Some(BESIDE.to_owned())
        );
    }

    #[test]
    fn a_carried_body_in_front_of_the_eye_does_not_hide_an_item_on_the_exact_ray() {
        let item_on_the_ray = (ON_AXIS, at(EYE.x + 10.0, EYE.y, EYE.z));
        let aimed = |carried| {
            let crate_on_the_ray = Crate {
                at: at(EYE.x + 4.0, EYE.y, EYE.z),
                carried,
            };
            let context = world_with_blocks(&[item_on_the_ray], &[], Some(crate_on_the_ray));
            aimed_ref(aim(&context, Vec3::X, RADIUS))
        };
        assert_eq!(aimed(false), Some(CRATE_REF.to_owned()));
        assert_eq!(aimed(true), Some(ON_AXIS.to_owned()));
    }

    #[test]
    fn an_item_between_the_eye_and_a_farther_touched_item_does_not_hide_it_from_the_eye() {
        let near_corner = at(EYE.x + 3.5, EYE.y, EYE.z - 0.5);
        let far_corner = at(EYE.x + 8.0, EYE.y, EYE.z - 1.5);
        let items = [
            item_at_corner(NEAR_ITEM, near_corner),
            item_at_corner(BESIDE, far_corner),
        ];
        let sliver = Block {
            path: SLIVER_PATH,
            ref_id: SLIVER,
            half_extents: Vec3::splat(SLIVER_HALF),
            at: EYE.lerp(near_corner, SLIVER_FRACTION_OF_THE_WAY),
        };
        let first_on_the_line_to = |corner: Vec3, context: &PhysicsContext| {
            let to_corner = corner - EYE;
            aimed_ref(context.ray_attribution(EYE, to_corner.normalize(), to_corner.length()))
        };

        let open = world_with_blocks(&items, &[], None);
        assert_eq!(
            aimed_ref(aim(&open, Vec3::X, RADIUS)),
            Some(NEAR_ITEM.to_owned()),
            "the item whose touched point lies nearer the aim wins"
        );

        let sliver_before_the_near_item = world_with_blocks(&items, &[sliver], None);
        assert_eq!(
            first_on_the_line_to(near_corner, &sliver_before_the_near_item),
            Some(SLIVER.to_owned())
        );
        assert_eq!(
            first_on_the_line_to(far_corner, &sliver_before_the_near_item),
            Some(NEAR_ITEM.to_owned()),
            "the line to the far item meets the near item and misses the sliver"
        );
        assert_eq!(
            aimed_ref(aim(&sliver_before_the_near_item, Vec3::X, RADIUS)),
            Some(BESIDE.to_owned())
        );
    }

    const fn game_units(units: f32) -> f32 {
        units * UNITS_TO_ENGINE
    }

    /// `fActivatePickLength` and `fActivatePickRadius` at the built-in defaults the original was
    /// measured with.
    const MEASURED_REACH: f32 = game_units(180.0);
    const MEASURED_RADIUS: f32 = game_units(16.0);
    /// Degrees either side of a switch at which the tests aim: wide enough for the shape cast,
    /// narrow enough that the other rules still name the other item.
    const CLEAR_OF_SWITCH: f32 = 0.05;
    /// Thirds or halvings of the test oracles' brackets, beyond the f32 resolution of their ends.
    const ORACLE_STEPS: u32 = 64;
    /// Items float this far above the floor or a top, so the line to their underside never grazes
    /// it.
    const ABOVE_THE_SURFACE: f32 = 0.1;

    fn aimed_measured(context: &PhysicsContext, dir: Vec3) -> Option<String> {
        aimed_ref(aim_with_reach(
            context,
            dir,
            MEASURED_RADIUS,
            MEASURED_REACH,
        ))
    }

    fn exact_measured(context: &PhysicsContext, dir: Vec3) -> Option<String> {
        aimed_ref(aim_with_reach(context, dir, 0.0, MEASURED_REACH))
    }

    /// The aim from +x turned by `degrees` toward `side`, a unit vector square to +x.
    fn turned_aim(degrees: f32, side: Vec3) -> Vec3 {
        let (sin, cos) = degrees.to_radians().sin_cos();
        Vec3::X * cos + side * sin
    }

    fn box_distance(point: Vec3, center: Vec3, half_extents: Vec3) -> f32 {
        ((point - center).abs() - half_extents)
            .max(Vec3::ZERO)
            .length()
    }

    fn span(values: impl Iterator<Item = f32>) -> (f32, f32) {
        values.fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), value| {
            (low.min(value), high.max(value))
        })
    }

    /// The boundary between `holds` (where `predicate` is true) and `fails`, either side of it.
    fn boundary(mut holds: f32, mut fails: f32, predicate: impl Fn(f32) -> bool) -> f32 {
        assert!(
            predicate(holds) && !predicate(fails),
            "{holds} and {fails} do not bracket the boundary"
        );
        for _ in 0..ORACLE_STEPS {
            let middle = f32::midpoint(holds, fails);
            if predicate(middle) {
                holds = middle;
            } else {
                fails = middle;
            }
        }
        f32::midpoint(holds, fails)
    }

    /// A point in the plane of a [`PlanarBox`]: along +x and toward its side, from its eye.
    #[derive(Copy, Clone, Debug)]
    struct InPlane {
        along: f32,
        across: f32,
    }

    impl InPlane {
        fn eye_distance(self) -> f32 {
            self.along.hypot(self.across)
        }

        /// Its depth along an aim turned by `degrees`.
        fn depth_along(self, degrees: f32) -> f32 {
            let (sin, cos) = degrees.to_radians().sin_cos();
            self.along * cos + self.across * sin
        }

        /// How far it lies from an aim turned by `degrees`.
        fn offset_from(self, degrees: f32) -> f32 {
            let (sin, cos) = degrees.to_radians().sin_cos();
            (self.across * cos - self.along * sin).abs()
        }
    }

    /// An axis-aligned box seen from `eye` in the plane spanned by +x and `side`. The test boxes
    /// are symmetric about that plane, so for any aim in it their nearest points and the points a
    /// ball swept along it touches lie in it, and their projected corners give their exact
    /// angles, distances and depths.
    struct PlanarBox {
        eye: Vec3,
        center: Vec3,
        half_extents: Vec3,
        side: Vec3,
    }

    impl PlanarBox {
        fn new(center: Vec3, half_extents: Vec3, side: Vec3) -> Self {
            Self {
                eye: EYE,
                center,
                half_extents,
                side,
            }
        }

        fn item(center: Vec3, side: Vec3) -> Self {
            Self::new(center, Vec3::splat(ITEM_HALF), side)
        }

        fn seen_from(self, eye: Vec3) -> Self {
            Self { eye, ..self }
        }

        fn in_plane(&self, point: Vec3) -> InPlane {
            let offset = point - self.eye;
            InPlane {
                along: offset.x,
                across: offset.dot(self.side),
            }
        }

        /// Each corner in the plane.
        fn corners(&self) -> impl Iterator<Item = InPlane> + '_ {
            [-1.0_f32, 1.0].into_iter().flat_map(move |x| {
                [-1.0_f32, 1.0].into_iter().flat_map(move |y| {
                    [-1.0_f32, 1.0].map(move |z| {
                        self.in_plane(self.center + self.half_extents * Vec3::new(x, y, z))
                    })
                })
            })
        }

        /// The angle (degrees) from +x toward `side` at which the eye sees its centre.
        fn center_angle(&self) -> f32 {
            let center = self.in_plane(self.center);
            center.across.atan2(center.along).to_degrees()
        }

        /// The smallest and largest angle (degrees) from +x toward `side` at which the eye sees it.
        fn angular_span(&self) -> (f32, f32) {
            span(
                self.corners()
                    .map(|corner| corner.across.atan2(corner.along).to_degrees()),
            )
        }

        /// How far it lies from an aim turned by `degrees` that misses it.
        fn distance_from_aim(&self, degrees: f32) -> f32 {
            span(self.corners().map(|corner| corner.offset_from(degrees))).0
        }

        /// Its nearest and farthest depth along an aim turned by `degrees`.
        fn depth_span(&self, degrees: f32) -> (f32, f32) {
            span(self.corners().map(|corner| corner.depth_along(degrees)))
        }

        /// Where a ball of the measured radius, its centre travelling the measured reach from the
        /// eye along an aim turned by `degrees`, first touches it. The distance from the moving
        /// centre to the box is convex in the travel: a ternary search finds its least value, a
        /// bisection before it the first touch.
        fn measured_touched_point(&self, degrees: f32) -> Option<InPlane> {
            let center = self.in_plane(self.center);
            let half = InPlane {
                along: self.half_extents.x,
                across: self.half_extents.dot(self.side.abs()),
            };
            let (sin, cos) = degrees.to_radians().sin_cos();
            let nearest = |travel: f32| {
                let ball = InPlane {
                    along: travel * cos,
                    across: travel * sin,
                };
                let point = InPlane {
                    along: ball
                        .along
                        .clamp(center.along - half.along, center.along + half.along),
                    across: ball
                        .across
                        .clamp(center.across - half.across, center.across + half.across),
                };
                let gap = (point.along - ball.along).hypot(point.across - ball.across);
                (point, gap)
            };
            let touches = |travel: f32| nearest(travel).1 <= MEASURED_RADIUS;
            let (mut low, mut high) = (0.0, MEASURED_REACH);
            for _ in 0..ORACLE_STEPS {
                let third = (high - low) / 3.0;
                if nearest(low + third).1 < nearest(high - third).1 {
                    high -= third;
                } else {
                    low += third;
                }
            }
            if !touches(low) {
                return None;
            }
            if touches(0.0) {
                return Some(nearest(0.0).0);
            }
            let first_touch = boundary(low, 0.0, touches);
            Some(nearest(first_touch).0)
        }

        /// How far the touched point of an aim turned by `degrees` lies from that aim, when it is
        /// within the measured reach of the eye.
        fn touched_offset(&self, degrees: f32) -> Option<f32> {
            self.measured_touched_point(degrees)
                .filter(|touched| touched.eye_distance() <= MEASURED_REACH)
                .map(|touched| touched.offset_from(degrees))
        }
    }

    /// The aim at which `narrower` (at the smaller angles) and `wider` lie at equal angles.
    fn equal_angle_aim(narrower: &PlanarBox, wider: &PlanarBox) -> f32 {
        let (_, narrower_edge) = narrower.angular_span();
        let (wider_edge, _) = wider.angular_span();
        assert!(narrower_edge < wider_edge, "the boxes overlap in angle");
        f32::midpoint(narrower_edge, wider_edge)
    }

    /// The aim between the centre angles of `first` and `second` at which their touched points
    /// lie equally far from it, `first`'s being the nearer one before it.
    fn equal_offset_aim(first: &PlanarBox, second: &PlanarBox) -> f32 {
        let first_nearer = |degrees: f32| match (
            first.touched_offset(degrees),
            second.touched_offset(degrees),
        ) {
            (Some(first), Some(second)) => first < second,
            (Some(_), None) => true,
            (None, _) => false,
        };
        boundary(first.center_angle(), second.center_angle(), first_nearer)
    }

    /// Whether a ball of `radius` swept along an aim turned by `degrees` meets `first` before
    /// `second`, both within the radius of the aim: it reaches a box no earlier than its nearest
    /// depth less the radius and no later than its farthest depth.
    fn first_contact_meets(
        first: &PlanarBox,
        second: &PlanarBox,
        degrees: f32,
        radius: f32,
    ) -> bool {
        first.distance_from_aim(degrees) < radius
            && second.distance_from_aim(degrees) < radius
            && first.depth_span(degrees).1 < second.depth_span(degrees).0 - radius
    }

    /// Two items level with the eye: the farther one 145 ahead on the aim, the nearer one
    /// `depth_gap` nearer and 14 beside it, and the aims just either side of the one at which
    /// their touched points lie equally far from the aim.
    struct LevelPair {
        farther_at: Vec3,
        nearer_at: Vec3,
        farther: PlanarBox,
        nearer: PlanarBox,
        before: f32,
        after: f32,
    }

    const LEVEL_PAIR_SIDE: Vec3 = Vec3::Z;

    fn level_pair(depth_gap: f32) -> LevelPair {
        let farther_at = at(EYE.x + game_units(145.0), EYE.y, EYE.z);
        let nearer_at = at(
            EYE.x + game_units(145.0 - depth_gap),
            EYE.y,
            EYE.z + game_units(14.0),
        );
        let farther = PlanarBox::item(farther_at, LEVEL_PAIR_SIDE);
        let nearer = PlanarBox::item(nearer_at, LEVEL_PAIR_SIDE);
        let switch = equal_offset_aim(&farther, &nearer);
        LevelPair {
            farther_at,
            nearer_at,
            farther,
            nearer,
            before: switch - CLEAR_OF_SWITCH,
            after: switch + CLEAR_OF_SWITCH,
        }
    }

    /// With equal boxes the swapped pair (P5 for P4, Q2 for Q1) is the same geometry: the
    /// winner follows the geometry, not the item.
    fn assert_the_pair_switches_from_the_farther_to_the_nearer(pair: &LevelPair) {
        for (far_ref, near_ref) in [(TOASTER, TEDDY), (TEDDY, TOASTER)] {
            let context = world(
                &[(far_ref, pair.farther_at), (near_ref, pair.nearer_at)],
                &[],
                &[],
            );
            for degrees in [pair.before, pair.after] {
                assert_eq!(
                    exact_measured(&context, turned_aim(degrees, LEVEL_PAIR_SIDE)),
                    None
                );
            }
            assert_eq!(
                aimed_measured(&context, turned_aim(pair.before, LEVEL_PAIR_SIDE)),
                Some(far_ref.to_owned()),
                "{far_ref} farther, at {} degrees",
                pair.before
            );
            assert_eq!(
                aimed_measured(&context, turned_aim(pair.after, LEVEL_PAIR_SIDE)),
                Some(near_ref.to_owned()),
                "{near_ref} nearer, at {} degrees",
                pair.after
            );
        }
    }

    /// P4 and P5 (2026-11-13/14): eye 145 from the farther item on the aim, the nearer item 40
    /// nearer and 14 beside it. First contact switched to the nearer item about 5 degrees early;
    /// the switch came where the two touched points lay equally far from the aim.
    #[test]
    fn the_item_whose_touched_point_lies_nearest_the_aim_wins_over_one_the_ball_meets_first() {
        let pair = level_pair(40.0);
        assert!(first_contact_meets(
            &pair.nearer,
            &pair.farther,
            pair.before,
            MEASURED_RADIUS
        ));
        assert_the_pair_switches_from_the_farther_to_the_nearer(&pair);
    }

    /// Q1 and Q2 (2026-11-14): the pair of P4/P5 with the nearer item 55 nearer. The smallest
    /// angle between the aim and the shapes missed the switch by 1.70 and 0.68 degrees; the
    /// touched point nearest the aim fitted within one sweep step.
    #[test]
    fn in_a_deeper_pair_the_touched_point_nearest_the_aim_wins_where_the_smaller_angle_names_the_other_item()
     {
        let pair = level_pair(55.0);
        let equal_angles = equal_angle_aim(&pair.farther, &pair.nearer);
        assert!(
            pair.after < equal_angles,
            "at {} degrees the farther item still lies at the smaller angle (equal at {equal_angles})",
            pair.after
        );
        assert_the_pair_switches_from_the_farther_to_the_nearer(&pair);
    }

    /// P6: the nearer item 60 nearer on the same line below the eye, the aim swept down from the
    /// farther one. The eye stands 4 above the item row so both lie well within the reach, and the
    /// items float 0.5 above the ground so the ground hides neither.
    #[test]
    fn sweeping_down_from_a_far_item_to_a_near_one_below_it_switches_where_their_touched_points_lie_equally_near_the_aim()
     {
        let side = Vec3::NEG_Y;
        let row_y = EYE.y - 4.0;
        let farther_at = at(EYE.x + game_units(145.0), row_y, EYE.z);
        let nearer_at = at(EYE.x + game_units(145.0 - 60.0), row_y, EYE.z);
        let farther = PlanarBox::item(farther_at, side);
        let nearer = PlanarBox::item(nearer_at, side);
        let switch = equal_offset_aim(&farther, &nearer);
        let before = switch - CLEAR_OF_SWITCH;
        let after = switch + CLEAR_OF_SWITCH;
        assert!(first_contact_meets(
            &nearer,
            &farther,
            before,
            MEASURED_RADIUS
        ));
        let context = world(&[(TOASTER, farther_at), (TEDDY, nearer_at)], &[], &[]);
        let exact = exact_measured(&context, turned_aim(before, side));
        assert!(
            exact.is_none(),
            "the exact ray meets only the ground, not {exact:?}"
        );
        assert_eq!(
            aimed_measured(&context, turned_aim(before, side)),
            Some(TOASTER.to_owned())
        );
        assert_eq!(
            aimed_measured(&context, turned_aim(after, side)),
            Some(TEDDY.to_owned())
        );
    }

    /// Q4 and the P6 gap (2026-11-14): the eye 118.5 above the floor, the farther item 145 ahead
    /// on the floor, the nearer one 90 nearer on the same line, the aim swept down from the
    /// farther one. The farther item was lost where its touched point passed the reach from the
    /// eye (measured 6.94, from the eye 7.18, along the aim 8.77 degrees below it), and nothing
    /// was named until the nearer item.
    #[test]
    fn the_far_item_of_a_pair_is_lost_where_its_touched_point_passes_the_reach_from_the_eye_not_along_the_aim()
     {
        let side = Vec3::NEG_Y;
        let eye = at(EYE.x, game_units(118.5), EYE.z);
        let row_y = ABOVE_THE_SURFACE + ITEM_HALF;
        let farther_at = at(eye.x + game_units(145.0), row_y, eye.z);
        let nearer_at = at(eye.x + game_units(145.0 - 90.0), row_y, eye.z);
        let farther = PlanarBox::item(farther_at, side).seen_from(eye);
        let nearer = PlanarBox::item(nearer_at, side).seen_from(eye);
        let aimed_at_the_farther = farther.center_angle();
        let swept_past_its_loss = aimed_at_the_farther + 6.0;
        let touched = |degrees: f32| {
            farther
                .measured_touched_point(degrees)
                .expect("the ball touches the farther item over the whole sweep")
        };
        let lost_from_the_eye = boundary(aimed_at_the_farther, swept_past_its_loss, |degrees| {
            touched(degrees).eye_distance() <= MEASURED_REACH
        });
        let lost_along_the_aim = boundary(aimed_at_the_farther, swept_past_its_loss, |degrees| {
            touched(degrees).depth_along(degrees) <= MEASURED_REACH
        });
        assert!(
            lost_along_the_aim - lost_from_the_eye > 2.0 * CLEAR_OF_SWITCH,
            "from the eye {lost_from_the_eye}, along the aim {lost_along_the_aim}"
        );
        let between = f32::midpoint(lost_from_the_eye, lost_along_the_aim);
        assert_eq!(nearer.touched_offset(between), None);

        let context = world(&[(TOASTER, farther_at), (TEDDY, nearer_at)], &[], &[]);
        let aimed = |degrees: f32| {
            aimed_ref(aim_from(
                &context,
                eye,
                turned_aim(degrees, side),
                MEASURED_RADIUS,
                MEASURED_REACH,
            ))
        };
        assert_eq!(
            aimed(lost_from_the_eye - CLEAR_OF_SWITCH),
            Some(TOASTER.to_owned())
        );
        assert_eq!(aimed(between), None);
    }

    /// D1: a door end and an item beside it compete by the same rule as two items.
    #[test]
    fn a_door_and_an_item_in_the_ball_compete_by_how_near_their_touched_points_lie_to_the_aim() {
        let side = Vec3::Z;
        let door_at = at(108.0, 5.0, -101.2);
        let item_at = at(112.0, 5.0, -99.0);
        let door = PlanarBox::item(door_at, side);
        let item = PlanarBox::item(item_at, side);
        let switch = equal_offset_aim(&door, &item);
        let toward_the_item = switch + CLEAR_OF_SWITCH;
        assert!(first_contact_meets(
            &door,
            &item,
            toward_the_item,
            MEASURED_RADIUS
        ));
        let context = world(&[(BESIDE, item_at)], &[], &[door_at]);
        assert_eq!(
            aimed_measured(&context, turned_aim(toward_the_item, side)),
            Some(BESIDE.to_owned())
        );
        assert_eq!(
            aimed_measured(&context, turned_aim(switch - CLEAR_OF_SWITCH, side)),
            Some(DOOR.to_owned())
        );
    }

    const DOOR_SLAB_HALF: Vec3 = Vec3::new(0.1, 3.0, 5.0);

    /// D3: an item 15 inside the end of a door and just behind it. The door was named until about
    /// one radius past its end, the item never.
    #[test]
    fn a_door_is_aimed_at_until_about_a_radius_past_its_end_and_the_item_it_hides_never() {
        let side = Vec3::Z;
        let item = (TOASTER, at(EYE.x + game_units(145.0), EYE.y, EYE.z));
        let door_end_z = EYE.z + game_units(15.0);
        let door = Block {
            path: DOOR_SLAB_PATH,
            ref_id: DOOR_PANEL,
            half_extents: DOOR_SLAB_HALF,
            at: at(
                item.1.x - game_units(15.0),
                EYE.y,
                door_end_z - DOOR_SLAB_HALF.z,
            ),
        };
        let open = world_with_blocks(&[item], &[], None);
        assert_eq!(
            aimed_measured(&open, turned_aim(4.0, side)),
            Some(TOASTER.to_owned()),
            "without the door the item is in the ball"
        );
        let behind_the_door = world_with_blocks(&[item], &[door], None);
        for degrees in 0..=16 {
            let aimed = aimed_measured(&behind_the_door, turned_aim(degrees as f32, side));
            assert!(
                matches!(aimed.as_deref(), Some(DOOR_PANEL) | None),
                "{degrees} degrees: {aimed:?}"
            );
        }
        let past_the_end_within_the_radius = turned_aim(10.0, side);
        assert_eq!(
            exact_measured(&behind_the_door, past_the_end_within_the_radius),
            None
        );
        assert_eq!(
            aimed_measured(&behind_the_door, past_the_end_within_the_radius),
            Some(DOOR_PANEL.to_owned())
        );
        assert_eq!(
            aimed_measured(&behind_the_door, turned_aim(16.0, side)),
            None
        );
    }

    const TABLE_HALF: Vec3 = Vec3::new(4.0, 0.1, 3.0);
    const TABLE_AT: Vec3 = Vec3::new(108.0, 2.8, -100.0);

    /// T2: an item 20 in from a table's near edge, the crosshair on the top short of it. The
    /// original kept naming the item where a ball stopped at the top would have lost it.
    #[test]
    fn an_item_on_a_table_stays_aimed_at_while_the_crosshair_rests_on_the_top_short_of_it() {
        let table = Block {
            path: TABLE_PATH,
            ref_id: TABLE,
            half_extents: TABLE_HALF,
            at: TABLE_AT,
        };
        let top = TABLE_AT.y + TABLE_HALF.y;
        let near_edge = TABLE_AT.x - TABLE_HALF.x;
        let item_at = at(
            near_edge + game_units(20.0) + ITEM_HALF,
            top + ABOVE_THE_SURFACE + ITEM_HALF,
            EYE.z,
        );
        let context = world_with_blocks(&[(TOASTER, item_at)], &[table], None);
        let short_of_the_item = Vec3::new(2.0, -1.0, 0.0);
        let exact = aim_with_reach(&context, short_of_the_item, 0.0, MEASURED_REACH)
            .expect("the exact ray hits the table");
        assert_eq!(exact.ref_id.as_deref(), Some(TABLE));
        let top_hit = EYE + short_of_the_item.normalize() * exact.dist;
        assert!(
            box_distance(top_hit, item_at, Vec3::splat(ITEM_HALF)) > MEASURED_RADIUS,
            "a ball stopped at the top never reaches the item"
        );
        assert_eq!(
            aimed_measured(&context, short_of_the_item),
            Some(TOASTER.to_owned())
        );
    }

    const GLASS_PATH: &str = "meshes\\glass.glb";
    const TRANSPARENT_LAYER: u8 = 3;
    const TRIGGER_LAYER: u8 = 12;
    const GLASS_HALF: [f32; 3] = [0.1, 3.0, 3.0];

    fn layer_row(
        form_key: &str,
        editor_id: &str,
        index: u8,
        flags: i64,
        collides_with: &[&str],
    ) -> CollisionLayerRow {
        CollisionLayerRow {
            form_key: form_key.to_owned(),
            editor_id: Some(editor_id.to_owned()),
            layer_index: Some(i64::from(index)),
            flags: Some(flags),
            collides_with: Some(collides_with.iter().map(|key| (*key).to_owned()).collect()),
        }
    }

    /// The installed table's say on the layers of a glass pane: `L_TRANSPARENT` collides with the
    /// character controller; a trigger layer never touches the player.
    fn installed_layers() -> PlayerCollisionLayers {
        PlayerCollisionLayers::from_rows(&[
            layer_row(
                "C:x.esm",
                "L_CHARCONTROLLER",
                30,
                0,
                &["B:x.esm", "T:x.esm"],
            ),
            layer_row("B:x.esm", "L_CHARBUMPER", 56, 0, &["C:x.esm"]),
            layer_row(
                "T:x.esm",
                "L_TRANSPARENT",
                TRANSPARENT_LAYER,
                0,
                &["C:x.esm", "T:x.esm"],
            ),
            layer_row(
                "G:x.esm",
                "L_TRIGGER",
                TRIGGER_LAYER,
                layer_flags::TRIGGER_VOLUME,
                &["C:x.esm"],
            ),
        ])
        .expect("a complete layer table")
    }

    /// A glass pane read from its collision sidecar, its slab on `layer`.
    fn glass_pane_on(layer: u8) -> Vec<PartShape> {
        let sidecar = serde_json::json!({
            "shapes": [{ "type": "box", "half_extents": GLASS_HALF, "layer": layer }],
        });
        let bytes = serde_json::to_vec(&sidecar).expect("the sidecar serialises");
        let parsed = parse_sidecar_bytes(&bytes, GLASS_PATH, &installed_layers())
            .expect("the sidecar parses");
        parsed.shapes.to_vec()
    }

    /// G2-G4 (2026-11-14): no rollover through a glass pane, exactly as through a plywood wall.
    /// The pane's slab sits on `L_TRANSPARENT`, so it loads as a collider and hides from the eye
    /// what lies behind it.
    #[test]
    fn a_glass_pane_on_the_transparent_layer_hides_an_item_behind_it_like_a_wall() {
        let item = (BESIDE, at(104.5, 5.0, -101.0));
        let pane_at = at(103.0, 5.0, -100.0);
        let aimed_through_a_pane_on = |layer: u8| {
            let mut batch = ground_batch();
            batch
                .objects
                .push(pinned_group(ITEM_PATH, vec![placement(item.0, item.1)]));
            batch.objects.push(object_group(
                GLASS_PATH,
                vec![placement(GLASS, pane_at)],
                false,
                None,
            ));
            let context = settled(
                &batch,
                &[
                    (ITEM_PATH, vec![box_shape(ITEM_HALF)]),
                    (GLASS_PATH, glass_pane_on(layer)),
                ],
            );
            aimed_ref(aim(&context, Vec3::X, RADIUS))
        };
        assert_eq!(
            aimed_through_a_pane_on(TRANSPARENT_LAYER),
            Some(GLASS.to_owned())
        );
        assert_eq!(
            aimed_through_a_pane_on(TRIGGER_LAYER),
            Some(BESIDE.to_owned()),
            "a pane on a layer that never touches the player is no collider and hides nothing"
        );
    }
}
