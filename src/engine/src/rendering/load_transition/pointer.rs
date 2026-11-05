//! How the window's pointer and focus events reach a large loading screen's model.

use super::LoadScreens;

impl LoadScreens {
    /// The pointer moved to `(x, y)`; it drags the model only between a press and its release.
    pub fn pointer_moved(&mut self, x: f32, y: f32) {
        if let Some((dx, dy)) = self.drag.moved(x, y) {
            self.host.drag(dx, dy);
        }
    }

    /// The primary button went down at `(x, y)`; it starts a drag only while the model is shown.
    pub fn pointer_pressed(&mut self, x: f32, y: f32, art_shown: bool) {
        if art_shown {
            self.drag.press(x, y);
        }
    }

    pub fn pointer_released(&mut self) {
        self.end_drag();
    }

    /// Lets go of the model: no drag is on and the model's spin counts down to resume.
    pub(super) fn end_drag(&mut self) {
        self.drag.release();
        self.host.end_drag();
    }

    /// The window lost the focus, and with it the releases of whatever was held.
    pub fn focus_lost(&mut self) {
        self.end_drag();
        self.host.release_keys();
    }
}
