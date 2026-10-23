//! The native HUD overlay: a custom screen-space wgpu pass that composites HP/AP
//! meters and the crosshair on top of the 3D scene. It is the custom-wgpu half
//! of the HUD design: a single vertex-colored
//! triangle pipeline whose output is multiplied by a global tint uniform (FO4's
//! white-art * Pip-Boy-green model). The pass is color-only with no depth — it
//! slots in after `HiZ` generation and before the egui debug pass in
//! [`crate::rendering`].
//!
//! Every geometry, font and string value comes from the converted `HUDMenu` movie
//! ([`LAYOUT_KEYS`]); the native engine supplies the meter fill fractions and the tint.

mod clip;
mod crosshair;
pub mod field_text;
pub(crate) mod layout;
mod meters;
pub(crate) mod pipeline;
pub(crate) mod quad;
pub(crate) mod shadow;
pub(crate) mod shape;

use crate::rendering::gpu_profiler::{GpuPass, GpuProfiler, SpanPart};
use crate::rendering::ui_colour::UiTint;
use crate::rendering::ui_fonts::UiFonts;
use crate::sim::{CrosshairState, HudSnapshot};
use crate::ui_layout::{ModuleKeys, UiLayout, UiLayoutError};
use crosshair::{CrosshairSpec, CrosshairWidget};
use field_text::{FieldText, TextItem};
use layout::{Anchor, Layout};
use meters::{AP_KEYS, HP_KEYS, MeterSpec};
use pipeline::HudPipeline;
use quad::{HudVertex, QuadBuilder};

const INITIAL_VBUF_VERTS: u64 = 1024;

/// Every layout key the HUD reads, and that the movie's pieces fit together as the HUD needs.
fn validate(layout: &UiLayout) -> Result<(), UiLayoutError> {
    HudSpec::resolve(layout).map(drop)
}

pub static LAYOUT_KEYS: ModuleKeys = ModuleKeys {
    keys: &[&HP_KEYS, &AP_KEYS, &meters::SET_TEXT, &crosshair::KEYS],
    resolve: validate,
};

/// The HUD's values from the layout, resolved once.
struct HudSpec {
    stage: (f32, f32),
    hp: MeterSpec,
    ap: MeterSpec,
    crosshair: CrosshairSpec,
}

impl HudSpec {
    fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let stage = layout.movie(meters::MOVIE)?.stage().rect;
        let [xmin, ymin, xmax, ymax] = stage;
        if xmin != 0.0 || ymin != 0.0 {
            return Err(UiLayoutError::unresolved(
                meters::MOVIE,
                "stage",
                format!("the stage starts at ({xmin}, {ymin}), not at the origin"),
            ));
        }
        Ok(Self {
            stage: (xmax, ymax),
            hp: MeterSpec::resolve(layout, &HP_KEYS, Anchor::Bl, stage)?,
            ap: MeterSpec::resolve(layout, &AP_KEYS, Anchor::Br, stage)?,
            crosshair: CrosshairSpec::resolve(layout)?,
        })
    }
}

pub struct HudRenderer {
    pipeline: HudPipeline,
    tint_bind_group: wgpu::BindGroup,
    vbuf: wgpu::Buffer,
    vbuf_verts: u64,
    /// Retained, cleared+refilled each frame (capacity preserved).
    verts: Vec<HudVertex>,
    stage: (f32, f32),
    hp: MeterSpec,
    ap: MeterSpec,
    tint: UiTint,
    crosshair: CrosshairWidget,
    text: FieldText,
    /// The labels for the framebuffer size they were built for.
    labels: Vec<TextItem>,
    labels_for: (u32, u32),
    fb: (u32, u32),
    /// Whether the HUD is shown this frame (false until the player has spawned,
    /// so meters and crosshair appear/disappear together — FO4 hides the HUD
    /// until the player is loaded).
    visible: bool,
}

