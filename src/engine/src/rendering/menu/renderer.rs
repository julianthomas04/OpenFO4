//! Draws the menu screens (black background, list rows with a selection bar) and the loading
//! overlay (a black layer with the animated loading logo) with the HUD's quad pipeline, glyphon and
//! the logo's textured quad.

use std::sync::Arc;

use super::layout::MenuLayout;
use super::loading_icon::{IconScene, LoadingIcon, LoadingIconError};
use super::video_layer::VideoLayer;
use crate::assets::pipboy_art::ArtLibrary;
use crate::game_flow::MenuView;
use crate::rendering::game_settings::SafeZone;
use crate::rendering::hud::field_text::{FieldText, TextItem};
use crate::rendering::hud::{
    pipeline::HudPipeline,
    quad::{HudVertex, PxRect, QuadBuilder},
};
use crate::rendering::post::tonemap::ui_gamma_format;
use crate::rendering::ui_colour::UiTint;
use crate::rendering::ui_fonts::{UiFontError, UiFonts};
use crate::ui_layout::{UiLayout, UiLayoutError};
use crate::video::DecodedFrame;

/// The menu draws in the colours the layout gives, so the HUD's tint is the identity.
const NEUTRAL_TINT: UiTint = UiTint {
    color: [1.0; 3],
    opacity: 1.0,
};
const INITIAL_VBUF_VERTS: u64 = 256;

