//! Which actors splat: those whose body the water surface cuts through (one rule
//! for actors, one for the player).

use crate::sim::water_physics::HAVOK_PER_GAME_UNIT;

/// An actor wades when the water surface lies above the body's lower bound and
/// below its top.
///
/// Units: the water height is compared in Havok units, so the game-unit body top
/// (`height + actor.z`) is scaled *down* by `HAVOK_PER_GAME_UNIT` to meet it.
///
/// Simplifications: the capsule soles stand in for the lower bound of the body's
/// physics proxy, and no minimum movement speed is required: a standing actor
/// draws a zero-length segment, which splats nothing.
pub fn is_wading(water_height: f32, feet_z: f32, body_height: f32) -> bool {
    let water_havok = water_height * HAVOK_PER_GAME_UNIT;
    let body_floor_havok = feet_z * HAVOK_PER_GAME_UNIT;
    let body_top_havok = (feet_z + body_height) * HAVOK_PER_GAME_UNIT;
    water_havok > body_floor_havok && body_top_havok > water_havok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standing_with_the_feet_in_the_water_is_wading() {
        assert!(is_wading(10.0, 0.0, 128.0));
    }

    #[test]
    fn standing_on_dry_ground_is_not() {
        assert!(!is_wading(10.0, 10.0, 128.0));
        assert!(!is_wading(10.0, 50.0, 128.0));
    }

    #[test]
    fn being_fully_below_the_surface_is_swimming_not_wading() {
        assert!(!is_wading(200.0, 0.0, 128.0));
    }

    #[test]
    fn the_body_top_is_compared_in_havok_units_against_the_water() {
        // Both sides carry the `0.0142875` factor, so the comparison is exactly
        // the game-unit one: the body top at 128.0 against water at 127.99.
        assert!(is_wading(127.99, 0.0, 128.0));
        assert!(
            !is_wading(128.0, 0.0, 128.0),
            "the top must be strictly above"
        );
    }

    #[test]
    fn the_havok_scale_is_the_games_not_one_over_seventy() {
        assert!((HAVOK_PER_GAME_UNIT.recip() - 69.991_25).abs() < 1.0e-3);
    }
}
