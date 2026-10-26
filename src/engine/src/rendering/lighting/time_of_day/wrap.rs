pub(super) const HOURS_PER_DAY: f32 = 24.0;
const DEGREES_PER_TURN: f32 = 360.0;

fn wrap(value: f32, period: f32) -> f32 {
    let wrapped = value.rem_euclid(period);
    if wrapped >= period { 0.0 } else { wrapped }
}

/// Maps a finite hour offset onto `[0, 24)`.
pub(super) fn wrap_hours(hours: f32) -> f32 {
    wrap(hours, HOURS_PER_DAY)
}

pub(super) fn wrap_degrees(degrees: f32) -> f32 {
    wrap(degrees, DEGREES_PER_TURN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hours_wrap_into_a_day_in_both_directions() {
        assert!((wrap_hours(25.5) - 1.5).abs() < 1e-5);
        assert!((wrap_hours(-1.0) - 23.0).abs() < 1e-5);
        assert!((wrap_hours(48.0)).abs() < 1e-5);
    }

    #[test]
    fn a_hair_below_zero_never_wraps_to_exactly_a_full_period() {
        let wrapped = wrap_hours(-1e-9);
        assert!((0.0..24.0).contains(&wrapped), "{wrapped}");
        let degrees = wrap_degrees(-1e-9);
        assert!((0.0..360.0).contains(&degrees), "{degrees}");
    }
}
