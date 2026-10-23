//! Frames that are not the 3D world: the menu screens, queued toasts and the frame limiter.

use std::time::{Duration, Instant};

use super::State;
use super::menu::MenuScene;

impl State {
    /// Draws a menu-phase frame: the black screen with the menu list, plus toasts.
    pub(super) fn render_menu_frame(
        &mut self,
        output: wgpu::SurfaceTexture,
        view: &wgpu::TextureView,
        frame_start: Instant,
    ) {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Menu Encoder"),
            });
        let scene = self.flow.menu_view().map_or(
            MenuScene::Overlay {
                alpha: 1.0,
                loading_seconds: None,
            },
            MenuScene::List,
        );
        let menu_view = self.menu.target_view(&output.texture);
        self.menu.render(
            &self.device,
            &self.queue,
            &mut encoder,
            &menu_view,
            (self.config.width, self.config.height),
            scene,
        );
        self.render_version_overlay(&mut encoder, view);
        self.show_queued_toasts();
        self.debug_ui.render(
            &self.window,
            &self.device,
            &self.queue,
            &self.config,
            &mut encoder,
            view,
            &self.gpu_profiler,
            |_| {},
        );
        let frame_capture = self.begin_frame_capture(&mut encoder, &output.texture);
        self.queue.submit(std::iter::once(encoder.finish()));
        self.finish_frame_capture(frame_capture);
        self.queue.present(output);
        self.limit_frame_rate(frame_start);
    }

    pub(super) fn render_version_overlay(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
    ) {
        self.version_overlay.render(
            &self.device,
            &self.queue,
            encoder,
            view,
            (self.config.width, self.config.height),
        );
    }

    pub(super) fn show_queued_toasts(&mut self) {
        if !self.flow.toasts_visible() || self.capture_active() {
            return;
        }
        for msg in &self.toast_queue {
            self.debug_ui.show_toast(&msg.0, &msg.1);
        }
        self.toast_queue.clear();
    }

    pub(super) fn limit_frame_rate(&self, frame_start: Instant) {
        if self.fps_limit > 0 {
            let elapsed = frame_start.elapsed().as_secs_f32();
            let remaining = self.target_delta - elapsed;
            if remaining > 0.0 {
                let _span = tracing::info_span!("frame_limiter").entered();
                spin_sleep::sleep(Duration::from_secs_f32(remaining));
            }
        }
    }
}
