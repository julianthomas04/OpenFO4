//! Where a dropped inventory item enters the world. Pure.
//!
//! Source: original-game drop trials on flat floor (toaster, 10mm stack, teddy, cue, single round).
//! The item appears at the first free candidate point and physics settles it.

use glam::Vec3;

use super::world_streamer::{DroppedItemSpec, dropped_item_rotation};
use crate::bethesda_space::UNITS_TO_ENGINE;
use crate::sim::collision_box::CollisionBox;
use crate::sim::look_direction;

/// How far beyond the item's half diagonal a candidate point lies from the body axis. Measured
/// 97-100 game units.
const BASE_DISTANCE: f32 = 98.5 * UNITS_TO_ENGINE;
/// How many item collision heights below the standing eye an obstacle top may reach before it
/// refuses a direction. Measured 0.720-0.732.
const HEIGHT_FACTOR: f32 = 0.726;
/// The turn between candidate directions, clockwise from the facing.
const DIRECTION_STEP_DEGREES: u32 = 36;
const DIRECTION_COUNT: usize = (360 / DIRECTION_STEP_DEGREES) as usize;
/// The candidate straight behind the body, where the item goes when every direction is refused.
const BEHIND: usize = DIRECTION_COUNT / 2;
const _: () = assert!(
    360 % DIRECTION_STEP_DEGREES == 0 && DIRECTION_COUNT.is_multiple_of(2),
    "the steps must close the circle and include the direction behind the body"
);

/// Which candidate the placement ended at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum DropSpot {
    /// The free direction, in radians clockwise from the facing.
    Toward(f32),
    /// No direction was free: in front of what blocks the direction behind the body.
    Behind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct DropPlacement {
    /// Where the item's model origin starts.
    pub position: Vec3,
    pub spot: DropSpot,
    /// The engine camera yaw the item faces: the direction of the view.
    pub yaw: f32,
}

/// The player the item is dropped by, in engine units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Dropper {
    pub feet: Vec3,
    pub standing_eye: f32,
    /// Horizontal radius of the player's collision capsule.
    pub body_radius: f32,
    /// The engine camera yaw of the view.
    pub yaw: f32,
}

/// The angle of the `index`-th candidate direction, in radians clockwise from the facing.
fn candidate_angle(index: usize) -> f32 {
    (index as f32 * DIRECTION_STEP_DEGREES as f32).to_radians()
}

/// The candidate directions in the order they are tried.
fn candidate_angles() -> impl Iterator<Item = f32> {
    (0..DIRECTION_COUNT).map(candidate_angle)
}

/// Where the item starts. Each candidate direction is asked with `ray(origin, direction, reach)`,
/// the distance to the first static surface within `reach`, horizontally from the body axis at the
/// height an obstacle top must reach to refuse it: eye − `HEIGHT_FACTOR` × the item's collision
/// height. A free direction puts the centre of the item's collision box `BASE_DISTANCE` + its half
/// diagonal out; its collision bottom starts at the refusal height, so a lower obstacle there is
/// landed on. With every direction refused the box centre goes behind the body, a half diagonal
/// short of the hit there, but never into the player's capsule.
pub(super) fn placement(
    dropper: Dropper,
    item: CollisionBox,
    mut ray: impl FnMut(Vec3, Vec3, f32) -> Option<f32>,
) -> DropPlacement {
    let refusal_height = dropper.standing_eye - HEIGHT_FACTOR * item.height();
    let origin = dropper.feet + Vec3::Y * refusal_height;
    let half_diagonal = item.half_diagonal();
    let distance = BASE_DISTANCE + half_diagonal;
    let reach = distance + half_diagonal;
    let centre_offset = dropped_item_rotation(dropper.yaw) * item.center();
    let start = |direction: Vec3, along: f32, spot| DropPlacement {
        position: origin + direction * along
            - Vec3::new(centre_offset.x, item.min.y, centre_offset.z),
        spot,
        yaw: dropper.yaw,
    };
    let direction_at = |angle: f32| look_direction(dropper.yaw + angle, 0.0);
    let mut hits = [0.0; DIRECTION_COUNT];
    for (index, angle) in candidate_angles().enumerate() {
        match ray(origin, direction_at(angle), reach) {
            None => return start(direction_at(angle), distance, DropSpot::Toward(angle)),
            Some(hit) => hits[index] = hit,
        }
    }
    let clear_of_body = dropper.body_radius + item.horizontal_half_extent();
    let along = (hits[BEHIND] - half_diagonal).max(clear_of_body);
    start(
        direction_at(candidate_angle(BEHIND)),
        along,
        DropSpot::Behind,
    )
}

