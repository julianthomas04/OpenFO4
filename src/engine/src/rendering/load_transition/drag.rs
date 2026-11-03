//! The pointer drag that turns and zooms a loading screen's model.

/// Follows the primary button's drag across the window, in framebuffer pixels.
#[derive(Debug, Default)]
pub(in crate::rendering) struct DragTracker {
    last: Option<(f32, f32)>,
}

impl DragTracker {
    pub fn press(&mut self, x: f32, y: f32) {
        self.last = Some((x, y));
    }

    /// The travel since the press or the last move, `(dx, dy)` with right and down positive;
    /// `None` while no drag is on.
    pub fn moved(&mut self, x: f32, y: f32) -> Option<(f32, f32)> {
        let (last_x, last_y) = self.last.as_mut()?;
        let travel = (x - *last_x, y - *last_y);
        (*last_x, *last_y) = (x, y);
        Some(travel)
    }

    pub fn release(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_without_a_press_travel_nowhere() {
        let mut drag = DragTracker::default();
        assert_eq!(drag.moved(40.0, 10.0), None);
    }

    #[test]
    fn repeated_moves_without_a_press_never_start_a_drag() {
        let mut drag = DragTracker::default();
        assert_eq!(drag.moved(40.0, 10.0), None);
        assert_eq!(drag.moved(60.0, 30.0), None);
        assert_eq!(drag.moved(80.0, 50.0), None);
    }

    #[test]
    fn a_drag_reports_the_travel_between_moves_in_both_directions() {
        let mut drag = DragTracker::default();
        drag.press(100.0, 50.0);
        assert_eq!(drag.moved(130.0, 40.0), Some((30.0, -10.0)));
        assert_eq!(drag.moved(120.0, 45.0), Some((-10.0, 5.0)));
    }

    #[test]
    fn a_release_ends_the_drag_and_the_next_press_starts_from_its_own_position() {
        let mut drag = DragTracker::default();
        drag.press(100.0, 100.0);
        drag.release();
        assert_eq!(drag.moved(500.0, 500.0), None);
        drag.press(10.0, 20.0);
        assert_eq!(drag.moved(15.0, 20.0), Some((5.0, 0.0)));
    }
}
