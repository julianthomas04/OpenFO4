//! A diagnostic line for the body a dropped item received in the physics world. Logged at trace
//! level under its own target.

use glam::Vec3;

pub(super) const DROP_BODY_TRACE_TARGET: &str = "openfo4::drop_body_trace";

pub(super) fn dropped_body_line(ref_key: &str, colliders: usize, position: Vec3) -> String {
    format!(
        "sim: dropped item body entity={ref_key} colliders={colliders} pos=({:.3},{:.3},{:.3})",
        position.x, position.y, position.z
    )
}

pub(super) fn dropped_without_body_line(ref_key: &str) -> String {
    format!("sim: dropped item body entity={ref_key} colliders=0 pos=none (no body was created)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_names_the_entity_collider_count_and_engine_position() {
        assert_eq!(
            dropped_body_line("000001:*dropped", 3, Vec3::new(1.0, 2.5, -3.25)),
            "sim: dropped item body entity=000001:*dropped colliders=3 pos=(1.000,2.500,-3.250)"
        );
    }

    #[test]
    fn a_drop_that_got_no_body_says_so() {
        assert_eq!(
            dropped_without_body_line("000001:*dropped"),
            "sim: dropped item body entity=000001:*dropped colliders=0 pos=none (no body was created)"
        );
    }
}
