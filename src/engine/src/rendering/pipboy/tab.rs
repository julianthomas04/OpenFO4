//! The contract every Pip-Boy screen implements, so the shell can host pages written in parallel.

use egui::{Color32, Painter, Pos2, Vec2};

use super::art::ArtCache;
use super::paint::{draw_sprite, draw_sprite_affine, shade};
use super::perk_grid::PerkGridTextures;
use super::world_map_texture::WorldMapImage;
use crate::assets::pipboy_art::{ArtClip, ArtLibrary, FixedArt, Icon};
use crate::sim::pipboy::{InventoryCategory, PipBoyAction, PipBoySnapshot};
use crate::ui_layout::UiLayout;
use crate::ui_layout::values::Matrix;

/// The keys the hint bar names (PC labels).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HintKey {
    Enter,
    R,
    T,
    Q,
    E,
    C,
    Z,
    X,
    Ctrl,
    Alt,
    Tab,
    Mouse1,
}

impl HintKey {
    pub const ALL: [HintKey; 12] = [
        HintKey::Enter,
        HintKey::R,
        HintKey::T,
        HintKey::Q,
        HintKey::E,
        HintKey::C,
        HintKey::Z,
        HintKey::X,
        HintKey::Ctrl,
        HintKey::Alt,
        HintKey::Tab,
        HintKey::Mouse1,
    ];

    /// The key a hint label names, as the movies spell it.
    pub fn from_label(label: &str) -> Option<HintKey> {
        Self::ALL.into_iter().find(|key| key.label() == label)
    }

    pub fn label(self) -> &'static str {
        match self {
            HintKey::Enter => "Enter",
            HintKey::R => "R",
            HintKey::T => "T",
            HintKey::Q => "Q",
            HintKey::E => "E",
            HintKey::C => "C",
            HintKey::Z => "Z",
            HintKey::X => "X",
            HintKey::Ctrl => "Ctrl",
            HintKey::Alt => "Alt",
            HintKey::Tab => "Tab",
            HintKey::Mouse1 => "Mouse1",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ButtonHint {
    pub key: HintKey,
    pub label: String,
    pub enabled: bool,
    pub flashing: bool,
}

impl ButtonHint {
    pub fn new(key: HintKey, label: impl Into<String>) -> Self {
        Self {
            key,
            label: label.into(),
            enabled: true,
            flashing: false,
        }
    }

    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    #[must_use]
    pub fn flashing(mut self, flashing: bool) -> Self {
        self.flashing = flashing;
        self
    }
}

/// Navigation input after key mapping; positions and deltas are in stage pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NavInput {
    Up,
    Down,
    PrevSubTab,
    NextSubTab,
    Hint(HintKey),
    /// A favourites-wheel slot key (`0..12`), pressed after the favourite key.
    Slot(u8),
    PointerMoved(Pos2),
    Clicked(Pos2),
    /// The left button went up; positioned at the last known pointer, even outside the window.
    Released(Pos2),
    /// The Scaleform wheel delta: [`super::input::WHEEL_DELTA_PER_NOTCH`] per notch, positive
    /// away from the user.
    Scrolled(f32),
    /// The pointer moved by `delta` while `button` was held.
    Dragged {
        delta: Vec2,
        button: PointerButton,
    },
    /// The button that drove a drag went up.
    DragEnded(PointerButton),
    /// The cursor left the window or the window lost focus: no pointer position, no drag.
    PointerLeft,
}

/// The mouse button that drives a drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    Primary,
    Secondary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputResult {
    Consumed,
    /// Not handled here; the shell applies its own meaning (sub-tab or page switch, close).
    Ignored,
}

/// A change of screen, or a message, a tab asks the shell for.
#[derive(Clone, Debug, PartialEq)]
pub enum ShellRequest {
    /// Opens the full-screen perk chart.
    OpenPerkChart,
    ClosePerkChart,
    /// DATA asks for the MAP page centred on an engine-space position.
    ShowOnMap(glam::Vec3),
    /// Shows a transient message line that fades after a few seconds.
    Notify(String),
}

/// Which content the bottom info bar shows under a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BottomBar {
    Stat,
    Inventory(InventoryCategory),
    /// Date and time; with the current location on MAP.
    DateTime {
        with_location: bool,
    },
    Radio,
}

