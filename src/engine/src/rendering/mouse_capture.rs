use winit::window::{CursorGrabMode, Window};

use super::camera::CameraController;

/// Mouse capture: while locked the cursor is hidden and confined to the window
/// and raw mouse motion steers the camera. Esc toggles it.
///
/// A frozen camera (`OPENFO4_FREEZE_CAMERA`) starts released, so mouse motion
/// does not turn the view until Esc locks the capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseCapture {
    locked: bool,
}

impl MouseCapture {
    pub fn new(camera_frozen: bool) -> Self {
        Self {
            locked: !camera_frozen,
        }
    }

    pub fn is_locked(self) -> bool {
        self.locked
    }

    pub fn toggle(&mut self) {
        self.locked = !self.locked;
    }

    pub fn set_locked(&mut self, locked: bool) {
        self.locked = locked;
    }

    /// Turns raw mouse motion into camera look while the capture is locked.
    pub fn steer(self, controller: &mut CameraController, dx: f64, dy: f64) {
        if self.locked {
            controller.handle_mouse(dx, dy);
        }
    }

    /// Mirrors the lock state onto the window's cursor visibility and grab.
    pub fn apply(self, window: &Window) {
        window.set_cursor_visible(!self.locked);
        let grab = if self.locked {
            CursorGrabMode::Confined
        } else {
            CursorGrabMode::None
        };
        if let Err(e) = window.set_cursor_grab(grab) {
            tracing::warn!("cursor grab {grab:?} failed: {e}");
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "a released capture must leave the look bit-for-bit untouched"
)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::rendering::camera::parse_spawn_look;

    fn controller_at_spawn_look(raw: &str) -> CameraController {
        let (yaw, pitch) = parse_spawn_look(raw).expect("valid spawn look");
        let mut controller = CameraController::new(10.0, 0.01);
        controller.set_look(yaw, pitch);
        controller
    }

    fn direction(yaw_deg: f32, pitch_deg: f32) -> Vec3 {
        let (yaw, pitch) = (yaw_deg.to_radians(), pitch_deg.to_radians());
        Vec3::new(
            yaw.cos() * pitch.cos(),
            pitch.sin(),
            yaw.sin() * pitch.cos(),
        )
    }

    #[test]
    fn an_unfrozen_capture_starts_locked_and_motion_steers_the_camera() {
        let capture = MouseCapture::new(false);
        let mut controller = controller_at_spawn_look("30,-25");
        let before = controller.yaw();
        capture.steer(&mut controller, 40.0, 10.0);
        assert!(capture.is_locked());
        assert!(controller.yaw() > before);
    }

    #[test]
    fn a_frozen_capture_starts_released_and_the_pose_is_exactly_the_spawn_look() {
        let capture = MouseCapture::new(true);
        let mut controller = controller_at_spawn_look("30,-25");
        let spawn_forward = controller.forward();
        let spawn_yaw = controller.yaw();

        for (dx, dy) in [(250.0, -400.0), (-1e6, 1e6), (0.5, 0.5)] {
            capture.steer(&mut controller, dx, dy);
        }

        assert!(!capture.is_locked());
        assert_eq!(controller.yaw(), spawn_yaw);
        assert_eq!(controller.forward(), spawn_forward);
        assert!(spawn_forward.abs_diff_eq(direction(30.0, -25.0), 1e-6));
    }

    #[test]
    fn escape_toggles_an_unfrozen_capture() {
        let mut capture = MouseCapture::new(false);
        capture.toggle();
        assert!(!capture.is_locked());
        capture.toggle();
        assert!(capture.is_locked());
    }

    #[test]
    fn escape_locks_a_frozen_capture_and_motion_then_turns_the_view() {
        let mut capture = MouseCapture::new(true);
        let mut controller = controller_at_spawn_look("30,-25");
        let spawn_yaw = controller.yaw();

        capture.toggle();
        capture.steer(&mut controller, 300.0, 0.0);

        assert!(capture.is_locked());
        assert!(controller.yaw() > spawn_yaw);
    }

    #[test]
    fn setting_the_lock_is_idempotent_and_independent_of_the_previous_state() {
        for start_frozen in [false, true] {
            let mut capture = MouseCapture::new(start_frozen);
            capture.set_locked(true);
            capture.set_locked(true);
            assert!(capture.is_locked());
            capture.set_locked(false);
            capture.set_locked(false);
            assert!(!capture.is_locked());
        }
    }

    #[test]
    fn a_capture_released_by_set_locked_ignores_motion_until_locked_again() {
        let mut capture = MouseCapture::new(false);
        let mut controller = controller_at_spawn_look("0,0");
        capture.set_locked(false);
        capture.steer(&mut controller, 100.0, 100.0);
        assert_eq!(controller.yaw(), 0.0);
        capture.set_locked(true);
        capture.steer(&mut controller, 100.0, 100.0);
        assert!(controller.yaw() > 0.0);
    }

    #[test]
    fn a_released_capture_ignores_motion_until_locked_again() {
        let mut capture = MouseCapture::new(false);
        let mut controller = controller_at_spawn_look("0,0");
        capture.toggle();
        capture.steer(&mut controller, 100.0, 100.0);
        assert_eq!(controller.yaw(), 0.0);
        capture.toggle();
        capture.steer(&mut controller, 100.0, 100.0);
        assert!(controller.yaw() > 0.0);
    }
}
