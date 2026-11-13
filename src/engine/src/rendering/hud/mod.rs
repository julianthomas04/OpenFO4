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

mod backdrop;
mod bracket;
mod clip;
mod crosshair;
pub mod field_text;
pub(crate) mod layout;
mod message_list;
mod messages;
mod meters;
pub(crate) mod pipeline;
pub(crate) mod quad;
mod rollover;
pub(crate) mod shadow;
pub(crate) mod shape;
mod text_face;

use std::sync::Arc;

use crate::rendering::gpu_profiler::{GpuPass, GpuProfiler, SpanPart};
use crate::rendering::rollover_prompt::RolloverPrompt;
use crate::rendering::ui_colour::UiTint;
use crate::rendering::ui_fonts::UiFonts;
use crate::sim::{CrosshairState, HudSnapshot};
use crate::ui_layout::{ModuleKeys, UiLayout, UiLayoutError};
use crosshair::{CrosshairSpec, CrosshairWidget};
use field_text::{FieldText, TextExtent, TextItem};
use layout::{Anchor, Layout};
use messages::{MessagesSpec, MessagesWidget, OBSERVED};
use meters::{AP_KEYS, HP_KEYS, MeterSpec};
use pipeline::HudPipeline;
use quad::{HudVertex, QuadBuilder};
use rollover::{RolloverSpec, RolloverWidget};

const INITIAL_VBUF_VERTS: u64 = 1024;

/// Every layout key the HUD reads, and that the movie's pieces fit together as the HUD needs.
fn validate(layout: &UiLayout) -> Result<(), UiLayoutError> {
    HudSpec::resolve(layout).map(drop)
}

pub static LAYOUT_KEYS: ModuleKeys = ModuleKeys {
    keys: &[
        &HP_KEYS,
        &AP_KEYS,
        &meters::SET_TEXT,
        &crosshair::KEYS,
        &rollover::KEYS,
        &messages::KEYS,
    ],
    resolve: validate,
};

/// The HUD's values from the layout, resolved once.
struct HudSpec {
    stage: (f32, f32),
    hp: MeterSpec,
    ap: MeterSpec,
    crosshair: CrosshairSpec,
    rollover: RolloverSpec,
    messages: MessagesSpec,
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
            rollover: RolloverSpec::resolve(layout)?,
            messages: MessagesSpec::resolve(layout)?,
        })
    }
}

/// What the HUD shows: all of it, or only the message list over the open Pip-Boy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HudScope {
    Full,
    OverPipboy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HudPart {
    Meters,
    Crosshair,
    Rollover,
    Messages,
}

impl HudScope {
    fn shows(self, part: HudPart) -> bool {
        match self {
            Self::Full => true,
            Self::OverPipboy => part == HudPart::Messages,
        }
    }
}

/// The HUD without the GPU: what is shown, laid out and as vertices, and the clocks that move it.
struct HudScene {
    /// Retained, cleared+refilled each frame (capacity preserved).
    verts: Vec<HudVertex>,
    stage: (f32, f32),
    hp: MeterSpec,
    ap: MeterSpec,
    tint: UiTint,
    crosshair: CrosshairWidget,
    rollover: RolloverWidget,
    messages: MessagesWidget,
    /// The labels for the framebuffer size they were built for.
    labels: Vec<TextItem>,
    labels_for: (u32, u32),
    /// What the text pass draws, as composed for `text_scope`.
    text_items: Vec<TextItem>,
    text_scope: HudScope,
    fb: (u32, u32),
    /// Whether the HUD is shown this frame (false until the player has spawned,
    /// so meters and crosshair appear/disappear together — FO4 hides the HUD
    /// until the player is loaded).
    visible: bool,
}

impl HudScene {
    fn new(spec: HudSpec, layout: &Arc<UiLayout>, tint: UiTint) -> Self {
        Self {
            verts: Vec::with_capacity(INITIAL_VBUF_VERTS as usize),
            stage: spec.stage,
            hp: spec.hp,
            ap: spec.ap,
            tint,
            crosshair: CrosshairWidget::new(spec.crosshair),
            rollover: RolloverWidget::new(spec.rollover, Arc::clone(layout)),
            messages: MessagesWidget::new(spec.messages, OBSERVED),
            labels: Vec::new(),
            labels_for: (0, 0),
            text_items: Vec::new(),
            text_scope: HudScope::Full,
            fb: (0, 0),
            visible: false,
        }
    }

