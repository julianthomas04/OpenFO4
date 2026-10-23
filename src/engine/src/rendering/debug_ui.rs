use egui_notify::Toasts;
use wgpu::{CommandEncoder, Device, Queue, SurfaceConfiguration, TextureView};
use winit::window::Window;

use crate::rendering::egui_pass::{EguiPass, EguiTarget};
use crate::rendering::gpu_profiler::{GpuPass, GpuProfiler};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastType {
    Info,
    Warning,
    Error,
    Success,
}

pub struct DebugUi {
    ctx: egui::Context,
    toasts: Toasts,
    winit: egui_winit::State,
    pass: EguiPass,
}

impl DebugUi {
    pub fn new(
        device: &wgpu::Device,
        window: &winit::window::Window,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        let ctx = egui::Context::default();
        let toasts = Toasts::default();
        let winit = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            None,
            None,
            None,
        );
        let pass = EguiPass::new(device, surface_format);
        Self {
            ctx,
            toasts,
            winit,
            pass,
        }
    }

    pub fn render<F>(
        &mut self,
        window: &Window,
        device: &Device,
        queue: &Queue,
        config: &SurfaceConfiguration,
        encoder: &mut CommandEncoder,
        view: &TextureView,
        profiler: &GpuProfiler,
        ui_builder: F,
    ) where
        F: FnMut(&mut egui::Ui),
    {
        let raw_input = self.winit.take_egui_input(window);

        // Toasts paint inside the pass: painting after run_ui ends the pass
        // would defer their shapes to the next frame (one-frame lag).
        let toasts = &mut self.toasts;
        let mut ui_builder = ui_builder;
        let mut full_output = self.ctx.run_ui(raw_input, |ui| {
            ui_builder(ui);
            toasts.show(ui);
        });

        self.winit
            .handle_platform_output(window, std::mem::take(&mut full_output.platform_output));

        let target = EguiTarget {
            label: "Egui Render Pass",
            view,
            size_in_pixels: [config.width, config.height],
            pixels_per_point: self.ctx.pixels_per_point(),
            profiled_as: GpuPass::Egui,
        };
        self.pass.draw(
            device,
            queue,
            encoder,
            profiler,
            &target,
            &self.ctx,
            full_output,
        );
    }

    pub fn register_texture(&mut self, device: &Device, view: &TextureView) -> egui::TextureId {
        self.pass.register_texture(device, view)
    }

    /// Offers `event` to the developer menu. Tab never reaches egui while no widget wants the
    /// keyboard: egui would focus the first widget on it, and that focus then swallows every
    /// later key (movement, the Pip-Boy close key) until Esc drops it.
    pub fn handle_event(
        &mut self,
        window: &Window,
        event: &winit::event::WindowEvent,
    ) -> egui_winit::EventResponse {
        if !egui_sees_key(
            pressed_key_code(event),
            self.ctx.egui_wants_keyboard_input(),
        ) {
            return egui_winit::EventResponse {
                consumed: false,
                repaint: false,
            };
        }
        self.winit.on_window_event(window, event)
    }

    /// Drops the keyboard focus of any developer menu widget, so the game's keys reach the game.
    pub fn release_keyboard_focus(&self) {
        surrender_focus(&self.ctx);
    }

    pub fn show_toast(&mut self, message: &str, toast_type: &ToastType) {
        match toast_type {
            ToastType::Info => self.toasts.info(message),
            ToastType::Warning => self.toasts.warning(message),
            ToastType::Error => self.toasts.error(message),
            ToastType::Success => self.toasts.success(message),
        };
    }
}

fn pressed_key_code(event: &winit::event::WindowEvent) -> Option<winit::keyboard::KeyCode> {
    match event {
        winit::event::WindowEvent::KeyboardInput {
            event:
                winit::event::KeyEvent {
                    physical_key: winit::keyboard::PhysicalKey::Code(code),
                    ..
                },
            ..
        } => Some(*code),
        _ => None,
    }
}

/// Whether egui may see a key event: everything but Tab, which egui sees only once a widget
/// already wants the keyboard (focus cycling inside the menu).
fn egui_sees_key(code: Option<winit::keyboard::KeyCode>, egui_wants_keyboard: bool) -> bool {
    egui_wants_keyboard || code != Some(winit::keyboard::KeyCode::Tab)
}

fn surrender_focus(ctx: &egui::Context) {
    ctx.memory_mut(|memory| {
        if let Some(focused) = memory.focused() {
            memory.surrender_focus(focused);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_frame(ctx: &egui::Context, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            events,
            ..egui::RawInput::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            let _ = ui.button("widget");
        });
        output.textures_delta.clear();
    }

    fn tab_press() -> egui::Event {
        egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: Some(egui::Key::Tab),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn escape_press() -> egui::Event {
        egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: Some(egui::Key::Escape),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn a_tab_fed_to_egui_focuses_a_widget_that_then_wants_every_key() {
        let ctx = egui::Context::default();
        run_frame(&ctx, vec![]);
        assert!(!ctx.egui_wants_keyboard_input());
        run_frame(&ctx, vec![tab_press()]);
        run_frame(&ctx, vec![]);
        assert!(ctx.egui_wants_keyboard_input());
        run_frame(&ctx, vec![escape_press()]);
        assert!(!ctx.egui_wants_keyboard_input());
    }

    #[test]
    fn releasing_the_keyboard_focus_returns_the_keys_to_the_game() {
        let ctx = egui::Context::default();
        run_frame(&ctx, vec![]);
        run_frame(&ctx, vec![tab_press()]);
        run_frame(&ctx, vec![]);
        assert!(ctx.egui_wants_keyboard_input());
        surrender_focus(&ctx);
        assert!(!ctx.egui_wants_keyboard_input());
    }

    #[test]
    fn tab_stays_with_the_game_until_a_widget_wants_the_keyboard() {
        use winit::keyboard::KeyCode;
        assert!(!egui_sees_key(Some(KeyCode::Tab), false));
        assert!(egui_sees_key(Some(KeyCode::Tab), true));
        assert!(egui_sees_key(Some(KeyCode::KeyW), false));
        assert!(egui_sees_key(None, false));
    }
}
