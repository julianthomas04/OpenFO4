//! Where a dropped inventory item enters the world. Pure.

use glam::Vec3;

use super::world_streamer::DroppedItemSpec;
use crate::bethesda_space::UNITS_TO_ENGINE;
use crate::sim::HOLD_INSET;

/// Where the original puts a dropped item, in engine units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct DropLook {
    /// How far in front of the body, horizontally along the facing, the item starts.
    pub forward_distance: f32,
    /// How far below the eye line the item starts; it falls from there.
    pub below_eye: f32,
    /// How much free ray length is asked beyond a candidate spot, so its body begins clear of the
    /// surface.
    pub margin: f32,
    /// How far from the body the item starts when the front spot is blocked.
    pub side_distance: f32,
    /// The angle to the right of the facing the side spot lies at, in radians.
    pub side_angle: f32,
}

/// The one place the drop numbers live. Source: original-game Pip-Boy sessions 2026-11-12 and
/// 2026-11-13, flat floor, eye height 118.5 above the feet.
/// - `forward_distance` 90 units: the item lands 81, 85, 118 and about 100 ahead along the facing
///   (n = 4, scatter from bounce and roll); the spawn rule fitted to the frames is 80-100 ahead.
///   The midpoint of that band. Derived, medium confidence. View pitch does not move it, so the
///   drop follows the body yaw only.
/// - `below_eye` 50 units: first seen 319 ms after the Pip-Boy closed about 60 above the floor,
///   falling about 190-220 units/s; the fitted spawn is 60-80 above the feet (40-60 below the
///   eye), released under gravity. The midpoint. Derived from 3 drops, medium confidence.
/// - `side_distance` 110 units and `side_angle` 97 degrees: with the front spot blocked by a wall
///   the item was found 105 units away at 96.6 degrees to the right and 115 at 96.85 degrees (n =
///   2, two walls, medium confidence; a left fallback was never seen). The midpoint of the
///   distances and the mean of the angles. If the side spot is blocked too (never observed) the
///   item is pulled back along the front ray.
/// - `margin`: not measured. Stand-in: the clearance of a carried body (`HOLD_INSET`).
pub(super) const OBSERVED_DROP: DropLook = DropLook {
    forward_distance: 90.0 * UNITS_TO_ENGINE,
    below_eye: 50.0 * UNITS_TO_ENGINE,
    margin: HOLD_INSET,
    side_distance: 110.0 * UNITS_TO_ENGINE,
    side_angle: 97.0 * std::f32::consts::PI / 180.0,
};

/// Which of the candidate spots the placement ended at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DropSpot {
    Front,
    Side,
    PulledBack,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct DropPlacement {
    pub position: Vec3,
    pub spot: DropSpot,
    /// The engine camera yaw the item faces: the direction of the view.
    pub yaw: f32,
}

/// The horizontal direction of the view: the body's facing, pitch ignored. Zero for a view
/// straight up or down.
fn facing(forward: Vec3) -> Vec3 {
    Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero()
}

/// The horizontal direction `angle` to the right of `facing` (a unit vector).
fn to_the_right(facing: Vec3, angle: f32) -> Vec3 {
    let right = facing.cross(Vec3::Y);
    facing * angle.cos() + right * angle.sin()
}

/// Where the item starts: in front of the body along the facing; when a wall or object blocks that
/// spot, to the right of the body; when that is blocked too, pulled back along the front ray.
/// `ray(origin, direction, reach)` is the distance to the first surface within `reach`; it is
/// asked horizontally from the body axis at the spawn height, so what the item body would stand
/// in blocks it. Infallible; refusals are made in the sim and the Pip-Boy.
pub(super) fn placement(
    eye: Vec3,
    forward: Vec3,
    ray: impl FnMut(Vec3, Vec3, f32) -> Option<f32>,
) -> DropPlacement {
    placement_with(&OBSERVED_DROP, eye, forward, ray)
}