    /// Advances and lays out what `scope` shows. `world_dt` moves the crosshair and stands still
    /// while the world is held; `frame_dt` is the wall-clock delta and moves the message list.
    #[allow(
        clippy::too_many_arguments,
        reason = "one frame's inputs, named at the call"
    )]
    fn compose(
        &mut self,
        scope: HudScope,
        snapshot: &HudSnapshot,
        activate: bool,
        prompt: Option<&RolloverPrompt>,
        world_dt: f32,
        frame_dt: f32,
        fb: (u32, u32),
        mut measure: impl FnMut(&TextItem) -> TextExtent,
    ) {
        // The HUD is shown only once the player has spawned; before that the
        // meters and crosshair stay hidden together (no half-built overlay).
        self.visible = snapshot.visible;
        if !self.visible {
            self.verts.clear();
            return;
        }
        if scope.shows(HudPart::Crosshair) {
            self.crosshair
                .update(requested_crosshair(snapshot.crosshair, activate), world_dt);
        }

        let layout = Layout::for_stage(self.stage.0, self.stage.1, fb.0 as f32, fb.1 as f32);
        let hp_bar = meters::bar_rect(&layout, &self.hp);
        let ap_bar = meters::bar_rect(&layout, &self.ap);
        self.fb = fb;
        let mut changed = scope != self.text_scope;
        if scope.shows(HudPart::Meters) && (self.labels_for != fb || self.labels.is_empty()) {
            self.labels = vec![
                meters::label_item(&layout, &self.hp, hp_bar, &self.tint),
                meters::label_item(&layout, &self.ap, ap_bar, &self.tint),
            ];
            self.labels_for = fb;
            changed = true;
        }
        if scope.shows(HudPart::Rollover) {
            changed |= self
                .rollover
                .update(prompt, fb, &layout, &self.tint, &mut measure);
        }
        if scope.shows(HudPart::Messages) {
            changed |= self
                .messages
                .update(frame_dt, fb, &layout, &self.tint, &mut measure);
        }
        if changed {
            self.text_scope = scope;
            self.text_items.clear();
            if scope.shows(HudPart::Meters) {
                self.text_items.extend_from_slice(&self.labels);
            }
            if scope.shows(HudPart::Rollover) {
                self.text_items.extend_from_slice(self.rollover.items());
            }
            if scope.shows(HudPart::Messages) {
                self.text_items.extend_from_slice(self.messages.items());
            }
        }

        let shadow = layout.s(shadow::QUAD_OFFSET_STAGE_PX);
        let mut qb = QuadBuilder::new(
            &mut self.verts,
            fb.0 as f32,
            fb.1 as f32,
            shadow,
            shadow::QUAD_COLOUR,
        );
        if scope.shows(HudPart::Meters) {
            meters::emit(&mut qb, &layout, &self.hp, hp_bar, snapshot.hp);
            meters::emit(&mut qb, &layout, &self.ap, ap_bar, snapshot.ap);
        }
        if scope.shows(HudPart::Crosshair) {
            self.crosshair.emit(&mut qb, &layout);
        }
        if scope.shows(HudPart::Messages) {
            self.messages.emit(&mut qb, &layout);
        }
        qb.finish();
    }
}

pub struct HudRenderer {
    pipeline: HudPipeline,
    tint_bind_group: wgpu::BindGroup,
    vbuf: wgpu::Buffer,
    vbuf_verts: u64,
    scene: HudScene,
    text: FieldText,
}