impl HudRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        fonts: &UiFonts,
        layout: &UiLayout,
        tint: UiTint,
    ) -> anyhow::Result<Self> {
        let spec = HudSpec::resolve(layout)?;
        for meter in [&spec.hp, &spec.ap] {
            fonts.require_family(&meter.label.family)?;
        }
        let pipeline = HudPipeline::new(device, format);
        let text = FieldText::new(device, queue, format, fonts, glyphon::ColorMode::Accurate);
        let tint_bind_group = pipeline.create_tint_bind_group(device, &tint);
        let vbuf = Self::create_vbuf(device, INITIAL_VBUF_VERTS);
        Ok(Self {
            pipeline,
            tint_bind_group,
            vbuf,
            vbuf_verts: INITIAL_VBUF_VERTS,
            verts: Vec::with_capacity(INITIAL_VBUF_VERTS as usize),
            stage: spec.stage,
            hp: spec.hp,
            ap: spec.ap,
            tint,
            crosshair: CrosshairWidget::new(spec.crosshair),
            text,
            labels: Vec::new(),
            labels_for: (0, 0),
            fb: (0, 0),
            visible: false,
        })
    }

    pub fn stage(&self) -> (f32, f32) {
        self.stage
    }

    fn create_vbuf(device: &wgpu::Device, verts: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hud_vbuf"),
            size: verts * std::mem::size_of::<HudVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Advance animation and rebuild the retained vertex list from the
    /// snapshot. `activate` overrides the crosshair to the Activate state
    /// (door-aim, selected render-side); `dt` is the wall-clock frame delta.
    pub fn update(
        &mut self,
        snapshot: &HudSnapshot,
        activate: bool,
        dt: f32,
        fb_w: u32,
        fb_h: u32,
    ) {
        // The HUD is shown only once the player has spawned; before that the
        // meters and crosshair stay hidden together (no half-built overlay).
        self.visible = snapshot.visible;
        if !self.visible {
            self.verts.clear();
            return;
        }
        let requested = if activate && snapshot.crosshair != CrosshairState::Hidden {
            CrosshairState::Activate
        } else {
            snapshot.crosshair
        };
        self.crosshair.update(requested, dt);

        let layout = Layout::for_stage(self.stage.0, self.stage.1, fb_w as f32, fb_h as f32);
        let hp_bar = meters::bar_rect(&layout, &self.hp);
        let ap_bar = meters::bar_rect(&layout, &self.ap);
        self.fb = (fb_w, fb_h);
        if self.labels_for != self.fb || self.labels.is_empty() {
            self.labels = vec![
                meters::label_item(&layout, &self.hp, hp_bar, &self.tint),
                meters::label_item(&layout, &self.ap, ap_bar, &self.tint),
            ];
            self.labels_for = self.fb;
        }

        let shadow = layout.s(shadow::QUAD_OFFSET_STAGE_PX);
        let mut qb = QuadBuilder::new(
            &mut self.verts,
            fb_w as f32,
            fb_h as f32,
            shadow,
            shadow::QUAD_COLOUR,
        );
        meters::emit(&mut qb, &layout, &self.hp, hp_bar, snapshot.hp);
        meters::emit(&mut qb, &layout, &self.ap, ap_bar, snapshot.ap);
        self.crosshair.emit(&mut qb, &layout);
        qb.finish();
    }

    /// Encode the HUD pass: upload the vertices and draw them color-only over
    /// the swapchain view (`LoadOp::Load`, no depth). Caller slots this between
    /// `HiZ` generation and the egui pass.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        profiler: &GpuProfiler,
    ) {
        // Hidden until the player spawns: no pass at all (saves the full-screen
        // load/store while the world streams in).
        if !self.visible {
            return;
        }
        // Upload text glyphs/positions (no encoder needed) and the quad
        // vertices before the pass.
        self.text.prepare(device, queue, self.fb, &self.labels);
        if !self.verts.is_empty() {
            let needed = self.verts.len() as u64;
            if needed > self.vbuf_verts {
                self.vbuf_verts = needed.next_power_of_two();
                self.vbuf = Self::create_vbuf(device, self.vbuf_verts);
            }
            queue.write_buffer(&self.vbuf, 0, bytemuck::cast_slice(&self.verts));
        }

        // A SINGLE color-only pass for the whole HUD (quads then text). One
        // pass, not two: each extra full-screen LoadOp::Load/Store pass costs
        // swapchain bandwidth, which is the dominant per-frame HUD cost.
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("HUD Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: profiler.render_writes(GpuPass::Hud, SpanPart::Whole),
            multiview_mask: None,
        });
        if !self.verts.is_empty() {
            pass.set_pipeline(&self.pipeline.pipeline);
            pass.set_bind_group(0, &self.tint_bind_group, &[]);
            pass.set_vertex_buffer(0, self.vbuf.slice(..));
            pass.draw(0..self.verts.len() as u32, 0..1);
        }
        // Text (numbers/labels) composited over the bars, in the same pass.
        self.text.render_into(&mut pass);
    }
}

