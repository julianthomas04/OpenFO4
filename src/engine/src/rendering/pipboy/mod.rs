//! The Pip-Boy screen: a full-screen 2D overlay painted with its own egui context over the paused
//! world. Reads `sim::pipboy::PipBoySnapshot`, emits `PipBoyAction`s.

pub mod art;
pub mod bottom_bar;
pub mod fonts;
pub mod grid_view;
pub mod header;
pub mod hint_bar;
pub mod input;
pub mod nav;
pub mod notice;
pub mod pages;
pub mod paint;
pub mod perk_grid;
pub mod scaleform_text;
pub mod scrolling_list;
pub mod shapes;
pub mod stage;
pub mod summary;
pub mod tab;
#[cfg(test)]
pub mod test_layout;
pub mod world_map_texture;

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use egui::emath::TSTransform;
use egui::{Color32, Id, LayerId, Order, Pos2, RawInput, Rect, Vec2};
use winit::event::WindowEvent;

use self::art::ArtCache;
use self::bottom_bar::{BarInputs, BottomBarLayout};
use self::fonts::PipBoyFonts;
use self::grid_view::GridViewHint;
use self::header::{Chrome, ChromeHit, HeaderLayout};
use self::hint_bar::HintBarLayout;
use self::input::{InputTranslator, ShellInput};
use self::nav::{Nav, Screen, Step};
use self::notice::{NoticeLine, PERK_CHART_LINE_TOP, SHELL_LINE_TOP};
use self::pages::Pages;
use self::perk_grid::{PerkGridError, PerkGridTextures};
use self::stage::{StageFit, Stages};
use self::tab::{HintKey, InputResult, NavInput, ShellRequest, Tab, TabContext};
use self::world_map_texture::{WorldMapImage, WorldMapTextureError};
use crate::assets::layout;
use crate::assets::pipboy_art::{ArtLibrary, ArtManifestError};
use crate::rendering::egui_pass::{EguiPass, EguiTarget};
use crate::rendering::game_settings::SafeZone;
use crate::rendering::gpu_profiler::{GpuPass, GpuProfiler};
use crate::rendering::ui_colour::{UiColourSet, UiColours};
use crate::sim::pipboy::{PipBoyAction, PipBoyActionError, PipBoySnapshot};
use crate::ui_layout::set_text::SetTextKeys;
use crate::ui_layout::{KeyGroup, TranslateKey, UiLayout, UiLayoutError};

const OPEN_ENV: &str = "OPENFO4_PIPBOY_OPEN";
const SELECT_ENV: &str = "OPENFO4_PIPBOY_SELECT";
const LAYER_NAME: &str = "pipboy";
const HINT_LAYER_NAME: &str = "pipboy-hints";

/// The capture hooks: open a screen as soon as the player has spawned, with a list row selected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PipBoyEnv {
    pub open: Option<Screen>,
    pub select_row: Option<usize>,
}

impl PipBoyEnv {
    pub fn from_env() -> anyhow::Result<Self> {
        let open = crate::utils::optional_env_var(OPEN_ENV)
            .map(|text| nav::parse_screen(&text))
            .transpose()
            .map_err(|error| anyhow::anyhow!("{OPEN_ENV}: {error}"))?;
        let select_row = crate::utils::optional_env_var(SELECT_ENV)
            .map(|text| text.trim().parse::<usize>())
            .transpose()
            .map_err(|error| anyhow::anyhow!("{SELECT_ENV}: not a row number ({error})"))?;
        Ok(Self { open, select_row })
    }
}

pub(crate) static SET_TEXT: SetTextKeys = SetTextKeys::new("PipboyMenu");

/// Every layout key the Pip-Boy reads; `UiLayout::validate` resolves them all at boot.
pub static LAYOUT_KEYS: KeyGroup = KeyGroup(&[
    &bottom_bar::LAYOUT_KEYS,
    &SET_TEXT,
    &pages::LAYOUT_KEYS,
    &pages::PERK_CHART_KEYS,
    &hint_bar::LAYOUT_KEYS,
    &grid_view::LAYOUT_KEYS,
    &header::LAYOUT_KEYS,
    &stage::LAYOUT_KEYS,
]);

fn layer() -> LayerId {
    LayerId::new(Order::Foreground, Id::new(LAYER_NAME))
}

fn hint_layer() -> LayerId {
    LayerId::new(Order::Foreground, Id::new(HINT_LAYER_NAME))
}

