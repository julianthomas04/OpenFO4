//! Window events to Pip-Boy input. Every binding lives in the tables below.

use egui::Pos2;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

use super::nav::Page;
use super::stage::StageFit;
use super::tab::{HintKey, NavInput, PointerButton};

/// What a key means while the Pip-Boy is open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Binding {
    Nav(NavInput),
    PrevPage,
    NextPage,
    /// Offered to the tab as its cancel key first; closes the Pip-Boy when the tab ignores it.
    Close,
}

/// An input the shell routes: a tab-level navigation input or a shell-level one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShellInput {
    Nav(NavInput),
    PrevPage,
    NextPage,
    Close,
}

impl From<Binding> for ShellInput {
    fn from(binding: Binding) -> Self {
        match binding {
            Binding::Nav(input) => ShellInput::Nav(input),
            Binding::PrevPage => ShellInput::PrevPage,
            Binding::NextPage => ShellInput::NextPage,
            Binding::Close => ShellInput::Close,
        }
    }
}

/// The key table while the Pip-Boy is open. Digits and `-`/`=` pick a favourites slot; the
/// hint keys are the ones the hint bar names.
const KEY_BINDINGS: &[(KeyCode, Binding)] = &[
    (KeyCode::Tab, Binding::Close),
    (KeyCode::Escape, Binding::Close),
    (KeyCode::KeyA, Binding::PrevPage),
    (KeyCode::KeyD, Binding::NextPage),
    (KeyCode::ArrowLeft, Binding::Nav(NavInput::PrevSubTab)),
    (KeyCode::ArrowRight, Binding::Nav(NavInput::NextSubTab)),
    (KeyCode::KeyW, Binding::Nav(NavInput::Up)),
    (KeyCode::ArrowUp, Binding::Nav(NavInput::Up)),
    (KeyCode::KeyS, Binding::Nav(NavInput::Down)),
    (KeyCode::ArrowDown, Binding::Nav(NavInput::Down)),
    (KeyCode::Enter, Binding::Nav(NavInput::Hint(HintKey::Enter))),
    (
        KeyCode::NumpadEnter,
        Binding::Nav(NavInput::Hint(HintKey::Enter)),
    ),
    (KeyCode::KeyE, Binding::Nav(NavInput::Hint(HintKey::E))),
    (KeyCode::KeyR, Binding::Nav(NavInput::Hint(HintKey::R))),
    (KeyCode::KeyT, Binding::Nav(NavInput::Hint(HintKey::T))),
    (KeyCode::KeyQ, Binding::Nav(NavInput::Hint(HintKey::Q))),
    (KeyCode::KeyC, Binding::Nav(NavInput::Hint(HintKey::C))),
    (KeyCode::KeyZ, Binding::Nav(NavInput::Hint(HintKey::Z))),
    (KeyCode::KeyX, Binding::Nav(NavInput::Hint(HintKey::X))),
    (
        KeyCode::ControlLeft,
        Binding::Nav(NavInput::Hint(HintKey::Ctrl)),
    ),
    (
        KeyCode::ControlRight,
        Binding::Nav(NavInput::Hint(HintKey::Ctrl)),
    ),
    (KeyCode::AltLeft, Binding::Nav(NavInput::Hint(HintKey::Alt))),
    (
        KeyCode::AltRight,
        Binding::Nav(NavInput::Hint(HintKey::Alt)),
    ),
    (KeyCode::Digit1, Binding::Nav(NavInput::Slot(0))),
    (KeyCode::Digit2, Binding::Nav(NavInput::Slot(1))),
    (KeyCode::Digit3, Binding::Nav(NavInput::Slot(2))),
    (KeyCode::Digit4, Binding::Nav(NavInput::Slot(3))),
    (KeyCode::Digit5, Binding::Nav(NavInput::Slot(4))),
    (KeyCode::Digit6, Binding::Nav(NavInput::Slot(5))),
    (KeyCode::Digit7, Binding::Nav(NavInput::Slot(6))),
    (KeyCode::Digit8, Binding::Nav(NavInput::Slot(7))),
    (KeyCode::Digit9, Binding::Nav(NavInput::Slot(8))),
    (KeyCode::Digit0, Binding::Nav(NavInput::Slot(9))),
    (KeyCode::Minus, Binding::Nav(NavInput::Slot(10))),
    (KeyCode::Equal, Binding::Nav(NavInput::Slot(11))),
];

