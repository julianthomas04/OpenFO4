//! Draws the menu screens (black background, list rows with a selection bar) and the loading
//! overlay (a black layer with the animated loading logo) with the HUD's quad pipeline, glyphon and
//! the logo's textured quad.

use std::sync::Arc;

use super::image_quad::{BackgroundQuad, ImageQuadPipeline};
use super::layout::MenuLayout;
use super::loading_icon::{IconScene, LoadingIcon, LoadingIconError};
use super::loading_panel::{ColouredRect, PanelContent};
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
        /// The text panel of a large loading screen.
        panel: Option<PanelContent<'a>>,
    },
}

/// What an overlay draws first: its black layer and, on a large loading screen, the image behind
/// the loading art.
#[derive(Debug, Clone, Copy)]
pub struct Backdrop<'a> {
    pub alpha: f32,
    /// Stretched over the whole target above the black layer.
    pub image: Option<&'a wgpu::TextureView>,
}

/// What an overlay draws above its backdrop and the loading art.
#[derive(Debug, Clone, Copy)]
pub struct OverlayForeground<'a> {
    pub alpha: f32,
    pub loading_seconds: Option<f64>,
    pub panel: Option<PanelContent<'a>>,
}

/// A vertex buffer of rectangles and the vertices it holds this frame.
struct QuadLayer {
    vbuf: wgpu::Buffer,
    capacity: u64,
    verts: Vec<HudVertex>,
}

impl QuadLayer {
    fn new(device: &wgpu::Device) -> Self {
        Self {
            vbuf: Self::create_buffer(device, INITIAL_VBUF_VERTS),
            capacity: INITIAL_VBUF_VERTS,
            verts: Vec::new(),
        }
    }

    fn create_buffer(device: &wgpu::Device, verts: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("menu_vbuf"),
            size: verts * std::mem::size_of::<HudVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.verts.is_empty() {
            return;
        }
        let needed = self.verts.len() as u64;
        if needed > self.capacity {
            self.capacity = needed.next_power_of_two();
            self.vbuf = Self::create_buffer(device, self.capacity);
        }
        queue.write_buffer(&self.vbuf, 0, bytemuck::cast_slice(&self.verts));
    }

    fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipeline: &HudPipeline,
        tint_bind_group: &wgpu::BindGroup,
    ) {
        if self.verts.is_empty() {
            return;
        }
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, tint_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vbuf.slice(..));
        pass.draw(0..self.verts.len() as u32, 0..1);
    }
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
    /// The list's rectangles and the overlay's black layer.
    quads: QuadLayer,
    /// The overlay's panel rectangles. Its own buffer, because the queue's writes of both layers
    /// land before the frame's passes run.
    foreground_quads: QuadLayer,
    text: FieldText,
    video: VideoLayer,
    image_pipeline: ImageQuadPipeline,
    background: BackgroundQuad,
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
        let image_pipeline = ImageQuadPipeline::new(device, format);
        let background = BackgroundQuad::new(device);
        Ok(Self {
            gamma_format: format,
            layout,
            ui_layout,
            tint,
            pipeline,
            tint_bind_group,
            quads: QuadLayer::new(device),
            foreground_quads: QuadLayer::new(device),
            text: FieldText::new(device, queue, format, fonts, glyphon::ColorMode::Web),
            video: VideoLayer::new(device, format),
            image_pipeline,
            background,
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

    /// The text a list row shows for `label`, by the game's text-setting rule.
    fn shown(&self, label: &str) -> String {
        self.layout
            .set_text
            .apply(&self.ui_layout, label, false, false)
    }

    /// Encodes the pass for `scene` over `view`, a surface view of `fb` pixels. A list scene
    /// clears the view to black and draws the video, fitted with black bars, behind its rows; an
    /// overlay scene draws its backdrop and then its foreground over the view's contents.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        fb: (u32, u32),
        scene: MenuScene<'_>,
    ) {
        match scene {
            MenuScene::List(list) => self.render_list(device, queue, encoder, view, fb, list),
            MenuScene::Overlay {
                alpha,
                loading_seconds,
                panel,
            } => {
                let backdrop = Backdrop { alpha, image: None };
                self.render_backdrop(device, queue, encoder, view, fb, backdrop);
                let foreground = OverlayForeground {
                    alpha,
                    loading_seconds,
                    panel,
                };
                self.render_foreground(device, queue, encoder, view, fb, foreground);
            }
        }
    }

    fn render_list(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        fb: (u32, u32),
        list: MenuView<'_>,
    ) {
        let mut verts = std::mem::take(&mut self.quads.verts);
        let mut quads = QuadBuilder::new(&mut verts, fb.0 as f32, fb.1 as f32, 0.0, [0.0; 4]);
        let mut items = Vec::new();
        self.draw_list(fb, list, &mut quads, &mut items);
        self.quads.verts = verts;
        let video_visible = self.video.prepare(queue, fb);

        self.text.prepare(device, queue, fb, &items);
        self.quads.upload(device, queue);

        let mut pass = Self::begin_pass(
            encoder,
            "Menu Pass",
            view,
            wgpu::LoadOp::Clear(wgpu::Color::BLACK),
        );
        if video_visible {
            self.video.draw(&mut pass);
        }
        self.quads
            .draw(&mut pass, &self.pipeline, &self.tint_bind_group);
        self.text.render_into(&mut pass);
    }

    /// Encodes the pass of an overlay's black layer and, over it, its image, over `view`'s
    /// contents. An overlay is drawn in three steps so that the loading art can be drawn between
    /// the backdrop and the foreground: this, the art's own pass, then
    /// [`Self::render_foreground`].
    pub fn render_backdrop(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        fb: (u32, u32),
        backdrop: Backdrop<'_>,
    ) {
        let mut verts = std::mem::take(&mut self.quads.verts);
        let mut quads = QuadBuilder::new(&mut verts, fb.0 as f32, fb.1 as f32, 0.0, [0.0; 4]);
        Self::draw_overlay(fb, backdrop.alpha, &mut quads);
        self.quads.verts = verts;
        self.quads.upload(device, queue);
        match backdrop.image {
            Some(image) => self.background.prepare(
                &self.image_pipeline,
                device,
                queue,
                image,
                fb,
                backdrop.alpha,
            ),
            None => self.background.clear(),
        }

        let mut pass = Self::begin_pass(encoder, "Menu Backdrop Pass", view, wgpu::LoadOp::Load);
        self.quads
            .draw(&mut pass, &self.pipeline, &self.tint_bind_group);
        self.background.draw(&self.image_pipeline, &mut pass);
    }

    /// Encodes the pass of an overlay's loading logo and panel over `view`'s contents.
    pub fn render_foreground(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        fb: (u32, u32),
        foreground: OverlayForeground<'_>,
    ) {
        let OverlayForeground {
            alpha,
            loading_seconds,
            panel,
        } = foreground;
        let icon_visible = match loading_seconds {
            Some(seconds) => self.prepare_loading_icon(device, queue, fb, alpha, seconds),
            None => false,
        };
        let (items, rects) = match panel.filter(|_| alpha > 0.0) {
            Some(content) => self.panel_layers(fb, &content),
            None => (Vec::new(), Vec::new()),
        };
        self.text.prepare(device, queue, fb, &items);
        let mut verts = std::mem::take(&mut self.foreground_quads.verts);
        let mut quads = QuadBuilder::new(&mut verts, fb.0 as f32, fb.1 as f32, 0.0, [0.0; 4]);
        for (rect, colour) in rects {
            quads.rect(rect, colour);
        }
        self.foreground_quads.verts = verts;
        self.foreground_quads.upload(device, queue);

        let mut pass = Self::begin_pass(encoder, "Menu Foreground Pass", view, wgpu::LoadOp::Load);
        if icon_visible {
            self.loading_icon.draw(&mut pass);
        }
        self.foreground_quads
            .draw(&mut pass, &self.pipeline, &self.tint_bind_group);
        self.text.render_into(&mut pass);
    }

    /// The text and the rectangles of the loading panel on a target of `fb` pixels. The tip's box
    /// is drawn around the lines the tip really wraps into.
    fn panel_layers(
        &mut self,
        fb: (u32, u32),
        content: &PanelContent<'_>,
    ) -> (Vec<TextItem>, Vec<ColouredRect>) {
        let fit = self.layout.fit(fb.0, fb.1);
        let panel = &self.layout.loading_panel;
        let text = panel.text(&fit, &self.tint, content);
        let tip_extent = text.tip.as_ref().map(|tip| self.text.measure(tip));
        let rects = panel.rects(&fit, &self.tint, tip_extent, content.experience);
        (text.into_items(), rects)
    }

    fn begin_pass<'encoder>(
        encoder: &'encoder mut wgpu::CommandEncoder,
        label: &'static str,
        view: &wgpu::TextureView,
        load: wgpu::LoadOp<wgpu::Color>,
    ) -> wgpu::RenderPass<'encoder> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
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
        })
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
                wrap_width: None,
                max_height: None,
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