impl HudRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        fonts: &UiFonts,
        layout: &Arc<UiLayout>,
        tint: UiTint,
    ) -> anyhow::Result<Self> {
        let spec = HudSpec::resolve(layout)?;
        for meter in [&spec.hp, &spec.ap] {
            fonts.require_family(&meter.label.face.family)?;
        }
        for family in spec.rollover.families() {
            fonts.require_family(family)?;
        }
        fonts.require_family(spec.messages.family())?;
        let pipeline = HudPipeline::new(device, format);
        let text = FieldText::new(device, queue, format, fonts, glyphon::ColorMode::Accurate);
        let tint_bind_group = pipeline.create_tint_bind_group(device, &tint);
        let vbuf = Self::create_vbuf(device, INITIAL_VBUF_VERTS);
        Ok(Self {
            pipeline,
            tint_bind_group,
            vbuf,
            vbuf_verts: INITIAL_VBUF_VERTS,
            scene: HudScene::new(spec, layout, tint),
            text,
        })
    }

    pub fn stage(&self) -> (f32, f32) {
        self.scene.stage
    }

    /// Queues one line; shown from the next update.
    pub(in crate::rendering) fn show_message(&mut self, text: Arc<str>) {
        self.scene.messages.push(text);
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
    /// (door-aim, selected render-side); `prompt` is the rollover text to show, if any;
    /// `world_dt` is the world's frame delta (nothing while the world is held) and `frame_dt`
    /// the wall-clock one, which moves the message list.
    #[allow(
        clippy::too_many_arguments,
        reason = "one frame's inputs, named at the call"
    )]
    pub(in crate::rendering) fn update(
        &mut self,
        snapshot: &HudSnapshot,
        activate: bool,
        prompt: Option<&RolloverPrompt>,
        world_dt: f32,
        frame_dt: f32,
        fb_w: u32,
        fb_h: u32,
    ) {
        let text = &mut self.text;
        self.scene.compose(
            HudScope::Full,
            snapshot,
            activate,
            prompt,
            world_dt,
            frame_dt,
            (fb_w, fb_h),
            |item| text.measure(item),
        );
    }

    /// The update while the Pip-Boy is open: only the message list, on the wall-clock delta.
    pub(in crate::rendering) fn update_over_pipboy(
        &mut self,
        snapshot: &HudSnapshot,
        frame_dt: f32,
        fb_w: u32,
        fb_h: u32,
    ) {
        let text = &mut self.text;
        self.scene.compose(
            HudScope::OverPipboy,
            snapshot,
            false,
            None,
            0.0,
            frame_dt,
            (fb_w, fb_h),
            |item| text.measure(item),
        );
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
        if !self.scene.visible {
            return;
        }
        // Upload text glyphs/positions (no encoder needed) and the quad
        // vertices before the pass.
        self.text
            .prepare(device, queue, self.scene.fb, &self.scene.text_items);
        let verts = &self.scene.verts;
        if !verts.is_empty() {
            let needed = verts.len() as u64;
            if needed > self.vbuf_verts {
                self.vbuf_verts = needed.next_power_of_two();
                self.vbuf = Self::create_vbuf(device, self.vbuf_verts);
            }
            queue.write_buffer(&self.vbuf, 0, bytemuck::cast_slice(verts));
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
        if !verts.is_empty() {
            pass.set_pipeline(&self.pipeline.pipeline);
            pass.set_bind_group(0, &self.tint_bind_group, &[]);
            pass.set_vertex_buffer(0, self.vbuf.slice(..));
            pass.draw(0..verts.len() as u32, 0..1);
        }
        // Text (numbers/labels) composited over the bars, in the same pass.
        self.text.render_into(&mut pass);
    }
}

/// The crosshair to draw: the activate shape over any visible one when something can be activated.
fn requested_crosshair(current: CrosshairState, activate: bool) -> CrosshairState {
    if activate && current != CrosshairState::Hidden {
        CrosshairState::Activate
    } else {
        current
    }
}

#[cfg(test)]
mod crosshair_tests {
    use super::*;

    #[test]
    fn something_to_activate_switches_a_visible_crosshair_to_the_activate_shape() {
        assert_eq!(
            requested_crosshair(CrosshairState::Dot, true),
            CrosshairState::Activate
        );
        assert_eq!(
            requested_crosshair(CrosshairState::Dot, false),
            CrosshairState::Dot
        );
        assert_eq!(
            requested_crosshair(CrosshairState::Hidden, true),
            CrosshairState::Hidden
        );
    }
}

#[cfg(test)]
mod scene_tests {
    use super::*;

    const FB: (u32, u32) = (1280, 720);
    const LINE: &str = "Keys stay in the bag.";

    /// A scene on synthetic movie values: the meter tests' meters, a one-rect crosshair, the rollover
    /// tests' rollover, and the message list of the message tests' fixture.
    fn scene() -> HudScene {
        let (rollover, layout) = rollover::tests::synthetic();
        let (hp, ap) = meters::tests::synthetic_meters();
        let spec = HudSpec {
            stage: (640.0, 360.0),
            hp,
            ap,
            crosshair: CrosshairSpec::synthetic(),
            rollover,
            messages: messages::tests::synthetic_spec(),
        };
        let tint = UiTint {
            color: [0.0, 1.0, 0.0],
            opacity: 1.0,
        };
        HudScene::new(spec, &layout, tint)
    }