fn placement_with(
    look: &DropLook,
    eye: Vec3,
    forward: Vec3,
    mut ray: impl FnMut(Vec3, Vec3, f32) -> Option<f32>,
) -> DropPlacement {
    let direction = facing(forward);
    let origin = eye - Vec3::Y * look.below_eye;
    let yaw = direction.z.atan2(direction.x);
    if direction == Vec3::ZERO {
        return DropPlacement {
            position: origin,
            spot: DropSpot::Front,
            yaw,
        };
    }
    let Some(front_hit) = ray(origin, direction, look.forward_distance + look.margin) else {
        return DropPlacement {
            position: origin + direction * look.forward_distance,
            spot: DropSpot::Front,
            yaw,
        };
    };
    let side = to_the_right(direction, look.side_angle);
    if ray(origin, side, look.side_distance + look.margin).is_none() {
        return DropPlacement {
            position: origin + side * look.side_distance,
            spot: DropSpot::Side,
            yaw,
        };
    }
    let free_distance = (front_hit - look.margin).max(0.0);
    DropPlacement {
        position: origin + direction * free_distance,
        spot: DropSpot::PulledBack,
        yaw,
    }
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
    use std::f32::consts::FRAC_PI_2;

    use super::super::ACTIVATE_REACH;
    use super::*;
    use crate::sim::look_direction;

    const LOOK: DropLook = DropLook {
        forward_distance: 12.0,
        below_eye: 0.5,
        margin: 2.0,
        side_distance: 14.0,
        side_angle: FRAC_PI_2,
    };
    const EYE: Vec3 = Vec3::new(10.0, 20.0, 30.0);
    const SPAWN_Y: f32 = 19.5;

    fn assert_near(actual: Vec3, expected: Vec3) {
        assert!(
            (actual - expected).length() < 1e-4,
            "{actual:?} against {expected:?}"
        );
    }

    fn open(_: Vec3, _: Vec3, _: f32) -> Option<f32> {
        None
    }

    /// A ray that hits at `distance` along the front and nowhere else.
    fn wall_ahead(distance: f32) -> impl FnMut(Vec3, Vec3, f32) -> Option<f32> {
        move |_, direction, reach| (direction == Vec3::X && distance <= reach).then_some(distance)
    }

    #[test]
    fn with_nothing_in_the_way_the_item_starts_at_the_full_distance_below_the_eye() {
        let placed = placement_with(&LOOK, EYE, Vec3::X, open);
        assert_near(placed.position, Vec3::new(22.0, SPAWN_Y, 30.0));
    }

    #[test]
    fn a_blocked_front_spot_moves_the_item_to_the_right_of_the_body() {
        let placed = placement_with(&LOOK, EYE, Vec3::X, wall_ahead(7.0));
        assert_near(placed.position, Vec3::new(10.0, SPAWN_Y, 44.0));
    }

    #[test]
    fn the_placement_names_the_spot_it_ended_at() {
        let spot =
            |ray: fn(Vec3, Vec3, f32) -> Option<f32>| placement_with(&LOOK, EYE, Vec3::X, ray).spot;
        assert_eq!(spot(open), DropSpot::Front);
        assert_eq!(
            spot(|_, direction, _| (direction == Vec3::X).then_some(7.0)),
            DropSpot::Side
        );
        assert_eq!(spot(|_, _, _| Some(1.0)), DropSpot::PulledBack);
        assert_eq!(
            placement_with(&LOOK, EYE, Vec3::ZERO, open).spot,
            DropSpot::Front
        );
    }

    #[test]
    fn a_wall_within_the_margin_beyond_the_spot_blocks_it_and_one_further_does_not() {
        let blocked = placement_with(&LOOK, EYE, Vec3::X, wall_ahead(14.0));
        assert_near(blocked.position, Vec3::new(10.0, SPAWN_Y, 44.0));
        let free = placement_with(&LOOK, EYE, Vec3::X, wall_ahead(14.1));
        assert_near(free.position, Vec3::new(22.0, SPAWN_Y, 30.0));
    }

    #[test]
    fn with_both_spots_blocked_the_item_is_pulled_back_along_the_front_ray() {
        let placed = placement_with(&LOOK, EYE, Vec3::X, |_, direction, _| {
            Some(if direction == Vec3::X { 7.0 } else { 3.0 })
        });
        assert_near(placed.position, Vec3::new(15.0, SPAWN_Y, 30.0));
    }

    #[test]
    fn a_front_wall_inside_the_margin_with_both_spots_blocked_leaves_the_item_at_the_body() {
        let placed = placement_with(&LOOK, EYE, Vec3::X, |_, _, _| Some(1.0));
        assert_near(placed.position, Vec3::new(10.0, SPAWN_Y, 30.0));
    }

    #[test]
    fn the_rays_start_on_the_body_axis_at_the_spawn_height_and_reach_a_margin_past_the_spot() {
        let mut asked = Vec::new();
        placement_with(&LOOK, EYE, Vec3::X, |origin, direction, reach| {
            asked.push((origin, direction, reach));
            Some(1.0)
        });
        assert_eq!(asked.len(), 2);
        let spawn = Vec3::new(10.0, SPAWN_Y, 30.0);
        assert_eq!(asked[0], (spawn, Vec3::X, 14.0));
        assert_near(asked[1].0, spawn);
        assert_near(asked[1].1, Vec3::Z);
        assert!((asked[1].2 - 16.0).abs() < 1e-4);
    }

    #[test]
    fn a_free_front_spot_asks_no_further_ray() {
        let mut asked = 0;
        placement_with(&LOOK, EYE, Vec3::X, |_, _, _| {
            asked += 1;
            None
        });
        assert_eq!(asked, 1);
    }

    #[test]
    fn the_side_spot_lies_to_the_right_and_just_behind_the_player_at_the_observed_angle() {
        let side = to_the_right(Vec3::X, OBSERVED_DROP.side_angle);
        assert!(side.z > 0.99 * OBSERVED_DROP.side_angle.sin());
        assert!(side.x < 0.0, "{side:?}");
        assert!((side.length() - 1.0).abs() < 1e-5);
        let turned = to_the_right(look_direction(1.3, 0.0), OBSERVED_DROP.side_angle);
        let expected = look_direction(1.3 + OBSERVED_DROP.side_angle, 0.0);
        assert_near(turned, expected);
    }

    #[test]
    fn the_view_direction_is_normalised_and_a_zero_view_places_the_item_below_the_eye() {
        let placed = placement_with(&LOOK, EYE, Vec3::new(0.0, 0.0, 5.0), open);
        assert_near(placed.position, Vec3::new(10.0, SPAWN_Y, 42.0));
        let none = placement_with(&LOOK, EYE, Vec3::ZERO, open);
        assert_near(none.position, Vec3::new(10.0, SPAWN_Y, 30.0));
        assert!(none.yaw.is_finite());
    }

    #[test]
    fn the_view_pitch_does_not_move_the_drop() {
        let level = placement_with(&LOOK, EYE, look_direction(0.7, 0.0), open);
        for pitch in [-1.2, -0.5, 0.5, 1.2] {
            let pitched = placement_with(&LOOK, EYE, look_direction(0.7, pitch), open);
            assert_near(pitched.position, level.position);
            assert!((pitched.yaw - level.yaw).abs() < 1e-4, "{pitch}");
        }
    }

    #[test]
    fn a_view_straight_up_or_down_drops_below_the_eye_with_a_finite_yaw() {
        for forward in [Vec3::Y, Vec3::NEG_Y] {
            let placed = placement_with(&LOOK, EYE, forward, |_, _, _| Some(1.0));
            assert_near(placed.position, Vec3::new(10.0, SPAWN_Y, 30.0));
            assert!(placed.yaw.is_finite());
        }
    }

    #[test]
    fn the_facing_is_the_horizontal_part_of_the_view_or_zero() {
        assert_near(facing(Vec3::new(3.0, 5.0, 0.0)), Vec3::X);
        assert_eq!(facing(Vec3::Y), Vec3::ZERO);
    }

    #[test]
    fn the_item_faces_the_view_even_when_it_lands_at_the_side() {
        for yaw in [-3.0_f32, -FRAC_PI_2, 0.0, 0.4, 2.9] {
            let forward = look_direction(yaw, 0.3);
            for blocked in [false, true] {
                let placed = placement_with(&LOOK, EYE, forward, |_, _, _| blocked.then_some(1.0));
                assert!((placed.yaw - yaw).abs() < 1e-4, "{yaw}: {}", placed.yaw);
            }
        }
    }

    #[test]
    fn the_observed_look_starts_the_item_within_the_activation_reach_so_it_can_be_taken_again() {
        let front = placement(EYE, Vec3::X, open);
        assert!(((front.position - EYE).x - OBSERVED_DROP.forward_distance).abs() < 1e-4);
        const { assert!(OBSERVED_DROP.forward_distance < ACTIVATE_REACH) };
        const { assert!(OBSERVED_DROP.side_distance < ACTIVATE_REACH) };
    }

    #[test]
    fn the_spec_carries_the_item_count_position_and_yaw_of_the_placement() {
        let placed = DropPlacement {
            position: Vec3::new(1.0, 2.0, 3.0),
            spot: DropSpot::Front,
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
}
