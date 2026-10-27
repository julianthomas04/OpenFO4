use super::boot_handoff::{StartupToasts, configure_pending_surface, finish_boot};
use super::key_input::resolve_key_code;
use crate::assets::AssetServer;
use crate::game_flow::StartMode;
use crate::rendering::State;
use crate::rendering::boot::{BootContext, StartupInputs};
use crate::rendering::fatal_runtime_error::FatalRuntimeError;
use crate::rendering::pipboy_capture::CaptureConfig;
use crate::rendering::shot::ShotConfig;
use crate::startup::{BootState, CancelToken, StartupError, run_startup, spawn_worker};
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, StartCause, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow},
    window::{Fullscreen, Icon, Window, WindowId},
};

/// How many pixels of a touchpad scroll count as one wheel line.
const PIXELS_PER_LINE: f32 = 40.0;

/// Time between the black frames shown while the engine starts up.
const BOOT_FRAME_INTERVAL: Duration = Duration::from_millis(33);

/// Whether the debug UI is offered `event`. Pointer and keyboard input is the game's while the
/// developer window is hidden (menus, loading); everything else the UI needs to stay in step with
/// the window.
fn debug_ui_takes(event: &WindowEvent, developer_window_visible: bool) -> bool {
    let is_pointer_or_keyboard = matches!(
        event,
        WindowEvent::CursorMoved { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::KeyboardInput { .. }
    );
    developer_window_visible || !is_pointer_or_keyboard
}

type AppPhase = BootState<BootContext, State>;

pub struct App {
    phase: Option<AppPhase>,
    asset_server: AssetServer,
    start_mode: StartMode,
    capture_window_size: Option<PhysicalSize<u32>>,
    perf_log: bool,
    startup_toasts: StartupToasts,
    cancel_startup: CancelToken,
    runtime_error: Option<FatalRuntimeError>,
}

impl App {
    pub fn new(startup_toasts: StartupToasts) -> anyhow::Result<Self> {
        Ok(Self {
            phase: None,
            asset_server: AssetServer::from_environment()?,
            start_mode: StartMode::from_env(),
            capture_window_size: capture_window_size()?,
            perf_log: crate::utils::env_flag("OPENFO4_PERF_LOG", false),
            startup_toasts,
            cancel_startup: CancelToken::default(),
            runtime_error: None,
        })
    }

    /// The error that stopped startup, unless the user cancelled it by closing the window.
    pub fn take_startup_error(&mut self) -> Option<StartupError> {
        match self.phase.take() {
            Some(BootState::Failed(error)) if !error.is_cancellation() => Some(error),
            other => {
                self.phase = other;
                None
            }
        }
    }

    /// The error that ended the game after startup, if one did.
    pub fn take_runtime_error(&mut self) -> Option<FatalRuntimeError> {
        self.runtime_error.take()
    }

    fn fail_startup(&mut self, event_loop: &ActiveEventLoop, error: StartupError) {
        self.phase = Some(BootState::Failed(error));
        event_loop.exit();
    }

    fn create_window(
        event_loop: &ActiveEventLoop,
        capture_window_size: Option<PhysicalSize<u32>>,
    ) -> Result<Arc<Window>, StartupError> {
        let attributes = Window::default_attributes().with_title("OpenFO4");
        let attributes = match capture_window_size {
            Some(size) => attributes.with_inner_size(size).with_resizable(false),
            None => attributes.with_fullscreen(Some(Fullscreen::Borderless(None))),
        };
        let window = Arc::new(event_loop.create_window(attributes)?);
        match window_icon() {
            Ok(icon) => window.set_window_icon(Some(icon)),
            Err(error) => tracing::warn!("window icon not set: {error}"),
        }
        Ok(window)
    }

    fn begin_boot(&self, event_loop: &ActiveEventLoop) -> Result<AppPhase, StartupError> {
        let window = Self::create_window(event_loop, self.capture_window_size)?;
        let boot = BootContext::create(window, self.start_mode)?;
        let inputs = StartupInputs {
            asset_server: self.asset_server.clone(),
            perf_log: self.perf_log,
            start_mode: self.start_mode,
            cancel: self.cancel_startup.clone(),
        };
        let parts = boot.parts();
        let handle = spawn_worker("startup", move || run_startup(inputs, parts))?;
        Ok(BootState::booting(boot, handle))
    }

    fn schedule_next_boot_frame(event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + BOOT_FRAME_INTERVAL));
    }

    fn is_booting(&self) -> bool {
        matches!(self.phase, Some(BootState::Booting { .. }))
    }

    fn booting_event(&mut self, event_loop: &ActiveEventLoop, event: &WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.cancel_startup.cancel();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(BootState::Booting { boot, .. }) = self.phase.as_mut() {
                    boot.resize(*size);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(BootState::Booting { boot, .. }) = self.phase.as_ref() {
                    boot.present_black();
                }
                self.advance_boot(event_loop);
            }
            _ => {}
        }
    }

    /// Checks whether the worker is done; on success the new state takes over, on failure the
    /// event loop stops.
    fn advance_boot(&mut self, event_loop: &ActiveEventLoop) {
        let Some(phase) = self.phase.take() else {
            return;
        };
        let was_booting = matches!(phase, BootState::Booting { .. });
        let mut phase = phase.advance();
        match &mut phase {
            BootState::Running(state) if was_booting => {
                let size = state.window.inner_size();
                finish_boot(state, size, &mut self.startup_toasts);
                event_loop.set_control_flow(ControlFlow::Wait);
                state.window.request_redraw();
            }
            BootState::Failed(_) => event_loop.exit(),
            BootState::Booting { .. } | BootState::Running(_) => {}
        }
        self.phase = Some(phase);
    }
}