/// Builds a [`TabContext`] over the screen's disjoint fields, so the active tab can be borrowed
/// from `pages` at the same time.
macro_rules! tab_context {
    ($screen:ident, $painter:expr, $snapshot:expr, $tint:expr) => {
        TabContext {
            painter: $painter,
            snapshot: $snapshot,
            art_library: &$screen.art_library,
            layout: &$screen.layout,
            art: &mut $screen.art,
            world_map_texture: $screen.world_map_texture,
            perk_grid: $screen.perk_grid,
            tint: $tint,
            seconds_open: $screen.seconds_open,
            actions: &mut $screen.actions,
            requests: &mut $screen.requests,
        }
    };
}

/// One painted frame: the egui output and the fit it was laid out for.
pub struct PaintedFrame {
    pub output: egui::FullOutput,
    pub fit: StageFit,
}

/// Everything about the Pip-Boy that needs no GPU: navigation, pages, input, painting into an
/// egui context.
pub struct PipBoyScreen {
    ctx: egui::Context,
    /// Full white in the Pip-Boy colour: the pages, header, bottom bar and message line.
    tint: Color32,
    /// Full white in the HUD colour: the perk chart and the screen-space hint bar.
    hud_tint: Color32,
    nav: Nav,
    chrome: Chrome,
    bottom_bar: BottomBarLayout,
    hint_bar: HintBarLayout,
    grid_view: GridViewHint,
    safe_zone: SafeZone,
    stages: Stages,
    pages: Pages,
    art: ArtCache,
    art_library: Arc<ArtLibrary>,
    layout: Arc<UiLayout>,
    world_map_texture: Option<WorldMapImage>,
    perk_grid: Option<PerkGridTextures>,
    input: InputTranslator,
    actions: Vec<PipBoyAction>,
    requests: Vec<ShellRequest>,
    notice: NoticeLine,
    shown_rejection: Option<PipBoyActionError>,
    pointer_sync_pending: bool,
    window_px: Vec2,
    seconds_open: f64,
}