    fn snapshot() -> HudSnapshot {
        HudSnapshot {
            visible: true,
            hp: crate::sim::MeterFill { fraction: 0.8 },
            ap: crate::sim::MeterFill { fraction: 0.6 },
            crosshair: CrosshairState::Dot,
        }
    }

    fn measure(_: &TextItem) -> TextExtent {
        TextExtent {
            left: 0.0,
            top: 0.0,
            width: 100.0,
            height: 20.0,
        }
    }

    fn compose(scene: &mut HudScene, scope: HudScope, world_dt: f32, frame_dt: f32) {
        scene.compose(
            scope,
            &snapshot(),
            false,
            None,
            world_dt,
            frame_dt,
            FB,
            measure,
        );
    }

    fn push(scene: &mut HudScene) {
        scene.messages.push(Arc::from(LINE));
    }

    #[test]
    fn the_message_list_advances_with_frame_time_while_the_world_is_held() {
        let mut scene = scene();
        push(&mut scene);
        compose(&mut scene, HudScope::Full, 0.0, 0.0);
        assert_eq!(scene.messages.items().len(), 1);
        for _ in 0..40 {
            compose(&mut scene, HudScope::Full, 0.0, 0.25);
        }
        assert_eq!(scene.messages.items(), []);
    }

    #[test]
    fn over_the_pip_boy_the_list_ages_on_frame_time_with_the_same_clamp() {
        let long_hitch = [0.25_f32, 10.0];
        let ages = long_hitch.map(|frame_dt| {
            let mut scene = scene();
            push(&mut scene);
            compose(&mut scene, HudScope::OverPipboy, 0.0, 0.0);
            compose(&mut scene, HudScope::OverPipboy, 0.0, frame_dt);
            scene.messages.items().len()
        });
        assert_eq!(ages[0], ages[1]);
    }

    #[test]
    fn a_refusal_over_the_open_pip_boy_reaches_the_list_and_nothing_else_of_the_hud_is_drawn() {
        let mut scene = scene();
        push(&mut scene);
        compose(&mut scene, HudScope::OverPipboy, 0.0, 0.05);
        compose(&mut scene, HudScope::OverPipboy, 0.0, 0.05);
        assert_eq!(scene.text_items.len(), 1);
        assert_eq!(scene.text_items, scene.messages.items());
        let layout = Layout::for_stage(scene.stage.0, scene.stage.1, FB.0 as f32, FB.1 as f32);
        let mut only_messages = Vec::new();
        let mut qb = QuadBuilder::new(
            &mut only_messages,
            FB.0 as f32,
            FB.1 as f32,
            layout.s(shadow::QUAD_OFFSET_STAGE_PX),
            shadow::QUAD_COLOUR,
        );
        scene.messages.emit(&mut qb, &layout);
        qb.finish();
        assert!(!scene.verts.is_empty());
        assert_eq!(scene.verts.len(), only_messages.len());
    }

    #[test]
    fn closing_the_pip_boy_brings_the_whole_hud_back() {
        let mut scene = scene();
        compose(&mut scene, HudScope::Full, 0.0, 0.0);
        let before = scene.text_items.clone();
        assert_ne!(before, Vec::new());
        compose(&mut scene, HudScope::OverPipboy, 0.0, 0.0);
        assert_eq!(scene.text_items, Vec::new());
        compose(&mut scene, HudScope::Full, 0.0, 0.0);
        assert_eq!(scene.text_items, before);
    }

    #[test]
    fn a_line_shown_across_an_open_and_a_close_is_shown_once_and_ages_through_both() {
        let mut scene = scene();
        push(&mut scene);
        compose(&mut scene, HudScope::Full, 0.0, 0.0);
        let hud_items = scene.text_items.len();
        compose(&mut scene, HudScope::OverPipboy, 0.0, 0.05);
        assert_eq!(scene.text_items, scene.messages.items());
        compose(&mut scene, HudScope::Full, 0.0, 0.05);
        assert_eq!(scene.text_items.len(), hud_items);
        let lines_in_hud = scene
            .text_items
            .iter()
            .filter(|item| scene.messages.items().contains(item))
            .count();
        assert_eq!(lines_in_hud, scene.messages.items().len());
        assert_eq!(scene.messages.items().len(), 1);
        for _ in 0..4 {
            compose(&mut scene, HudScope::OverPipboy, 0.0, 0.25);
        }
        assert_eq!(scene.messages.items(), []);
        compose(&mut scene, HudScope::Full, 0.0, 0.0);
        assert_eq!(scene.messages.items(), []);
    }
}