/// The fixed window size of a screenshot mode; `None` keeps the borderless fullscreen window.
fn capture_window_size() -> anyhow::Result<Option<PhysicalSize<u32>>> {
    let pipboy = CaptureConfig::from_env()?.map(|capture| capture.window_size);
    let shot = ShotConfig::from_env()?.map(|shot| shot.window_size);
    Ok(pipboy.or(shot))
}

fn window_icon() -> anyhow::Result<Icon> {
    let icon_image = image::load_from_memory(include_bytes!("placeholder.jpg"))?.into_rgba8();
    let (width, height) = icon_image.dimensions();
    Ok(Icon::from_rgba(icon_image.into_raw(), width, height)?)
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.phase.is_some() {
            return;
        }
        match self.begin_boot(event_loop) {
            Ok(phase) => {
                if let BootState::Booting { boot, .. } = &phase {
                    boot.window().request_redraw();
                }
                self.phase = Some(phase);
                Self::schedule_next_boot_frame(event_loop);
            }
            Err(error) => self.fail_startup(event_loop, error),
        }
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        if !matches!(cause, StartCause::ResumeTimeReached { .. }) {
            return;
        }
        self.advance_boot(event_loop);
        if let Some(BootState::Booting { boot, .. }) = self.phase.as_ref() {
            boot.window().request_redraw();
            Self::schedule_next_boot_frame(event_loop);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if self.is_booting() {
            self.booting_event(event_loop, &event);
            return;
        }
        let Some(BootState::Running(state)) = self.phase.as_mut() else {
            return;
        };
        // Let the debug ui consume the event first if applicable, else handle it ourselves.
        if debug_ui_takes(&event, state.developer_window_visible()) {
            let response = state.debug_ui.handle_event(&state.window, &event);
            if response.consumed {
                return;
            }
        }

        // The Pip-Boy, when open, takes every keyboard and mouse event the developer menu left.
        if state.pipboy_open() {
            if state.handle_pipboy_event(&event) {
                return;
            }
        } else {
            state.observe_pipboy_event(&event);
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => state.resize(size.width, size.height),
            WindowEvent::RedrawRequested => {
                configure_pending_surface(state, state.window.inner_size());
                let update_start = std::time::Instant::now();
                if let Err(fatal) = state.update(&mut self.asset_server) {
                    tracing::error!("{fatal}");
                    self.runtime_error = Some(fatal);
                    event_loop.exit();
                    return;
                }
                let render_start = std::time::Instant::now();
                if let Err(e) = state.render(&mut self.asset_server) {
                    tracing::error!("render error: {e}");
                }
                state.record_perf_frame(render_start - update_start, render_start.elapsed());
                #[cfg(feature = "tracy")]
                tracing_tracy::client::frame_mark();
            }
            WindowEvent::CursorMoved { position, .. } => {
                state.handle_pointer_moved(position.x, position.y);
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => state.handle_pointer_pressed(),
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => state.handle_pointer_released(),
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, lines) => lines,
                    MouseScrollDelta::PixelDelta(pixels) => pixels.y as f32 / PIXELS_PER_LINE,
                };
                state.handle_wheel(lines);
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key,
                        ref logical_key,
                        state: key_state,
                        repeat,
                        ..
                    },
                ..
            } => {
                if let Some(code) = resolve_key_code(physical_key, logical_key) {
                    state.handle_key(code, key_state.is_pressed(), repeat);
                }
            }
            _ => {}
        }

        if state.quit_requested() {
            event_loop.exit();
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        let Some(BootState::Running(state)) = self.phase.as_mut() else {
            return;
        };
        if let winit::event::DeviceEvent::MouseMotion { delta: (dx, dy) } = event
            && !state.pipboy_open()
        {
            state.handle_mouse_motion(dx, dy);
        }
    }
}

#[cfg(test)]
mod tests {
    use winit::dpi::{PhysicalPosition, PhysicalSize};
    use winit::event::{DeviceId, Modifiers, TouchPhase};

    use super::*;

    fn pointer_and_wheel_events() -> Vec<WindowEvent> {
        let device_id = DeviceId::dummy();
        vec![
            WindowEvent::CursorMoved {
                device_id,
                position: PhysicalPosition::new(1.0, 2.0),
            },
            WindowEvent::MouseInput {
                device_id,
                state: ElementState::Pressed,
                button: MouseButton::Left,
            },
            WindowEvent::MouseWheel {
                device_id,
                delta: MouseScrollDelta::LineDelta(0.0, 1.0),
                phase: TouchPhase::Moved,
            },
        ]
    }

    fn window_state_events() -> Vec<WindowEvent> {
        vec![
            WindowEvent::Resized(PhysicalSize::new(640, 480)),
            WindowEvent::Focused(true),
            WindowEvent::ModifiersChanged(Modifiers::default()),
            WindowEvent::RedrawRequested,
        ]
    }

    #[test]
    fn the_game_keeps_pointer_input_while_the_debug_window_is_hidden() {
        for event in pointer_and_wheel_events() {
            assert!(!debug_ui_takes(&event, false), "{event:?}");
        }
    }

    #[test]
    fn the_debug_ui_gets_every_event_while_its_window_is_shown() {
        for event in pointer_and_wheel_events()
            .into_iter()
            .chain(window_state_events())
        {
            assert!(debug_ui_takes(&event, true), "{event:?}");
        }
    }

    #[test]
    fn the_debug_ui_still_follows_the_window_while_hidden() {
        for event in window_state_events() {
            assert!(debug_ui_takes(&event, false), "{event:?}");
        }
    }
}