#[derive(Debug, thiserror::Error)]
pub enum MenuError {
    #[error(transparent)]
    Layout(#[from] UiLayoutError),
    #[error(transparent)]
    Font(#[from] UiFontError),
    #[error(transparent)]
    LoadingIcon(#[from] LoadingIconError),
}

/// What the menu renderer draws this frame.
#[derive(Debug, Clone, Copy)]
pub enum MenuScene<'a> {
    /// A black screen with a list of rows.
    List(MenuView<'a>),
    /// A black layer of the given opacity over what is already drawn, with the loading logo
    /// when `loading_seconds`, the time since loading began, is given. The logo fades with the
    /// layer.
    Overlay {
        alpha: f32,
        loading_seconds: Option<f64>,
    },
}

pub struct MenuRenderer {
    gamma_format: wgpu::TextureFormat,
    layout: MenuLayout,
    ui_layout: Arc<UiLayout>,
    /// The HUD colour the game tints its interface movies with: every menu element is
    /// multiplied by it.
    tint: UiTint,
    pipeline: HudPipeline,
    tint_bind_group: wgpu::BindGroup,
    vbuf: wgpu::Buffer,
    vbuf_verts: u64,
    verts: Vec<HudVertex>,
    text: FieldText,
    video: VideoLayer,
    loading_icon: LoadingIcon,
    icon_state: IconState,
    safe_zone: SafeZone,
}

/// What the renderer remembers between frames about the loading logo.
#[derive(Debug, Default)]
struct IconState {
    /// The framebuffer size the logo could not be drawn at; it stays hidden, unlogged, until the
    /// size changes or a new load begins.
    failed_at: Option<(u32, u32)>,
    /// The loading time of the previous frame, to tell a new load from a continuing one.
    last_seconds: f64,
}

impl MenuRenderer {
    /// A renderer for a swapchain of `surface_format`. It blends in gamma space, as Flash does,
    /// so it draws through [`Self::target_view`] only. Fails when the layout lacks the menu or
    /// the loading screen, `fonts` lacks the family the layout names for the list entries, or
    /// the frames of the loading logo in `art` cannot be read.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        ui_layout: Arc<UiLayout>,
        fonts: &UiFonts,
        art: &ArtLibrary,
        tint: UiTint,
        safe_zone: SafeZone,
    ) -> Result<Self, MenuError> {
        let format = ui_gamma_format(surface_format);
        let layout = MenuLayout::resolve(&ui_layout)?;
        fonts.require_family(&layout.list.entry.font_family)?;
        let pipeline = HudPipeline::new(device, format);
        let tint_bind_group = pipeline.create_tint_bind_group(device, &NEUTRAL_TINT);
        let loading_icon = LoadingIcon::new(device, format, art)?;
        Ok(Self {
            gamma_format: format,
            layout,
            ui_layout,
            tint,
            pipeline,
            tint_bind_group,
            vbuf: Self::create_vbuf(device, INITIAL_VBUF_VERTS),
            vbuf_verts: INITIAL_VBUF_VERTS,
            verts: Vec::new(),
            text: FieldText::new(device, queue, format, fonts, glyphon::ColorMode::Web),
            video: VideoLayer::new(device, format),
            loading_icon,
            icon_state: IconState::default(),
            safe_zone,
        })
    }

    pub fn layout(&self) -> &MenuLayout {
        &self.layout
    }

    /// The view of the swapchain `texture` this renderer draws through. The surface must be
    /// configured with `ui_gamma_format` among its view formats.
    pub fn target_view(&self, texture: &wgpu::Texture) -> wgpu::TextureView {
        texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.gamma_format),
            ..Default::default()
        })
    }

    /// Shows `frame` behind the main menu list from the next render on.
    pub fn upload_video(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &DecodedFrame,
    ) {
        self.video.upload(
            device,
            queue,
            &frame.planes(),
            frame.conversion(),
            frame.chroma_siting(),
        );
    }

    /// Frees the video's textures; the list draws on black again.
    pub fn clear_video(&mut self) {
        self.video.clear();
    }

    fn create_vbuf(device: &wgpu::Device, verts: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("menu_vbuf"),
            size: verts * std::mem::size_of::<HudVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// The text a list row shows for `label`, by the game's text-setting rule.
    fn shown(&self, label: &str) -> String {
        self.layout
            .set_text
            .apply(&self.ui_layout, label, false, false)
    }

    /// Encodes the pass for `scene` over `view`, a surface view of `fb` pixels. A list scene
    /// clears the view to black and draws the video, fitted with black bars, behind its rows; an
    /// overlay scene draws over its contents.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        fb: (u32, u32),
        scene: MenuScene<'_>,
    ) {
        let load = match scene {
            MenuScene::List(_) => wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            MenuScene::Overlay { .. } => wgpu::LoadOp::Load,
        };
        let mut verts = std::mem::take(&mut self.verts);
        let mut quads = QuadBuilder::new(&mut verts, fb.0 as f32, fb.1 as f32, 0.0, [0.0; 4]);
        let mut items = Vec::new();
        match scene {
            MenuScene::List(list) => self.draw_list(fb, list, &mut quads, &mut items),
            MenuScene::Overlay { alpha, .. } => Self::draw_overlay(fb, alpha, &mut quads),
        }
        self.verts = verts;
        let video_visible = matches!(scene, MenuScene::List(_)) && self.video.prepare(queue, fb);
        let icon_visible = match scene {
            MenuScene::Overlay {
                alpha,
                loading_seconds: Some(seconds),
            } => self.prepare_loading_icon(device, queue, fb, alpha, seconds),
            MenuScene::List(_) | MenuScene::Overlay { .. } => false,
        };

        self.text.prepare(device, queue, fb, &items);
        self.upload_vertices(device, queue);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Menu Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        if video_visible {
            self.video.draw(&mut pass);
        }
        if !self.verts.is_empty() {
            pass.set_pipeline(&self.pipeline.pipeline);
            pass.set_bind_group(0, &self.tint_bind_group, &[]);
            pass.set_vertex_buffer(0, self.vbuf.slice(..));
            pass.draw(0..self.verts.len() as u32, 0..1);
        }
        if icon_visible {
            self.loading_icon.draw(&mut pass);
        }
        self.text.render_into(&mut pass);
    }

    fn upload_vertices(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.verts.is_empty() {
            return;
        }
        let needed = self.verts.len() as u64;
        if needed > self.vbuf_verts {
            self.vbuf_verts = needed.next_power_of_two();
            self.vbuf = Self::create_vbuf(device, self.vbuf_verts);
        }
        queue.write_buffer(&self.vbuf, 0, bytemuck::cast_slice(&self.verts));
    }

    fn draw_list(
        &mut self,
        fb: (u32, u32),
        list: MenuView<'_>,
        quads: &mut QuadBuilder<'_>,
        items: &mut Vec<TextItem>,
    ) {
        let style = self.layout.list.clone();
        let panel = self.layout.panel_background.clone();
        let [panel_r, panel_g, panel_b] = self.tint.apply_to(panel.color).map(unit_channel);
        quads.rect(
            self.layout.panel_rect(fb.0, fb.1),
            [panel_r, panel_g, panel_b, panel.alpha],
        );
        let [fill_r, fill_g, fill_b] = self.tint.apply_to(style.selection.fill).map(unit_channel);
        let rows: Vec<_> = self
            .layout
            .row_rects(fb.0, fb.1, list.first_visible)
            .collect();
        for row in rows {
            let Some(&label) = list.labels.get(row.index) else {
                break;
            };
            let selected = row.index == list.selected;
            if selected {
                quads.rect(row.rect, [fill_r, fill_g, fill_b, style.selection.alpha]);
            }
            let colour = if selected {
                style.colors.text_selected
            } else {
                style.colors.text
            };
            let [r, g, b] = self.tint.apply_to(colour);
            items.push(TextItem {
                text: self.shown(label),
                family: style.entry.font_family.clone(),
                geometry: style.entry.text,
                map: row.map,
                shrink: true,
                colour: [r, g, b, 255],
                shadow: None,
            });
        }
    }

    /// Draws the black layer of opacity `alpha`.
    fn draw_overlay(fb: (u32, u32), alpha: f32, quads: &mut QuadBuilder<'_>) {
        let alpha = alpha.clamp(0.0, 1.0);
        if alpha <= 0.0 {
            return;
        }
        quads.rect(
            PxRect {
                x: 0.0,
                y: 0.0,
                w: fb.0 as f32,
                h: fb.1 as f32,
            },
            [0.0, 0.0, 0.0, alpha],
        );
    }

    /// Readies the loading logo, `seconds` into its loop and faded by the overlay's `alpha`, in
    /// the HUD colour; false when there is nothing to draw.
    fn prepare_loading_icon(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        fb: (u32, u32),
        alpha: f32,
        seconds: f64,
    ) -> bool {
        if alpha <= 0.0 {
            return false;
        }
        if seconds < self.icon_state.last_seconds {
            self.icon_state.failed_at = None;
        }
        self.icon_state.last_seconds = seconds;
        if self.icon_state.failed_at == Some(fb) {
            return false;
        }
        let fit = self.layout.fit(fb.0, fb.1);
        let scene = IconScene {
            target: fb,
            rect: self.layout.loading_icon.screen_rect(
                self.loading_icon.bounds(),
                &fit,
                self.safe_zone,
            ),
            seconds,
            tint: self.tint.text_rgba(),
            fade: alpha,
        };
        match self.loading_icon.prepare(device, queue, &scene) {
            Ok(visible) => {
                self.icon_state.failed_at = None;
                visible
            }
            Err(error) => {
                tracing::error!("the loading logo cannot be drawn: {error}");
                self.icon_state.failed_at = Some(fb);
                false
            }
        }
    }

    /// Whether the logo is being kept hidden after a failure to draw it.
    #[cfg(test)]
    pub fn loading_icon_failed(&self) -> bool {
        self.icon_state.failed_at.is_some()
    }
}

fn unit_channel(channel: u8) -> f32 {
    f32::from(channel) / 255.0
}