/// Keys that keep acting while held.
const REPEATING: [NavInput; 2] = [NavInput::Up, NavInput::Down];

/// What a key does while the Pip-Boy is closed: open it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenKey {
    /// Open where it was last left.
    Last,
    Page(Page),
}

const OPEN_KEYS: &[(KeyCode, OpenKey)] = &[
    (KeyCode::Tab, OpenKey::Last),
    (KeyCode::KeyI, OpenKey::Page(Page::Inv)),
    (KeyCode::KeyJ, OpenKey::Page(Page::Data)),
    (KeyCode::KeyK, OpenKey::Page(Page::Stat)),
    (KeyCode::KeyM, OpenKey::Page(Page::Map)),
    (KeyCode::KeyO, OpenKey::Page(Page::Radio)),
];

pub fn open_key(code: KeyCode) -> Option<OpenKey> {
    OPEN_KEYS
        .iter()
        .find_map(|(key, open)| (*key == code).then_some(*open))
}

/// What a key event does while the Pip-Boy is closed. Only a fresh press opens it: the repeats of
/// a held close key must not reopen it right after it closed.
pub fn open_key_press(code: KeyCode, is_pressed: bool, is_repeat: bool) -> Option<OpenKey> {
    if is_pressed && !is_repeat {
        open_key(code)
    } else {
        None
    }
}

pub fn key_binding(code: KeyCode) -> Option<Binding> {
    KEY_BINDINGS
        .iter()
        .find_map(|(key, binding)| (*key == code).then_some(*binding))
}

/// Mouse-wheel pixels that count as one notch on touchpads.
const PIXELS_PER_NOTCH: f32 = 40.0;
/// The Scaleform mouse-wheel delta one notch produces.
pub const WHEEL_DELTA_PER_NOTCH: f32 = 1.0;

pub fn wheel_notches(delta: MouseScrollDelta) -> f32 {
    match delta {
        MouseScrollDelta::LineDelta(_, y) => y,
        MouseScrollDelta::PixelDelta(position) => position.y as f32 / PIXELS_PER_NOTCH,
    }
}

/// The result of one window event: whether the Pip-Boy swallowed it and the inputs it produced.
#[derive(Debug, Default, PartialEq)]
pub struct Translated {
    pub consumed: bool,
    pub inputs: Vec<ShellInput>,
}

/// Turns window events into shell inputs, tracking the pointer in stage pixels.
#[derive(Debug, Default)]
pub struct InputTranslator {
    /// Last cursor position in window pixels; survives the Pip-Boy closing and opening.
    cursor_px: Option<Pos2>,
    dragging: Option<PointerButton>,
    wheel_remainder: f32,
}

impl InputTranslator {
    /// The pointer in stage pixels under `fit`, once a cursor position has been seen.
    pub fn pointer(&self, fit: StageFit) -> Option<Pos2> {
        self.cursor_px.map(|px| fit.window_to_stage(px))
    }

    /// Forgets the button and wheel state of the last session; the cursor position stays.
    pub fn begin_session(&mut self) {
        self.dragging = None;
        self.wheel_remainder = 0.0;
    }

    /// Remembers the cursor while the Pip-Boy is closed so its first frame already knows it.
    pub fn observe(&mut self, event: &WindowEvent) {
        if let WindowEvent::CursorMoved { position, .. } = event {
            self.cursor_px = Some(Pos2::new(position.x as f32, position.y as f32));
        }
    }

    /// Forgets the cursor and any drag, as when the cursor leaves or the window loses focus.
    fn lose_pointer(&mut self) -> Option<ShellInput> {
        let had_cursor = self.cursor_px.take().is_some();
        let had_drag = self.dragging.take().is_some();
        (had_cursor || had_drag).then_some(ShellInput::Nav(NavInput::PointerLeft))
    }

    /// The first held button drives the drag; releasing it ends the drag, which is reported.
    fn track_drag(&mut self, button: PointerButton, pressed: bool) -> Option<ShellInput> {
        if pressed {
            self.dragging.get_or_insert(button);
        } else if self.dragging == Some(button) {
            self.dragging = None;
            return Some(ShellInput::Nav(NavInput::DragEnded(button)));
        }
        None
    }