/// Developer checks against a real install (`OPENFO4_INSTALL_DIR`), run with `--ignored`.
#[cfg(test)]
pub(crate) mod real_install {
    use super::*;

    pub fn installed_layout() -> UiLayout {
        let install = std::env::var(crate::assets::INSTALL_DIR_VAR).expect("OPENFO4_INSTALL_DIR");
        let dir = crate::assets::layout::ui_layout_dir(&format!(
            "{install}/{}",
            crate::assets::layout::ASSETS_DIR
        ));
        UiLayout::open(std::path::Path::new(&dir)).unwrap()
    }

    /// The width of `text` in stage pixels at the version overlay's size, as glyphon shapes it.
    fn shaped_width(fonts: &crate::rendering::ui_fonts::UiFonts, text: &str) -> f32 {
        use crate::rendering::version_overlay::TEXT_SIZE;
        use crate::ui_layout::faces::BOLD_FACE;
        use glyphon::{Attrs, Buffer, Family, Metrics, Shaping};
        let mut font_system = fonts.font_system();
        let mut buffer = Buffer::new(&mut font_system, Metrics::new(TEXT_SIZE, TEXT_SIZE));
        buffer.set_size(None, None);
        buffer.set_text(
            text,
            &Attrs::new().family(Family::Name(BOLD_FACE)),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut font_system, false);
        buffer.layout_runs().next().map_or(0.0, |run| run.line_w)
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the installer's layout documents"]
    fn the_version_text_stays_clear_of_the_hp_meter() {
        use crate::assets::layout::{ASSETS_DIR, fonts_dir};
        use crate::rendering::ui_fonts::UiFonts;
        use crate::rendering::version_overlay::{TEXT_SIZE, corner_rect};
        use crate::ui_layout::faces::BOLD_FACE;
        let layout = installed_layout();
        let spec = HudSpec::resolve(&layout).unwrap();
        let metrics = layout.family_metrics(BOLD_FACE).unwrap();
        let install = std::env::var(crate::assets::INSTALL_DIR_VAR).expect("OPENFO4_INSTALL_DIR");
        let fonts = UiFonts::load(&fonts_dir(&format!("{install}/{ASSETS_DIR}"))).unwrap();
        let text_width_stage = crate::version::overlay_lines()
            .iter()
            .map(|line| shaped_width(&fonts, line))
            .fold(0.0, f32::max);
        for fb in [
            (1920_u32, 1080_u32),
            (1280, 720),
            (2560, 1440),
            (3440, 1440),
        ] {
            let hud_layout =
                Layout::for_stage(spec.stage.0, spec.stage.1, fb.0 as f32, fb.1 as f32);
            let bar = meters::bar_rect(&hud_layout, &spec.hp);
            let overhang = hud_layout.s(spec.hp.overhang);
            let rail = hud_layout.s(spec.hp.thickness);
            let meter = (
                bar.x - overhang,
                bar.y,
                bar.w + 2.0 * overhang,
                bar.h + rail,
            );
            let text = corner_rect(
                spec.stage,
                fb,
                metrics,
                text_width_stage * hud_layout.scale(),
            );
            println!(
                "fb {fb:?} size {TEXT_SIZE} text {text:?} hp bracket+bar (x,y,w,h) {meter:?} label {:?}",
                spec.hp.label.geometry
            );
            let apart = text.x + text.w <= meter.0
                || meter.0 + meter.2 <= text.x
                || text.y + text.h <= meter.1
                || meter.1 + meter.3 <= text.y;
            assert!(apart, "the version text overlaps the HP meter at {fb:?}");
        }
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the installer's layout documents"]
    fn the_installed_movie_satisfies_the_hud_keys() {
        let layout = installed_layout();
        layout.validate(&[&LAYOUT_KEYS]).unwrap();
        let spec = HudSpec::resolve(&layout).unwrap();
        println!("stage {:?} hp {:?} ap {:?}", spec.stage, spec.hp, spec.ap);
    }
}