pub(super) fn dropped_spec(
    form_key: &str,
    count: u32,
    placement: DropPlacement,
) -> DroppedItemSpec {
    DroppedItemSpec {
        base: form_key.to_owned(),
        count,
        position: placement.position,
        yaw: placement.yaw,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::SimWorld;
    use crate::sim::test_world::{
        Prop, SPAWN_AT, drop_crate, prop_box, spawned_at, tick, world_with,
    };

    const GAME: f32 = UNITS_TO_ENGINE;
    const FEET: Vec3 = Vec3::new(10.0, 2.0, 30.0);
    const EYE: f32 = 120.5 * GAME;
    const BODY_RADIUS: f32 = 2.5;
    const TOASTER: CollisionBox = item(11.0, 16.8, 19.3);
    const STACK: CollisionBox = item(6.8, 4.1, 12.0);

    /// An item box `width` × `height` × `depth` game units, its bottom at the model origin and
    /// centred above it.
    const fn item(width: f32, height: f32, depth: f32) -> CollisionBox {
        CollisionBox {
            min: Vec3::new(-width / 2.0 * GAME, 0.0, -depth / 2.0 * GAME),
            max: Vec3::new(width / 2.0 * GAME, height * GAME, depth / 2.0 * GAME),
        }
    }

    fn dropper(yaw: f32) -> Dropper {
        Dropper {
            feet: FEET,
            standing_eye: EYE,
            body_radius: BODY_RADIUS,
            yaw,
        }
    }

    /// Where the centre of `item`'s collision box starts.
    fn box_centre(placed: DropPlacement, item: CollisionBox) -> Vec3 {
        placed.position + dropped_item_rotation(placed.yaw) * item.center()
    }

    /// How far horizontally from the body axis the item's box centre starts, in game units.
    fn horizontal_reach(placed: DropPlacement, item: CollisionBox) -> f32 {
        let offset = box_centre(placed, item) - FEET;
        offset.x.hypot(offset.z) / GAME
    }

    /// A wall across the direction `degrees` clockwise from the facing, `near` game units from the
    /// body axis, rising from the floor to `top` game units.
    #[derive(Clone, Copy)]
    struct Wall {
        degrees: f32,
        near: f32,
        top: f32,
    }

    /// The static world of `walls` as the placement's ray sees it, the player facing yaw 0.
    fn world(walls: &[Wall]) -> impl FnMut(Vec3, Vec3, f32) -> Option<f32> + '_ {
        move |origin, direction, reach| {
            walls
                .iter()
                .filter(|wall| {
                    (look_direction(wall.degrees.to_radians(), 0.0) - direction).length() < 1e-4
                })
                .filter(|wall| origin.y - FEET.y <= wall.top * GAME)
                .map(|wall| wall.near * GAME)
                .filter(|near| *near <= reach)
                .reduce(f32::min)
        }
    }

    /// A ring of walls `near` game units out on every candidate direction but the one behind,
    /// whose wall stands `rear` out.
    fn ring(near: f32, rear: f32) -> Vec<Wall> {
        candidate_angles()
            .map(f32::to_degrees)
            .map(|degrees| Wall {
                degrees,
                near: if (degrees - 180.0).abs() < 1e-3 {
                    rear
                } else {
                    near
                },
                top: 300.0,
            })
            .collect()
    }

    fn assert_near(actual: Vec3, expected: Vec3) {
        assert!(
            (actual - expected).length() < 1e-3,
            "{actual:?} against {expected:?}"
        );
    }

    fn assert_toward(placed: DropPlacement, degrees: f32) {
        match placed.spot {
            DropSpot::Toward(angle) => {
                assert!((angle.to_degrees() - degrees).abs() < 1e-3, "{placed:?}");
            }
            DropSpot::Behind => panic!("expected {degrees} degrees, got {placed:?}"),
        }
    }

    #[test]
    fn the_directions_are_tried_from_the_facing_clockwise_in_36_degree_steps() {
        let mut asked = Vec::new();
        placement(dropper(0.7), TOASTER, |_, direction, _| {
            asked.push(direction);
            Some(1.0)
        });
        let expected: Vec<Vec3> = (0..10)
            .map(|step| look_direction(0.7 + (36.0 * step as f32).to_radians(), 0.0))
            .collect();
        assert_eq!(asked.len(), expected.len());
        for (direction, expected) in asked.into_iter().zip(expected) {
            assert_near(direction, expected);
        }
    }

    #[test]
    fn clockwise_turns_from_the_facing_towards_its_right() {
        let mut asked = Vec::new();
        placement(dropper(0.0), TOASTER, |_, direction, _| {
            asked.push(direction);
            Some(1.0)
        });
        let right = Vec3::X.cross(Vec3::Y);
        assert!(asked[1].dot(right) > 0.5, "{:?}", asked[1]);
        assert!(asked[9].dot(right) < -0.5, "{:?}", asked[9]);
    }

    #[test]
    fn a_free_facing_puts_the_box_centre_a_base_distance_and_half_diagonal_ahead() {
        let placed = placement(dropper(0.0), TOASTER, world(&[]));
        assert_toward(placed, 0.0);
        let out = 98.5 * GAME + TOASTER.half_diagonal();
        let bottom = EYE - 0.726 * TOASTER.height();
        let centre = FEET + Vec3::new(out, bottom + TOASTER.height() / 2.0, 0.0);
        assert_near(box_centre(placed, TOASTER), centre);
    }

    #[test]
    fn an_off_centre_box_is_centred_on_the_candidate_point_whatever_the_yaw() {
        let lopsided = CollisionBox {
            min: Vec3::new(0.0, -0.5, 0.0),
            max: Vec3::new(2.0, 1.0, 1.0),
        };
        for yaw in [-2.0_f32, 0.0, 1.0, 2.5] {
            let placed = placement(dropper(yaw), lopsided, |_, _, _| None);
            let out = 98.5 * GAME + lopsided.half_diagonal();
            let bottom = EYE - 0.726 * lopsided.height();
            let centre = FEET
                + look_direction(yaw, 0.0) * out
                + Vec3::Y * (bottom + lopsided.height() / 2.0);
            assert_near(box_centre(placed, lopsided), centre);
        }
    }

    #[test]
    fn the_ray_runs_from_the_body_axis_a_half_diagonal_beyond_the_candidate_point() {
        let mut asked = None;
        placement(dropper(0.0), STACK, |origin, direction, reach| {
            asked.get_or_insert((origin, direction, reach));
            None
        });
        let (origin, direction, reach) = asked.unwrap();
        assert_near(origin, FEET + Vec3::Y * (EYE - 0.726 * STACK.height()));
        assert_near(direction, Vec3::X);
        assert!((reach - (98.5 * GAME + 2.0 * STACK.half_diagonal())).abs() < 1e-4);
    }

    #[test]
    fn a_toaster_passes_a_108_2_obstacle_and_is_refused_by_a_108_4_one() {
        for near in [75.0, 88.0, 100.0] {
            let low = [Wall {
                degrees: 0.0,
                near,
                top: 108.2,
            }];
            assert_toward(placement(dropper(0.0), TOASTER, world(&low)), 0.0);
            let high = [Wall {
                top: 108.4,
                ..low[0]
            }];
            assert_toward(placement(dropper(0.0), TOASTER, world(&high)), 36.0);
        }
    }

    #[test]
    fn a_stack_passes_a_117_0_obstacle_and_is_refused_by_a_117_7_one() {
        let low = [Wall {
            degrees: 0.0,
            near: 88.0,
            top: 117.0,
        }];
        assert_toward(placement(dropper(0.0), STACK, world(&low)), 0.0);
        let high = [Wall {
            top: 117.7,
            ..low[0]
        }];
        assert_toward(placement(dropper(0.0), STACK, world(&high)), 36.0);
    }

    #[test]
    fn an_obstacle_beyond_a_half_diagonal_past_the_candidate_point_does_not_refuse() {
        let reach = 98.5 + 2.0 * TOASTER.half_diagonal() / GAME;
        let beyond = [Wall {
            degrees: 0.0,
            near: reach + 0.1,
            top: 300.0,
        }];
        assert_toward(placement(dropper(0.0), TOASTER, world(&beyond)), 0.0);
        let within = [Wall {
            near: reach - 0.1,
            ..beyond[0]
        }];
        assert_toward(placement(dropper(0.0), TOASTER, world(&within)), 36.0);
    }

    #[test]
    fn the_first_free_direction_in_order_is_taken() {
        let walls: Vec<Wall> = [0.0, 36.0, 72.0, 324.0]
            .map(|degrees| Wall {
                degrees,
                near: 60.0,
                top: 300.0,
            })
            .to_vec();
        assert_toward(placement(dropper(0.0), TOASTER, world(&walls)), 108.0);
    }

    #[test]
    fn with_every_direction_refused_the_toaster_lies_where_it_was_seen_behind_the_body() {
        for (rear, seen) in [(60.0, 46.0), (70.0, 56.0), (80.0, 66.0)] {
            let placed = placement(dropper(0.0), TOASTER, world(&ring(50.0, rear)));
            assert_eq!(placed.spot, DropSpot::Behind);
            let centre = box_centre(placed, TOASTER);
            assert!(centre.x < FEET.x, "{placed:?}");
            assert!((centre.z - FEET.z).abs() < 1e-3, "{placed:?}");
            let reach = horizontal_reach(placed, TOASTER);
            assert!((reach - seen).abs() <= 2.0, "rear {rear}: {reach}");
        }
    }

    #[test]
    fn a_fallback_hit_close_behind_keeps_the_item_clear_of_the_capsule() {
        let placed = placement(dropper(0.0), TOASTER, world(&ring(20.0, 20.0)));
        assert_eq!(placed.spot, DropSpot::Behind);
        let clear = (BODY_RADIUS + TOASTER.horizontal_half_extent()) / GAME;
        assert!(
            (horizontal_reach(placed, TOASTER) - clear).abs() < 1e-3,
            "{placed:?}"
        );
    }

    #[test]
    fn the_item_faces_the_view_wherever_it_lands() {
        for yaw in [-3.0_f32, -1.0, 0.0, 0.4, 2.9] {
            for refused in [false, true] {
                let placed = placement(dropper(yaw), TOASTER, |_, _, _| refused.then_some(1.0));
                assert!((placed.yaw - yaw).abs() < 1e-6, "{yaw}: {placed:?}");
            }
        }
    }

    #[test]
    fn the_spec_carries_the_item_count_position_and_yaw_of_the_placement() {
        let placed = DropPlacement {
            position: Vec3::new(1.0, 2.0, 3.0),
            spot: DropSpot::Behind,
            yaw: 0.75,
        };
        assert_eq!(
            dropped_spec("0F3B01:Test.esm", 7, placed),
            DroppedItemSpec {
                base: "0F3B01:Test.esm".to_owned(),
                count: 7,
                position: Vec3::new(1.0, 2.0, 3.0),
                yaw: 0.75,
            }
        );
    }

    /// The test world's 1 × 1 × 1 crate, centred on its origin.
    const CRATE: CollisionBox = CollisionBox {
        min: Vec3::splat(-0.5),
        max: Vec3::splat(0.5),
    };
    const ON_THE_GROUND: Vec3 = Vec3::new(50.0, 0.0, -50.0);

    /// A static table across the facing (+X) from 80 to 140 game units out, `top` game units high.
    fn table(top: f32) -> Prop {
        let half = Vec3::new(3.0, top * GAME / 2.0, 3.0);
        prop_box(
            "meshes\\table.glb",
            ON_THE_GROUND + Vec3::new(8.0 + half.x, half.y, 0.0),
            half,
        )
    }

    /// A world of `props` whose colliders answer queries and whose bodies simulate; the player
    /// stands well away from them.
    fn running_world(props: &[Prop]) -> SimWorld {
        spawned_at(world_with(props), SPAWN_AT)
    }

    fn placed_in(world: &SimWorld) -> DropPlacement {
        let on_the_ground = Dropper {
            feet: ON_THE_GROUND,
            ..dropper(0.0)
        };
        placement(on_the_ground, CRATE, |origin, direction, reach| {
            world.static_ray_distance(origin, direction, reach)
        })
    }

    #[test]
    fn an_item_dropped_over_a_lower_table_lands_on_its_top() {
        let placed = placed_in(&running_world(&[table(75.0)]));
        assert_toward(placed, 0.0);
        let mut landed = running_world(&[table(75.0)]);
        let dropped = drop_crate(&mut landed, placed.position);
        tick(&mut landed, 120);
        let crate_pose = landed
            .body_translation(dropped)
            .expect("the crate is a body");
        assert!(
            (crate_pose.y - (7.5 + 0.5)).abs() < 0.05,
            "the crate rests at {crate_pose:?}"
        );
        assert!(
            (crate_pose.x - placed.position.x).abs() < 0.5,
            "{crate_pose:?}"
        );
    }

    #[test]
    fn a_table_reaching_the_refusal_height_refuses_the_direction_in_the_physics_world() {
        let refusal = (EYE - 0.726 * CRATE.height()) / GAME;
        assert_toward(placed_in(&running_world(&[table(refusal - 0.2)])), 0.0);
        assert_toward(placed_in(&running_world(&[table(refusal + 0.2)])), 36.0);
    }

    #[test]
    fn with_every_direction_refused_the_fallback_ray_is_the_one_behind() {
        let mut asked = 0;
        let placed = placement(dropper(0.0), TOASTER, |_, direction, _| {
            asked += 1;
            Some(if direction.x < -0.99 {
                80.0 * GAME
            } else {
                1.0
            })
        });
        assert_eq!(asked, 10);
        let expected = 80.0 - TOASTER.half_diagonal() / GAME;
        assert!((horizontal_reach(placed, TOASTER) - expected).abs() < 1e-3);
    }
}