    /// Adds `notches` to the sub-notch remainder and returns the whole notches now due.
    fn accumulate_wheel(&mut self, notches: f32) -> f32 {
        if notches * self.wheel_remainder < 0.0 {
            self.wheel_remainder = 0.0;
        }
        let total = self.wheel_remainder + notches;
        let whole = total.trunc();
        self.wheel_remainder = total - whole;
        whole
    }

    /// Keyboard, mouse button, wheel and cursor events are consumed; everything else (resize,
    /// redraw, close, focus) passes through untouched.
    pub fn translate(&mut self, event: &WindowEvent, fit: StageFit) -> Translated {
        let mut out = Translated::default();
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                out.consumed = true;
                if let (PhysicalKey::Code(code), ElementState::Pressed) =
                    (event.physical_key, event.state)
                    && let Some(binding) = key_binding(code)
                {
                    let input = ShellInput::from(binding);
                    let allowed_repeat =
                        matches!(input, ShellInput::Nav(nav) if REPEATING.contains(&nav));
                    if !event.repeat || allowed_repeat {
                        out.inputs.push(input);
                    }
                }
            }
            WindowEvent::CursorMoved { .. } => {
                out.consumed = true;
                let previous = self.pointer(fit);
                self.observe(event);
                if let Some(now) = self.pointer(fit) {
                    if let (Some(button), Some(previous)) = (self.dragging, previous) {
                        out.inputs.push(ShellInput::Nav(NavInput::Dragged {
                            delta: now - previous,
                            button,
                        }));
                    }
                    out.inputs
                        .push(ShellInput::Nav(NavInput::PointerMoved(now)));
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                out.consumed = true;
                let pressed = *state == ElementState::Pressed;
                match button {
                    MouseButton::Left => {
                        out.inputs
                            .extend(self.track_drag(PointerButton::Primary, pressed));
                        if let Some(at) = self.pointer(fit) {
                            out.inputs.push(ShellInput::Nav(if pressed {
                                NavInput::Clicked(at)
                            } else {
                                NavInput::Released(at)
                            }));
                        }
                    }
                    MouseButton::Right => out
                        .inputs
                        .extend(self.track_drag(PointerButton::Secondary, pressed)),
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                out.consumed = true;
                let whole = self.accumulate_wheel(wheel_notches(*delta));
                if whole != 0.0 {
                    out.inputs.push(ShellInput::Nav(NavInput::Scrolled(
                        whole * WHEEL_DELTA_PER_NOTCH,
                    )));
                }
            }
            WindowEvent::CursorEntered { .. } => out.consumed = true,
            WindowEvent::CursorLeft { .. } => {
                out.consumed = true;
                out.inputs.extend(self.lose_pointer());
            }
            WindowEvent::Focused(false) => out.inputs.extend(self.lose_pointer()),
            _ => {}
        }
        out
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "exact literals")]
mod tests {
    use super::*;
    use egui::Vec2;
    use winit::dpi::PhysicalPosition;
    use winit::event::DeviceId;

    fn fit() -> StageFit {
        StageFit {
            scale: 2.0,
            origin_px: Pos2::new(100.0, 0.0),
        }
    }

