//! The moment the freshly built renderer state takes over from the boot screen.

use winit::dpi::PhysicalSize;

use crate::rendering::{State, debug_ui::ToastType};

pub type StartupToasts = Vec<(String, ToastType)>;

/// What the handoff needs from the renderer state.
pub trait HandoffTarget {
    fn resize(&mut self, width: u32, height: u32);
    fn is_surface_configured(&self) -> bool;
    fn queue_toast(&mut self, message: String, toast_type: ToastType);
}

impl HandoffTarget for State {
    fn resize(&mut self, width: u32, height: u32) {
        State::resize(self, width, height);
    }

    fn is_surface_configured(&self) -> bool {
        State::is_surface_configured(self)
    }

    fn queue_toast(&mut self, message: String, toast_type: ToastType) {
        State::queue_toast(self, message, toast_type);
    }
}

/// The window may have been resized while the worker ran; the new state still has the size it
/// was built for. Toasts collected before the state existed are delivered, once.
pub fn finish_boot(
    target: &mut impl HandoffTarget,
    window_size: PhysicalSize<u32>,
    toasts: &mut StartupToasts,
) {
    target.resize(window_size.width, window_size.height);
    for (message, toast_type) in toasts.drain(..) {
        target.queue_toast(message, toast_type);
    }
}

/// A zero-sized window at the handoff leaves the surface unconfigured; configures it as soon as
/// the window has a size.
pub fn configure_pending_surface(target: &mut impl HandoffTarget, window_size: PhysicalSize<u32>) {
    if !target.is_surface_configured() {
        target.resize(window_size.width, window_size.height);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingTarget {
        resizes: Vec<(u32, u32)>,
        toasts: Vec<(String, ToastType)>,
        configured: bool,
    }

    impl HandoffTarget for RecordingTarget {
        fn resize(&mut self, width: u32, height: u32) {
            self.resizes.push((width, height));
            self.configured = width > 0 && height > 0;
        }

        fn is_surface_configured(&self) -> bool {
            self.configured
        }

        fn queue_toast(&mut self, message: String, toast_type: ToastType) {
            self.toasts.push((message, toast_type));
        }
    }

    fn pending_toasts() -> StartupToasts {
        vec![
            ("log file missing".to_owned(), ToastType::Error),
            ("fell back to defaults".to_owned(), ToastType::Warning),
        ]
    }

    #[test]
    fn the_new_state_is_resized_to_the_current_window() {
        let mut target = RecordingTarget::default();
        finish_boot(&mut target, PhysicalSize::new(1280, 720), &mut Vec::new());
        assert_eq!(target.resizes, [(1280, 720)]);
        assert!(target.configured);
    }

    #[test]
    fn startup_toasts_reach_the_state_in_order_and_are_handed_over_once() {
        let mut target = RecordingTarget::default();
        let mut toasts = pending_toasts();
        finish_boot(&mut target, PhysicalSize::new(800, 600), &mut toasts);
        assert_eq!(
            target.toasts,
            [
                ("log file missing".to_owned(), ToastType::Error),
                ("fell back to defaults".to_owned(), ToastType::Warning),
            ]
        );
        assert_eq!(toasts, []);
    }

    #[test]
    fn a_zero_sized_window_leaves_the_surface_pending_until_it_has_a_size() {
        let mut target = RecordingTarget::default();
        finish_boot(&mut target, PhysicalSize::new(0, 0), &mut Vec::new());
        assert!(!target.is_surface_configured());

        configure_pending_surface(&mut target, PhysicalSize::new(0, 0));
        assert!(!target.is_surface_configured());

        configure_pending_surface(&mut target, PhysicalSize::new(1024, 768));
        assert!(target.is_surface_configured());
        assert_eq!(target.resizes.last(), Some(&(1024, 768)));
    }

    #[test]
    fn a_configured_surface_is_not_reconfigured_every_frame() {
        let mut target = RecordingTarget::default();
        finish_boot(&mut target, PhysicalSize::new(640, 480), &mut Vec::new());
        configure_pending_surface(&mut target, PhysicalSize::new(640, 480));
        assert_eq!(target.resizes.len(), 1);
    }
}
