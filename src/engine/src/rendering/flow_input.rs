//! The player-facing flow's glue on [`State`]: window input to menu events, flow actions to
//! engine effects, and the load readiness feed.

use winit::keyboard::KeyCode;

use super::stream_settle::{self, ColliderSettle, StreamReport};
use super::{ResolvedTarget, State};
use crate::game_flow::{FlowAction, FlowEvent, Phase, SpawnLocation, StreamCounts};

/// The menu input a key press stands for, if any. The operating system's key repeat moves the
/// selection but never activates a row or goes back: holding Enter must not carry through the
/// screen it opened.
fn menu_event_for_key(code: KeyCode, is_repeat: bool) -> Option<FlowEvent> {
    let event = match code {
        KeyCode::ArrowUp | KeyCode::KeyW => FlowEvent::NavUp,
        KeyCode::ArrowDown | KeyCode::KeyS => FlowEvent::NavDown,
        KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => FlowEvent::Activate,
        KeyCode::Escape | KeyCode::Backspace => FlowEvent::Back,
        _ => return None,
    };
    let repeats_harmlessly = matches!(event, FlowEvent::NavUp | FlowEvent::NavDown);
    (!is_repeat || repeats_harmlessly).then_some(event)
}

impl State {
    pub fn handle_mouse_motion(&mut self, dx: f64, dy: f64) {
        if self.flow.gameplay_input_enabled() {
            self.mouse_capture
                .steer(&mut self.camera_controller, dx, dy);
        }
    }

    pub fn handle_pointer_moved(&mut self, x: f64, y: f64) {
        self.pointer_px = (x as f32, y as f32);
        if let Some(row) = self.row_under_pointer() {
            self.handle_flow_event(FlowEvent::RowHovered(row));
        }
    }

    pub fn handle_pointer_pressed(&mut self) {
        if let Some(row) = self.row_under_pointer() {
            self.handle_flow_event(FlowEvent::RowPressed(row));
        }
    }

    pub fn handle_pointer_released(&mut self) {
        let row = self.row_under_pointer();
        self.handle_flow_event(FlowEvent::RowReleased(row));
    }

    /// `lines` is the wheel's travel in lines, positive away from the user.
    pub fn handle_wheel(&mut self, lines: f32) {
        self.flow.scroll(lines);
    }

    /// Whether the developer window is shown and so takes pointer and keyboard input.
    pub fn developer_window_visible(&self) -> bool {
        self.flow.developer_window_visible()
    }

    /// Whether the player asked to quit from the main menu.
    pub fn quit_requested(&self) -> bool {
        self.quit_requested
    }

    fn row_under_pointer(&self) -> Option<usize> {
        let list = self.flow.menu_view()?;
        self.menu.layout().row_at(
            self.config.width,
            self.config.height,
            list.first_visible,
            self.pointer_px.0,
            self.pointer_px.1,
        )
    }

    fn handle_flow_event(&mut self, event: FlowEvent) {
        let action = self.flow.handle(event);
        self.dispatch(action);
    }

    pub(super) fn feed_load_readiness(
        &mut self,
        settled: ColliderSettle,
        full_ring_complete: bool,
    ) {
        if self.flow.phase() != Phase::Loading {
            return;
        }
        let report = StreamReport {
            full_ring_complete,
            counts: StreamCounts {
                pending_cells: self.streamer.pending_cell_count(),
                pending_lod_cells: self.streamer.lod_pending_cell_count(),
                loaded_cells: self.streamer.loaded_cell_count(),
                loaded_lod_cells: self.streamer.lod_loaded_cell_count(),
            },
            spawn_cell: self.load_spawn_cell,
        };
        let readiness = stream_settle::readiness(settled, &self.sim, report);
        let action = self.flow.readiness(readiness);
        self.dispatch(action);
    }

    fn dispatch(&mut self, action: FlowAction) {
        match action {
            FlowAction::None => {}
            FlowAction::BeginPlay(location) => self.begin_play(&location),
            FlowAction::ReleaseFoldHold => self.sim.set_fold_hold(false),
            FlowAction::LockCursor => {
                self.mouse_capture.set_locked(!self.freeze_camera);
                self.mouse_capture.apply(&self.window);
            }
            FlowAction::Quit => self.quit_requested = true,
        }
    }

    /// Starts the load into the picked spawn: the streamer switches to its worldspace on the
    /// next `update`, and the physics fold stays held until the load completes.
    fn begin_play(&mut self, location: &SpawnLocation) {
        let target = ResolvedTarget::picked_spawn(location);
        self.sim.set_fold_hold(true);
        self.load_spawn_cell = target.grid;
        self.pending_teleport = Some(target);
    }

    /// Keys outside gameplay drive the menu list.
    pub(super) fn handle_menu_key(&mut self, code: KeyCode, is_pressed: bool, is_repeat: bool) {
        if !is_pressed {
            return;
        }
        if let Some(event) = menu_event_for_key(code, is_repeat) {
            self.handle_flow_event(event);
        }
    }
}

#[cfg(test)]
mod menu_input_tests {
    use super::*;

    const ACTIVATING_KEYS: [KeyCode; 3] = [KeyCode::Enter, KeyCode::NumpadEnter, KeyCode::Space];

    #[test]
    fn arrows_and_the_walk_keys_navigate_the_menu() {
        let key = |code| menu_event_for_key(code, false);
        assert_eq!(key(KeyCode::ArrowUp), Some(FlowEvent::NavUp));
        assert_eq!(key(KeyCode::KeyW), Some(FlowEvent::NavUp));
        assert_eq!(key(KeyCode::ArrowDown), Some(FlowEvent::NavDown));
        assert_eq!(key(KeyCode::KeyS), Some(FlowEvent::NavDown));
    }

    #[test]
    fn enter_and_space_activate_and_escape_and_backspace_go_back() {
        for code in ACTIVATING_KEYS {
            assert_eq!(menu_event_for_key(code, false), Some(FlowEvent::Activate));
        }
        for code in [KeyCode::Escape, KeyCode::Backspace] {
            assert_eq!(menu_event_for_key(code, false), Some(FlowEvent::Back));
        }
    }

    #[test]
    fn a_repeated_activation_or_back_key_is_ignored() {
        for code in ACTIVATING_KEYS
            .into_iter()
            .chain([KeyCode::Escape, KeyCode::Backspace])
        {
            assert_eq!(menu_event_for_key(code, true), None, "{code:?}");
        }
    }

    #[test]
    fn a_repeated_navigation_key_keeps_moving_the_selection() {
        for (code, event) in [
            (KeyCode::ArrowUp, FlowEvent::NavUp),
            (KeyCode::KeyW, FlowEvent::NavUp),
            (KeyCode::ArrowDown, FlowEvent::NavDown),
            (KeyCode::KeyS, FlowEvent::NavDown),
        ] {
            assert_eq!(menu_event_for_key(code, true), Some(event), "{code:?}");
        }
    }

    #[test]
    fn other_keys_do_nothing_in_the_menu() {
        for code in [
            KeyCode::KeyM,
            KeyCode::KeyE,
            KeyCode::ShiftLeft,
            KeyCode::F1,
        ] {
            assert_eq!(menu_event_for_key(code, false), None);
        }
    }
}