#[cfg(test)]
mod scope_tests {
    use super::*;

    #[test]
    fn over_the_pip_boy_only_the_message_list_is_shown() {
        for part in [HudPart::Meters, HudPart::Crosshair, HudPart::Rollover] {
            assert!(HudScope::Full.shows(part), "{part:?}");
            assert!(!HudScope::OverPipboy.shows(part), "{part:?}");
        }
        assert!(HudScope::Full.shows(HudPart::Messages));
        assert!(HudScope::OverPipboy.shows(HudPart::Messages));
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
                spec.hp.label.face.geometry
            );
            let apart = text.x + text.w <= meter.0
                || meter.0 + meter.2 <= text.x
                || text.y + text.h <= meter.1
                || meter.1 + meter.3 <= text.y;
            assert!(apart, "the version text overlaps the HP meter at {fb:?}");
        }
    }

    fn scene() -> HudScene {
        let layout = Arc::new(installed_layout());
        let spec = HudSpec::resolve(&layout).unwrap();
        let tint = UiTint {
            color: [0.0, 1.0, 0.0],
            opacity: 1.0,
        };
        HudScene::new(spec, &layout, tint)
    }

    fn snapshot() -> HudSnapshot {
        HudSnapshot {
            visible: true,
            hp: crate::sim::MeterFill { fraction: 0.8 },
            ap: crate::sim::MeterFill { fraction: 0.6 },
            crosshair: CrosshairState::Dot,
        }
    }

    fn measure(_: &TextItem) -> TextExtent {
        TextExtent {
            left: 0.0,
            top: 0.0,
            width: 100.0,
            height: 20.0,
        }
    }

    const FB: (u32, u32) = (1280, 720);

    fn compose(scene: &mut HudScene, scope: HudScope, world_dt: f32, frame_dt: f32) {
        scene.compose(
            scope,
            &snapshot(),
            false,
            None,
            world_dt,
            frame_dt,
            FB,
            measure,
        );
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the installer's layout documents"]
    fn the_message_list_advances_with_frame_time_while_the_world_is_held() {
        let mut scene = scene();
        scene
            .messages
            .push(Arc::from("Too heavy for the test to lift."));
        compose(&mut scene, HudScope::Full, 0.0, 0.25);
        assert_eq!(scene.messages.items().len(), 1);
        for _ in 0..40 {
            compose(&mut scene, HudScope::Full, 0.0, 0.25);
        }
        assert_eq!(scene.messages.items(), []);
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the installer's layout documents"]
    fn a_refusal_over_the_open_pip_boy_reaches_the_list_and_nothing_else_of_the_hud_is_drawn() {
        let mut scene = scene();
        compose(&mut scene, HudScope::Full, 0.0, 0.0);
        scene.messages.push(Arc::from("Keys stay in the bag."));
        compose(&mut scene, HudScope::OverPipboy, 0.0, 0.25);
        compose(&mut scene, HudScope::OverPipboy, 0.0, 0.25);
        assert_eq!(scene.text_items, scene.messages.items());
        assert_eq!(scene.text_items.len(), 1);
        let layout = Layout::for_stage(scene.stage.0, scene.stage.1, FB.0 as f32, FB.1 as f32);
        let mut only_messages = Vec::new();
        let mut qb = QuadBuilder::new(
            &mut only_messages,
            FB.0 as f32,
            FB.1 as f32,
            layout.s(shadow::QUAD_OFFSET_STAGE_PX),
            shadow::QUAD_COLOUR,
        );
        scene.messages.emit(&mut qb, &layout);
        qb.finish();
        assert!(!scene.verts.is_empty());
        assert_eq!(scene.verts.len(), only_messages.len());
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the installer's layout documents"]
    fn closing_the_pip_boy_brings_the_whole_hud_back() {
        let mut scene = scene();
        compose(&mut scene, HudScope::Full, 0.0, 0.0);
        let before = scene.text_items.clone();
        compose(&mut scene, HudScope::OverPipboy, 0.0, 0.0);
        assert_eq!(scene.text_items, Vec::new());
        compose(&mut scene, HudScope::Full, 0.0, 0.0);
        assert_eq!(scene.text_items, before);
        assert_ne!(before, Vec::new());
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