impl PipBoyScreen {
    pub fn new(
        fonts: PipBoyFonts,
        art_library: Arc<ArtLibrary>,
        layout: Arc<UiLayout>,
        colours: &UiColours,
        safe_zone: SafeZone,
    ) -> Result<Self, UiLayoutError> {
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts.definitions());
        let stages = Stages::resolve(&layout)?;
        let pages = Pages::new(&layout)?;
        Ok(Self {
            ctx,
            tint: colours.tint_for(UiColourSet::Pipboy).srgb_color32(),
            hud_tint: colours.tint_for(UiColourSet::Hud).srgb_color32(),
            nav: Nav::default(),
            chrome: Chrome::new(HeaderLayout::resolve(&layout)?),
            bottom_bar: BottomBarLayout::resolve(&layout)?,
            hint_bar: HintBarLayout::resolve(&layout)?,
            grid_view: GridViewHint::resolve(&layout)?,
            safe_zone,
            stages,
            pages,
            art: ArtCache::default(),
            art_library,
            layout,
            world_map_texture: None,
            perk_grid: None,
            input: InputTranslator::default(),
            actions: Vec::new(),
            requests: Vec::new(),
            notice: NoticeLine::default(),
            shown_rejection: None,
            pointer_sync_pending: false,
            window_px: stages.shell(),
            seconds_open: 0.0,
        })
    }

    pub fn nav(&self) -> &Nav {
        &self.nav
    }

    pub fn fit(&self) -> StageFit {
        let (stage, window_fraction) = self.stages.of(self.nav.screen());
        StageFit::new(stage, self.window_px, window_fraction)
    }

    pub fn set_window_size(&mut self, window_px: Vec2) {
        self.window_px = window_px;
    }

    /// Shows `screen`, optionally with list row `select_row` selected on it.
    pub fn show(&mut self, screen: Screen, select_row: Option<usize>) {
        self.nav.open(screen);
        if let (Some(row), Screen::Shell(tab)) = (select_row, screen) {
            self.pages.tab_mut(tab).select_row(row);
        }
    }

    /// Shows `page` with its remembered sub-tab (the open-key shortcuts).
    pub fn show_page(&mut self, page: nav::Page) {
        self.nav.close_perk_chart();
        self.nav.select_page(page);
    }

    /// Called when the Pip-Boy opens: forgets button, slide, message and prompt state of the last
    /// session. The cursor position stays so hover and the first click work at once.
    pub fn reset_session(&mut self) {
        self.input.begin_session();
        self.chrome.reset();
        self.notice.clear();
        self.pages.on_open();
        self.seconds_open = 0.0;
        self.pointer_sync_pending = true;
    }

    /// Tracks the cursor while the Pip-Boy is closed.
    pub fn observe_window_event(&mut self, event: &WindowEvent) {
        self.input.observe(event);
    }

    pub fn set_world_map_texture(&mut self, texture: Option<WorldMapImage>) {
        self.world_map_texture = texture;
    }

    pub fn set_perk_grid(&mut self, textures: PerkGridTextures) {
        self.perk_grid = Some(textures);
    }

    pub fn take_actions(&mut self) -> Vec<PipBoyAction> {
        std::mem::take(&mut self.actions)
    }

    /// Routes one window event. Returns whether the Pip-Boy consumed it.
    pub fn handle_window_event(
        &mut self,
        event: &WindowEvent,
        snapshot: &PipBoySnapshot<'_>,
    ) -> bool {
        let translated = self.input.translate(event, self.fit());
        for input in translated.inputs {
            self.dispatch(input, snapshot);
        }
        translated.consumed
    }

    fn active_tab_is_modal(&mut self) -> bool {
        match self.nav.screen() {
            Screen::PerkChart => self.pages.perk_chart_mut().is_modal(),
            Screen::Shell(tab) => self.pages.tab_mut(tab).is_modal(),
        }
    }

    pub fn dispatch(&mut self, input: ShellInput, snapshot: &PipBoySnapshot<'_>) {
        let modal = self.active_tab_is_modal();
        match input {
            ShellInput::Nav(nav_input) => self.dispatch_nav(nav_input, modal, snapshot),
            ShellInput::PrevPage | ShellInput::NextPage if modal => {}
            ShellInput::PrevPage => self.step_page(Step::Previous),
            ShellInput::NextPage => self.step_page(Step::Next),
            ShellInput::Close => {
                if self.offer_to_tab(NavInput::Hint(HintKey::Tab), snapshot) == InputResult::Ignored
                {
                    if self.nav.screen() == Screen::PerkChart {
                        self.nav.close_perk_chart();
                    } else {
                        self.actions.push(PipBoyAction::Close);
                    }
                }
            }
        }
    }

    fn dispatch_nav(&mut self, input: NavInput, modal: bool, snapshot: &PipBoySnapshot<'_>) {
        if !modal
            && let (NavInput::Clicked(pos), Screen::Shell(_)) = (input, self.nav.screen())
            && let Some(hit) = self.chrome.hit_test(pos)
        {
            self.apply_chrome_hit(hit);
            return;
        }
        if self.offer_to_tab(input, snapshot) == InputResult::Consumed || modal {
            return;
        }
        match input {
            NavInput::PrevSubTab => self.step_sub(Step::Previous),
            NavInput::NextSubTab => self.step_sub(Step::Next),
            NavInput::Hint(HintKey::T) => {
                if let Screen::Shell(tab) = self.nav.screen()
                    && self.pages.tab_mut(tab).can_lower_pipboy()
                {
                    self.open_perk_chart();
                }
            }
            _ => {}
        }
    }

    fn open_perk_chart(&mut self) {
        self.nav.open_perk_chart();
        self.pages.perk_chart_mut().resume();
    }

    /// The full-white colour of the colour set `screen` is drawn in.
    fn tint_of(&self, screen: Screen) -> Color32 {
        match screen {
            Screen::PerkChart => self.hud_tint,
            Screen::Shell(_) => self.tint,
        }
    }

    fn offer_to_tab(&mut self, input: NavInput, snapshot: &PipBoySnapshot<'_>) -> InputResult {
        let painter = self.ctx.layer_painter(layer());
        let screen = self.nav.screen();
        let tint = self.tint_of(screen);
        let mut cx = tab_context!(self, &painter, snapshot, tint);
        let result = match screen {
            Screen::PerkChart => {
                let chart = self.pages.perk_chart_mut();
                if let NavInput::Dragged { delta, button } = input {
                    chart.dragged(button, delta, &mut cx);
                    InputResult::Consumed
                } else {
                    chart.handle_input(input, &mut cx)
                }
            }
            Screen::Shell(tab) => self.pages.tab_mut(tab).handle_input(input, &mut cx),
        };
        self.apply_requests();
        result
    }

    fn apply_requests(&mut self) {
        for request in std::mem::take(&mut self.requests) {
            match request {
                ShellRequest::OpenPerkChart => self.open_perk_chart(),
                ShellRequest::ClosePerkChart => self.nav.close_perk_chart(),
                ShellRequest::ShowOnMap(position) => {
                    self.nav.select_page(nav::Page::Map);
                    self.pages.focus_map(position);
                }
                ShellRequest::Notify(text) => self.notice.show(text, self.seconds_open),
            }
        }
    }

    fn apply_chrome_hit(&mut self, hit: ChromeHit) {
        match hit {
            ChromeHit::Page(page) => {
                self.nav.select_page(page);
            }
            ChromeHit::SubTab(index) => {
                let step = if index < self.nav.sub_index() {
                    Step::Previous
                } else {
                    Step::Next
                };
                if self.nav.select_sub(index) {
                    self.chrome.start_slide(step, self.seconds_open);
                }
            }
        }
    }

    fn step_sub(&mut self, step: Step) {
        if let Some(step) = self.nav.step_sub(step) {
            self.chrome.start_slide(step, self.seconds_open);
        }
    }

    fn step_page(&mut self, step: Step) {
        if matches!(self.nav.screen(), Screen::Shell(_)) {
            self.nav.step_page(step);
        }
    }

    /// Lays out and paints one frame into the egui context.
    pub fn paint_frame(
        &mut self,
        window_px: Vec2,
        snapshot: &PipBoySnapshot<'_>,
        seconds_open: f64,
    ) -> PaintedFrame {
        self.window_px = window_px;
        self.seconds_open = seconds_open;
        self.show_new_rejection(snapshot);
        let screen = self.nav.screen();
        let fit = self.fit();
        if std::mem::take(&mut self.pointer_sync_pending)
            && let Some(pointer) = self.input.pointer(fit)
        {
            self.dispatch_nav(NavInput::PointerMoved(pointer), false, snapshot);
        }
        self.art.set_scale(fit.scale);
        self.ctx.set_pixels_per_point(fit.scale);
        self.ctx
            .set_transform_layer(layer(), TSTransform::from_translation(fit.origin_points()));
        let raw_input = RawInput {
            screen_rect: Some(fit.screen_rect_points(window_px)),
            ..RawInput::default()
        };
        let ctx = self.ctx.clone();
        let output = ctx.run_ui(raw_input, |_ui| {
            self.draw(&ctx, screen, snapshot);
        });
        PaintedFrame { output, fit }
    }

    /// Puts a rejection the sim reports onto the message line once per change.
    fn show_new_rejection(&mut self, snapshot: &PipBoySnapshot<'_>) {
        let current = &snapshot.model.last_rejection;
        if *current != self.shown_rejection {
            if let Some(error) = current {
                self.show_rejection_text(error);
            }
            self.shown_rejection.clone_from(current);
        }
    }

    /// Shows the game's own refusal text for `error`; a rejection the game has no text for only
    /// reaches the log.
    fn show_rejection_text(&mut self, error: &PipBoyActionError) {
        let Some(key) = error.translation_key() else {
            tracing::warn!("Pip-Boy action rejected: {error}");
            return;
        };
        match self.layout.get(&TranslateKey::text(key)) {
            Ok(text) => self.notice.show(text, self.seconds_open),
            Err(error) => tracing::error!("Pip-Boy refusal text '{key}': {error}"),
        }
    }

    fn draw(&mut self, ctx: &egui::Context, screen: Screen, snapshot: &PipBoySnapshot<'_>) {
        let stage = Rect::from_min_size(Pos2::ZERO, self.stages.of(screen).0);
        let painter = ctx.layer_painter(layer()).with_clip_rect(stage);
        let tint = self.tint_of(screen);
        let mut cx = tab_context!(self, &painter, snapshot, tint);
        match screen {
            Screen::PerkChart => {
                self.pages.perk_chart_mut().draw(&mut cx);
                self.notice.draw(&cx, stage.center().x, PERK_CHART_LINE_TOP);
            }
            Screen::Shell(tab_id) => {
                self.chrome.draw(&cx, &self.nav);
                let tab = self.pages.tab_mut(tab_id);
                let inputs = BarInputs {
                    selected_hp_gain: tab.selected_hp_gain(snapshot),
                };
                self.bottom_bar.draw(&mut cx, tab.bottom_bar(), inputs);
                tab.draw(&mut cx);
                tab.draw_overlay(&mut cx);
                self.notice.draw(&cx, stage.center().x, SHELL_LINE_TOP);
                let mut hints = tab.hints(snapshot);
                if tab.can_lower_pipboy() {
                    let points = snapshot.model.character.progression.unspent_perk_points;
                    hints.push(self.grid_view.hint(points));
                }
                self.paint_hints(ctx, &hints);
            }
        }
        self.apply_requests();
    }

    /// Paints `hints` in the screen-space hint bar, over everything else.
    fn paint_hints(&self, ctx: &egui::Context, hints: &[tab::ButtonHint]) {
        let screen_points = self.window_px / ctx.pixels_per_point();
        let fit = self.hint_bar.fit(screen_points);
        ctx.set_transform_layer(hint_layer(), fit.transform);
        let painter = ctx.layer_painter(hint_layer()).with_clip_rect(fit.visible);
        self.hint_bar.paint(
            &painter,
            &self.layout,
            &fit,
            self.safe_zone,
            self.hud_tint,
            hints,
            self.seconds_open,
        );
    }
}

