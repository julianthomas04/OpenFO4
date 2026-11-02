//! The pointer drag that turns a loading screen's model.

/// Follows the primary button's drag across the window, in framebuffer pixels.
#[derive(Debug, Default)]
pub(in crate::rendering) struct DragTracker {
    last_x: Option<f32>,
}

impl DragTracker {
    pub fn press(&mut self, x: f32) {
        self.last_x = Some(x);
    }

    /// The horizontal travel since the press or the last move, to the right positive; `None`
    /// while no drag is on.
    pub fn moved(&mut self, x: f32) -> Option<f32> {
        let last = self.last_x.replace(x)?;
        Some(x - last)
    }

    pub fn release(&mut self) {
        self.last_x = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_without_a_press_travel_nowhere() {
        let mut drag = DragTracker::default();
        assert_eq!(drag.moved(40.0), None);
    }

    #[test]
    fn a_drag_reports_the_travel_between_moves() {
        let mut drag = DragTracker::default();
        drag.press(100.0);
        assert_eq!(drag.moved(130.0), Some(30.0));
        assert_eq!(drag.moved(120.0), Some(-10.0));
    }

    #[test]
    fn a_release_ends_the_drag_and_the_next_press_starts_from_its_own_position() {
        let mut drag = DragTracker::default();
        drag.press(100.0);
        drag.release();
        assert_eq!(drag.moved(500.0), None);
        drag.press(10.0);
        assert_eq!(drag.moved(15.0), Some(5.0));
    }
}