pub struct TabContext<'a> {
    /// Paints in stage pixels (1 point = 1 stage pixel), clipped to the stage.
    pub painter: &'a Painter,
    pub snapshot: &'a PipBoySnapshot<'a>,
    pub art_library: &'a ArtLibrary,
    pub layout: &'a UiLayout,
    pub art: &'a mut ArtCache,
    /// The world-map image once it is loaded and registered with the renderer.
    pub world_map_texture: Option<WorldMapImage>,
    /// The perk chart's poster textures, once loaded.
    pub perk_grid: Option<PerkGridTextures>,
    /// Full-white after the Pip-Boy tint; every greyscale value is a fraction of it.
    pub tint: Color32,
    /// Seconds since the Pip-Boy opened, for animation.
    pub seconds_open: f64,
    pub actions: &'a mut Vec<PipBoyAction>,
    pub requests: &'a mut Vec<ShellRequest>,
}

impl TabContext<'_> {
    /// The tint scaled to a greyscale fraction of full white.
    pub fn shade(&self, fraction: f32) -> Color32 {
        shade(self.tint, fraction)
    }

    /// Frame `frame` of a required clip with its registration point at `at`, drawn at
    /// `symbol_scale` times its authored size and multiplied by `colour`.
    pub fn draw_fixed(
        &mut self,
        art: FixedArt,
        frame: usize,
        at: Pos2,
        symbol_scale: f32,
        colour: Color32,
    ) {
        let sprite = self.art.fixed_scaled(
            self.painter.ctx(),
            self.art_library,
            art,
            frame,
            symbol_scale,
        );
        if let Some(sprite) = sprite {
            draw_sprite(self.painter, sprite, at, colour);
        }
    }

    /// Frame `frame` of a required clip drawn through `matrix` (stage space from the clip's
    /// registration point), rasterised at the matrix's larger axis scale.
    pub fn draw_fixed_affine(
        &mut self,
        art: FixedArt,
        frame: usize,
        matrix: &Matrix,
        colour: Color32,
    ) {
        let scale = matrix.a.hypot(matrix.b).max(matrix.c.hypot(matrix.d));
        let sprite = self
            .art
            .fixed_scaled(self.painter.ctx(), self.art_library, art, frame, scale);
        if let Some(sprite) = sprite {
            draw_sprite_affine(self.painter, sprite, scale, matrix, colour);
        }
    }

    pub fn draw_icon(&mut self, icon: Icon, at: Pos2, symbol_scale: f32, colour: Color32) {
        self.draw_fixed(FixedArt::Icon(icon), 0, at, symbol_scale, colour);
    }

    /// Like [`Self::draw_fixed`] for a data-keyed clip.
    pub fn draw_clip(
        &mut self,
        clip: &ArtClip,
        frame: usize,
        at: Pos2,
        symbol_scale: f32,
        colour: Color32,
    ) {
        let sprite = self.art.clip_scaled(
            self.painter.ctx(),
            self.art_library,
            clip,
            frame,
            symbol_scale,
        );
        if let Some(sprite) = sprite {
            draw_sprite(self.painter, sprite, at, colour);
        }
    }
}

pub trait Tab {
    fn handle_input(&mut self, input: NavInput, cx: &mut TabContext<'_>) -> InputResult;
    fn draw(&mut self, cx: &mut TabContext<'_>);
    /// The page's own hints; the shell appends its perk-chart hint after them.
    fn hints(&self, _snapshot: &PipBoySnapshot<'_>) -> Vec<ButtonHint> {
        Vec::new()
    }
    fn bottom_bar(&self) -> BottomBar;

    /// Paints over the shell chrome, under the hint bar: the dimmed modal layer and its panel.
    fn draw_overlay(&mut self, _cx: &mut TabContext<'_>) {}

    /// The healing of the selected item as a fraction of the maximum health, for the bottom
    /// bar's health ghost; pages without healing items keep `0`.
    fn selected_hp_gain(&self, _snapshot: &PipBoySnapshot<'_>) -> f32 {
        0.0
    }

    /// Whether the shell offers its perk-chart hint and key on this page.
    fn can_lower_pipboy(&self) -> bool {
        true
    }

    /// Called every time the Pip-Boy opens; tabs drop transient state such as open prompts.
    fn on_open(&mut self) {}

    /// While `true` the shell offers all input, page switches and chrome clicks included, to this
    /// tab first and swallows whatever it ignores of those.
    fn is_modal(&self) -> bool {
        false
    }

    /// Moves the list selection to `row`; for the `OPENFO4_PIPBOY_SELECT` capture hook. Tabs
    /// without a list ignore it.
    fn select_row(&mut self, _row: usize) {}
}