/// The Pip-Boy as the frame loop sees it: the CPU screen plus its own egui renderer.
pub struct PipBoy {
    screen: PipBoyScreen,
    pass: EguiPass,
    asset_root: String,
    opened_at: Option<Instant>,
}

impl PipBoy {
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        asset_root: &str,
        art_library: Arc<ArtLibrary>,
        layout: Arc<UiLayout>,
        colours: &UiColours,
        safe_zone: SafeZone,
    ) -> anyhow::Result<Self> {
        let fonts = PipBoyFonts::load(Path::new(&layout::fonts_dir(asset_root)))?;
        Ok(Self {
            screen: PipBoyScreen::new(fonts, art_library, layout, colours, safe_zone)?,
            pass: EguiPass::new(device, surface_format),
            asset_root: asset_root.to_owned(),
            opened_at: None,
        })
    }

    pub fn screen_mut(&mut self) -> &mut PipBoyScreen {
        &mut self.screen
    }

    /// Loads and registers the world-map image the MAP page draws; run once at startup when the
    /// catalog has a map.
    pub fn load_world_map(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_key: &str,
    ) -> Result<(), WorldMapTextureError> {
        let texture =
            world_map_texture::load(device, queue, &mut self.pass, &self.asset_root, texture_key)?;
        self.screen.set_world_map_texture(Some(texture));
        Ok(())
    }

    /// Fails when the art lacks the clip of any of `marker_types`; run once at startup so a map
    /// marker without art stops the load.
    pub fn require_marker_art(
        &self,
        marker_types: impl IntoIterator<Item = u16>,
    ) -> Result<(), ArtManifestError> {
        self.screen.art_library.require_marker_clips(marker_types)
    }

    /// Loads the perk chart's poster textures and installs the overlay's multiply pipeline; run
    /// once at startup.
    pub fn load_perk_grid(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
    ) -> Result<(), PerkGridError> {
        let textures = perk_grid::load(
            device,
            queue,
            &mut self.pass,
            &self.asset_root,
            surface_format,
        )?;
        self.screen.set_perk_grid(textures);
        Ok(())
    }

    /// Follows the model's open flag: starts the animation clock and clears stale input state on
    /// opening.
    pub fn sync_open(&mut self, open: bool) {
        match (open, self.opened_at) {
            (true, None) => {
                self.opened_at = Some(Instant::now());
                self.screen.reset_session();
            }
            (false, Some(_)) => self.opened_at = None,
            _ => {}
        }
    }

    pub fn handle_window_event(
        &mut self,
        event: &WindowEvent,
        snapshot: &PipBoySnapshot<'_>,
    ) -> bool {
        self.screen.handle_window_event(event, snapshot)
    }

    /// Lets the Pip-Boy track window events it does not own while closed (the cursor).
    pub fn observe_window_event(&mut self, event: &WindowEvent) {
        self.screen.observe_window_event(event);
    }

    pub fn take_actions(&mut self) -> Vec<PipBoyAction> {
        self.screen.take_actions()
    }

    pub fn set_window_size(&mut self, width: u32, height: u32) {
        self.screen
            .set_window_size(Vec2::new(width as f32, height as f32));
    }

    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        size_in_pixels: [u32; 2],
        profiler: &GpuProfiler,
        snapshot: &PipBoySnapshot<'_>,
    ) {
        let seconds_open = self
            .opened_at
            .map_or(0.0, |opened| opened.elapsed().as_secs_f64());
        let window_px = Vec2::new(size_in_pixels[0] as f32, size_in_pixels[1] as f32);
        let frame = self.screen.paint_frame(window_px, snapshot, seconds_open);
        let target = EguiTarget {
            label: "Pip-Boy Pass",
            view,
            size_in_pixels,
            pixels_per_point: frame.fit.scale,
            profiled_as: GpuPass::Hud,
        };
        self.pass.draw(
            device,
            queue,
            encoder,
            profiler,
            &target,
            &self.screen.ctx,
            frame.output,
        );
    }
}

#[cfg(test)]
mod tests;