    fn cursor_at(x: f64, y: f64) -> WindowEvent {
        WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(x, y),
        }
    }

    fn left_button(state: ElementState) -> WindowEvent {
        WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Left,
        }
    }

    #[test]
    fn every_bound_key_appears_once() {
        for (index, (code, _)) in KEY_BINDINGS.iter().enumerate() {
            assert!(
                KEY_BINDINGS[index + 1..]
                    .iter()
                    .all(|(other, _)| other != code),
                "{code:?} is bound twice"
            );
        }
    }

    #[test]
    fn the_documented_keys_map_to_their_inputs() {
        let cases = [
            (KeyCode::KeyW, Binding::Nav(NavInput::Up)),
            (KeyCode::ArrowDown, Binding::Nav(NavInput::Down)),
            (KeyCode::KeyA, Binding::PrevPage),
            (KeyCode::KeyD, Binding::NextPage),
            (KeyCode::ArrowLeft, Binding::Nav(NavInput::PrevSubTab)),
            (KeyCode::Enter, Binding::Nav(NavInput::Hint(HintKey::Enter))),
            (
                KeyCode::ControlLeft,
                Binding::Nav(NavInput::Hint(HintKey::Ctrl)),
            ),
            (KeyCode::Minus, Binding::Nav(NavInput::Slot(10))),
            (KeyCode::Digit1, Binding::Nav(NavInput::Slot(0))),
            (KeyCode::Escape, Binding::Close),
        ];
        for (code, expected) in cases {
            assert_eq!(key_binding(code), Some(expected), "{code:?}");
        }
        assert_eq!(key_binding(KeyCode::KeyF), None);
    }

    #[test]
    fn the_twelve_favourite_slots_are_each_reachable() {
        let mut slots: Vec<u8> = KEY_BINDINGS
            .iter()
            .filter_map(|(_, binding)| match binding {
                Binding::Nav(NavInput::Slot(slot)) => Some(*slot),
                _ => None,
            })
            .collect();
        slots.sort_unstable();
        assert_eq!(slots, (0..12).collect::<Vec<u8>>());
    }

    #[test]
    fn open_keys_cover_tab_and_every_page_shortcut() {
        assert_eq!(open_key(KeyCode::Tab), Some(OpenKey::Last));
        assert_eq!(open_key(KeyCode::KeyI), Some(OpenKey::Page(Page::Inv)));
        assert_eq!(open_key(KeyCode::KeyJ), Some(OpenKey::Page(Page::Data)));
        assert_eq!(open_key(KeyCode::KeyK), Some(OpenKey::Page(Page::Stat)));
        assert_eq!(open_key(KeyCode::KeyM), Some(OpenKey::Page(Page::Map)));
        assert_eq!(open_key(KeyCode::KeyO), Some(OpenKey::Page(Page::Radio)));
        assert_eq!(open_key(KeyCode::KeyW), None);
    }

    #[test]
    fn a_repeating_or_released_open_key_does_not_open() {
        assert_eq!(
            open_key_press(KeyCode::Tab, true, false),
            Some(OpenKey::Last)
        );
        assert_eq!(open_key_press(KeyCode::Tab, true, true), None);
        assert_eq!(open_key_press(KeyCode::KeyI, false, false), None);
        assert_eq!(open_key_press(KeyCode::KeyW, true, false), None);
    }

    #[test]
    fn wheel_lines_pass_through_and_pixels_become_notches() {
        assert!((wheel_notches(MouseScrollDelta::LineDelta(0.0, 2.0)) - 2.0).abs() < 1e-6);
        let pixels = MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -80.0));
        assert!((wheel_notches(pixels) + 2.0).abs() < 1e-6);
    }

    #[test]
    fn the_cursor_is_reported_in_stage_pixels_and_consumed() {
        let mut translator = InputTranslator::default();
        let result = translator.translate(&cursor_at(300.0, 40.0), fit());

        assert!(result.consumed);
        assert_eq!(
            result.inputs,
            vec![ShellInput::Nav(NavInput::PointerMoved(Pos2::new(
                100.0, 20.0
            )))]
        );
    }

    #[test]
    fn a_click_lands_at_the_last_pointer_position_and_a_held_button_drags() {
        let mut translator = InputTranslator::default();
        translator.translate(&cursor_at(300.0, 40.0), fit());

        let press = translator.translate(&left_button(ElementState::Pressed), fit());
        let moved = translator.translate(&cursor_at(310.0, 40.0), fit());
        let release = translator.translate(&left_button(ElementState::Released), fit());
        let after = translator.translate(&cursor_at(320.0, 40.0), fit());

        assert_eq!(
            press.inputs,
            vec![ShellInput::Nav(NavInput::Clicked(Pos2::new(100.0, 20.0)))]
        );
        assert_eq!(
            moved.inputs[0],
            ShellInput::Nav(NavInput::Dragged {
                delta: Vec2::new(5.0, 0.0),
                button: PointerButton::Primary,
            })
        );
        assert!(release.consumed);
        assert_eq!(
            release.inputs,
            vec![
                ShellInput::Nav(NavInput::DragEnded(PointerButton::Primary)),
                ShellInput::Nav(NavInput::Released(Pos2::new(105.0, 20.0))),
            ]
        );
        assert!(
            !after
                .inputs
                .iter()
                .any(|input| matches!(input, ShellInput::Nav(NavInput::Dragged { .. })))
        );
    }

    #[test]
    fn a_held_right_button_drags_without_clicking() {
        let mut translator = InputTranslator::default();
        translator.translate(&cursor_at(300.0, 40.0), fit());
        let right = |state| WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Right,
        };

        let press = translator.translate(&right(ElementState::Pressed), fit());
        let moved = translator.translate(&cursor_at(310.0, 40.0), fit());
        let release = translator.translate(&right(ElementState::Released), fit());
        let after = translator.translate(&cursor_at(320.0, 40.0), fit());

        assert!(press.consumed && press.inputs.is_empty());
        assert_eq!(
            moved.inputs[0],
            ShellInput::Nav(NavInput::Dragged {
                delta: Vec2::new(5.0, 0.0),
                button: PointerButton::Secondary,
            })
        );
        assert_eq!(
            release.inputs,
            vec![ShellInput::Nav(NavInput::DragEnded(
                PointerButton::Secondary
            ))]
        );
        assert_eq!(after.inputs.len(), 1);
    }

    #[test]
    fn small_wheel_deltas_accumulate_into_whole_notches_without_loss() {
        let mut translator = InputTranslator::default();
        let wheel = |pixels: f64| WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, pixels)),
            phase: winit::event::TouchPhase::Moved,
        };

        let first = translator.translate(&wheel(30.0), fit());
        let second = translator.translate(&wheel(30.0), fit());
        let third = translator.translate(&wheel(10.0), fit());
        let fourth = translator.translate(&wheel(20.0), fit());

        assert_eq!(first.inputs, []);
        assert_eq!(
            second.inputs,
            vec![ShellInput::Nav(NavInput::Scrolled(1.0))]
        );
        assert_eq!(third.inputs, []);
        assert_eq!(
            fourth.inputs,
            vec![ShellInput::Nav(NavInput::Scrolled(1.0))]
        );
    }

    #[test]
    fn reversing_the_wheel_drops_the_remainder_of_the_other_direction() {
        let mut translator = InputTranslator::default();
        assert_eq!(translator.accumulate_wheel(0.75), 0.0);
        assert_eq!(translator.accumulate_wheel(-0.5), 0.0);
        assert_eq!(translator.accumulate_wheel(-0.5), -1.0);
    }

    #[test]
    fn the_cursor_seen_while_closed_is_the_pointer_after_opening() {
        let mut translator = InputTranslator::default();
        translator.observe(&cursor_at(300.0, 40.0));
        translator.begin_session();

        assert_eq!(translator.pointer(fit()), Some(Pos2::new(100.0, 20.0)));
        let press = translator.translate(&left_button(ElementState::Pressed), fit());
        assert_eq!(
            press.inputs,
            vec![ShellInput::Nav(NavInput::Clicked(Pos2::new(100.0, 20.0)))]
        );
    }

    #[test]
    fn a_click_before_the_pointer_was_seen_is_consumed_without_an_input() {
        let mut translator = InputTranslator::default();
        let result = translator.translate(&left_button(ElementState::Pressed), fit());
        assert!(result.consumed && result.inputs.is_empty());
    }

    #[test]
    fn resize_and_redraw_pass_through() {
        let mut translator = InputTranslator::default();
        for event in [
            WindowEvent::RedrawRequested,
            WindowEvent::Resized(winit::dpi::PhysicalSize::new(10, 10)),
            WindowEvent::CloseRequested,
            WindowEvent::Focused(true),
        ] {
            assert_eq!(translator.translate(&event, fit()), Translated::default());
        }
    }

    #[test]
    fn leaving_the_window_or_losing_focus_forgets_the_cursor_and_the_drag() {
        for event in [
            WindowEvent::CursorLeft {
                device_id: DeviceId::dummy(),
            },
            WindowEvent::Focused(false),
        ] {
            let mut translator = InputTranslator::default();
            translator.translate(&cursor_at(300.0, 40.0), fit());
            translator.translate(&left_button(ElementState::Pressed), fit());

            let result = translator.translate(&event, fit());

            assert_eq!(result.inputs, vec![ShellInput::Nav(NavInput::PointerLeft)]);
            assert_eq!(translator.pointer(fit()), None);
            let after = translator.translate(&cursor_at(310.0, 40.0), fit());
            assert_eq!(after.inputs.len(), 1, "no drag survives");
        }
    }
}
