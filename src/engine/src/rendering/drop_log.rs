//! The log lines that follow one dropped stack from its request to the world or back to the
//! inventory. Positions are in game units, headings in compass degrees.

use std::fmt::Display;

use glam::Vec3;

use super::item_drop::{DropPlacement, DropSpot};
use super::world_streamer::DroppedItemSpec;
use crate::bethesda_space::{compass_heading_degrees, position_from_engine};
use crate::sim::ItemGrantError;

fn game_position(engine_position: Vec3) -> String {
    let position = position_from_engine(engine_position);
    format!("({:.1},{:.1},{:.1})", position.x, position.y, position.z)
}

fn spot_name(spot: DropSpot) -> String {
    match spot {
        DropSpot::Toward(angle) => format!("{:.0}", angle.to_degrees()),
        DropSpot::Behind => "behind".to_owned(),
    }
}

/// The request: where the stack was placed relative to the player's eye.
pub(super) fn drop_request_line(
    form_key: &str,
    count: u32,
    placement: &DropPlacement,
    player: Vec3,
) -> String {
    format!(
        "item drop: base={form_key} count={count} spot={} pos={} player={} yaw={:.1}",
        spot_name(placement.spot),
        game_position(placement.position),
        game_position(player),
        compass_heading_degrees(placement.yaw),
    )
}

/// The streamer joined the built item to its cell.
pub(super) fn drop_in_world_line(ref_key: &str, spec: &DroppedItemSpec) -> String {
    format!(
        "item drop: in world ref={ref_key} base={} count={} pos={}",
        spec.base,
        spec.count,
        game_position(spec.position),
    )
}

/// The one line of a drop that did not reach the world: why, and whether the units went back.
pub(super) fn drop_failure_line(
    form_key: &str,
    count: u32,
    reason: &impl Display,
    returned: &Result<(), ItemGrantError>,
) -> String {
    let fate = match returned {
        Ok(()) => format!("{count} returned to the inventory"),
        Err(error) => format!("{count} NOT returned to the inventory: {error}"),
    };
    format!("item drop: not placed base={form_key} count={count} reason={reason}; {fate}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "0F3B01:Test.esm";

    #[test]
    fn the_request_line_names_item_spot_positions_and_heading_in_game_units() {
        let placement = DropPlacement {
            position: Vec3::new(10.0, 2.0, -30.0),
            spot: DropSpot::Behind,
            yaw: -std::f32::consts::FRAC_PI_2,
        };
        assert_eq!(
            drop_request_line(KEY, 30, &placement, Vec3::new(1.25, 3.0, -3.0)),
            "item drop: base=0F3B01:Test.esm count=30 spot=behind \
             pos=(100.0,300.0,20.0) player=(12.5,30.0,30.0) yaw=0.0"
        );
    }

    #[test]
    fn a_free_direction_is_named_by_its_clockwise_degrees() {
        let names =
            [0.0_f32, 36.0, 324.0].map(|degrees| spot_name(DropSpot::Toward(degrees.to_radians())));
        assert_eq!(names, ["0", "36", "324"]);
    }

    #[test]
    fn the_in_world_line_names_reference_item_count_and_position() {
        let spec = DroppedItemSpec {
            base: KEY.to_owned(),
            count: 2,
            position: Vec3::new(0.05, 1.0, -0.25),
            yaw: 0.0,
        };
        assert_eq!(
            drop_in_world_line("000001:*dropped", &spec),
            "item drop: in world ref=000001:*dropped base=0F3B01:Test.esm count=2 \
             pos=(0.5,2.5,10.0)"
        );
    }

    #[test]
    fn the_failure_line_states_reason_and_that_the_units_went_back() {
        assert_eq!(
            drop_failure_line(KEY, 3, &"its cell left before the item was built", &Ok(())),
            "item drop: not placed base=0F3B01:Test.esm count=3 \
             reason=its cell left before the item was built; 3 returned to the inventory"
        );
    }

    #[test]
    fn the_failure_line_says_so_when_the_units_could_not_go_back() {
        let refused = Err(ItemGrantError::UnknownItem {
            form_key: KEY.to_owned(),
        });
        let line = drop_failure_line(KEY, 3, &"no body", &refused);
        assert!(
            line.contains("reason=no body; 3 NOT returned to the inventory: "),
            "{line}"
        );
    }
}
