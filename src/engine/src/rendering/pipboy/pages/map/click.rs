//! Tells a click from a drag on the map: a press starts it, drag deltas accumulate travel while
//! the button is held, and the release resolves it. A press that barely moved is a click.

use egui::{Pos2, Vec2};

/// Pointer travel, in stage pixels, up to which a press still counts as a click.
const CLICK_TRAVEL: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Press {
    at: Pos2,
    travel: f32,
    on_map: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClickTracker {
    held: Option<Press>,
}

impl ClickTracker {
    /// The button went down at `at`. `on_map` says whether the press started on the map; only
    /// those can pan and click.
    pub fn press(&mut self, at: Pos2, on_map: bool) {
        self.held = Some(Press {
            at,
            travel: 0.0,
            on_map,
        });
    }

    /// A drag delta. Returns whether the map should pan by it.
    pub fn drag(&mut self, delta: Vec2) -> bool {
        match &mut self.held {
            Some(press) => {
                press.travel += delta.length();
                press.on_map
            }
            None => false,
        }
    }

    /// Whether a press that started on the map has moved: the map is being dragged.
    pub fn moving(&self) -> bool {
        self.held
            .is_some_and(|press| press.on_map && press.travel > 0.0)
    }

    /// The button went up. Returns where the click happened when the press started on the map
    /// and barely moved.
    pub fn release(&mut self) -> Option<Pos2> {
        let press = self.held.take()?;
        (press.on_map && press.travel <= CLICK_TRAVEL).then_some(press.at)
    }

    /// Forgets a held press, as when the Pip-Boy closed under it.
    pub fn cancel(&mut self) {
        self.held = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: Pos2 = Pos2::new(100.0, 100.0);

    #[test]
    fn a_press_released_in_place_clicks_where_it_started() {
        let mut tracker = ClickTracker::default();
        tracker.press(AT, true);
        assert_eq!(tracker.release(), Some(AT));
        assert_eq!(tracker.release(), None);
    }

    #[test]
    fn a_drag_pans_and_never_clicks() {
        let mut tracker = ClickTracker::default();
        tracker.press(AT, true);
        for _ in 0..5 {
            assert!(tracker.drag(Vec2::new(5.0, 0.0)));
        }
        assert_eq!(tracker.release(), None);
    }

    #[test]
    fn a_jitter_inside_the_click_travel_still_clicks() {
        let mut tracker = ClickTracker::default();
        tracker.press(AT, true);
        assert!(tracker.drag(Vec2::new(1.0, 1.0)));
        assert_eq!(tracker.release(), Some(AT));
    }

    #[test]
    fn a_press_outside_the_map_neither_pans_nor_clicks() {
        let mut tracker = ClickTracker::default();
        tracker.press(AT, false);
        assert!(!tracker.drag(Vec2::new(30.0, 0.0)));
        assert_eq!(tracker.release(), None);
    }

    #[test]
    fn the_map_is_moving_once_a_press_on_it_has_travelled() {
        let mut tracker = ClickTracker::default();
        assert!(!tracker.moving());
        tracker.press(AT, true);
        assert!(!tracker.moving());
        tracker.drag(Vec2::new(1.0, 0.0));
        assert!(tracker.moving());
        tracker.release();
        assert!(!tracker.moving());
    }

    #[test]
    fn a_release_without_a_press_is_nothing() {
        assert_eq!(ClickTracker::default().release(), None);
    }

    #[test]
    fn a_cancelled_press_does_not_click_on_a_later_release() {
        let mut tracker = ClickTracker::default();
        tracker.press(AT, true);
        tracker.cancel();
        assert_eq!(tracker.release(), None);
    }

    #[test]
    fn dragging_with_nothing_held_does_not_pan() {
        assert!(!ClickTracker::default().drag(Vec2::new(10.0, 0.0)));
    }
}
