//! Drives a single page in tests: real model, real egui context and fonts, a real art library
//! over a synthetic one-shape manifest. Inputs and draws go through the page's `Tab` impl.

use egui::{Id, LayerId, Order, RawInput};
use tempfile::TempDir;

use crate::assets::layout::{INTERFACE_DIR, PIPBOY_ART_MANIFEST_FILE, PIPBOY_DIR};
use crate::assets::pipboy_art::{ArtLibrary, FixedArt};
use crate::rendering::pipboy::art::ArtCache;
use crate::rendering::pipboy::fonts::PipBoyFonts;
use crate::rendering::pipboy::tab::{InputResult, NavInput, ShellRequest, Tab, TabContext};
use crate::rendering::pipboy::world_map_texture::WorldMapImage;
use crate::sim::pipboy::{PipBoyAction, PipBoyModel, PipBoySnapshot, PlayerVitals};

const TINY_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 10 10"><rect width="10" height="10" fill="white"/></svg>"#;

/// The source movie of a Vault Boy clip with two frames that stops on the second at 10 fps.
pub const TWO_FRAME_MOVIE: &str = "Test/TwoFrames.swf";

pub fn vitals() -> PlayerVitals {
    PlayerVitals {
        hp: 80.0,
        hp_max: 100.0,
        ap: 0.0,
        ap_max: 0.0,
        position: None,
        heading_yaw: 0.0,
    }
}

fn art_library(dir: &TempDir) -> ArtLibrary {
    let root = dir.path().join(INTERFACE_DIR).join(PIPBOY_DIR);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("frame.svg"), TINY_SVG).unwrap();
    let mut clips: Vec<_> = FixedArt::all()
        .map(|art| {
            serde_json::json!({
                "id": art.clip_id(), "source": "Test.swf", "colour": "mask", "fps": 30.0,
                "script": [], "bounds": [-5.0, -5.0, 5.0, 5.0], "frames": ["frame.svg"],
            })
        })
        .collect();
    clips.push(serde_json::json!({
        "id": format!("vaultboy/{TWO_FRAME_MOVIE}"), "source": TWO_FRAME_MOVIE, "colour": "mask",
        "fps": 10.0, "script": [{"frame": 2, "action": "stop"}],
        "bounds": [-5.0, -5.0, 5.0, 5.0], "frames": ["frame.svg", "frame.svg"],
    }));
    let manifest =
        serde_json::json!({ "format": crate::assets::pipboy_art::MANIFEST_FORMAT, "clips": clips });
    std::fs::write(root.join(PIPBOY_ART_MANIFEST_FILE), manifest.to_string()).unwrap();
    ArtLibrary::load(dir.path()).unwrap()
}

pub struct Harness {
    ctx: egui::Context,
    library: ArtLibrary,
    layout: std::sync::Arc<crate::ui_layout::UiLayout>,
    art: ArtCache,
    _dir: TempDir,
    actions: Vec<PipBoyAction>,
    requests: Vec<ShellRequest>,
    pub vitals: PlayerVitals,
    pub seconds_open: f64,
    pub world_map: Option<WorldMapImage>,
}

impl Harness {
    pub fn new() -> Self {
        let defaults = egui::FontDefinitions::default();
        let bytes = defaults.font_data["Ubuntu-Light"].font.to_vec();
        let ctx = egui::Context::default();
        ctx.set_fonts(PipBoyFonts::new(bytes.clone(), bytes).definitions());
        let dir = tempfile::tempdir().unwrap();
        Self {
            ctx,
            library: art_library(&dir),
            layout: crate::rendering::pipboy::test_layout::pipboy_layout(),
            art: ArtCache::default(),
            _dir: dir,
            actions: Vec::new(),
            requests: Vec::new(),
            vitals: vitals(),
            seconds_open: 0.0,
            world_map: None,
        }
    }

    fn layer() -> LayerId {
        LayerId::new(Order::Foreground, Id::new("page-test"))
    }

    pub fn with_context<R>(
        &mut self,
        model: &PipBoyModel,
        run: impl FnOnce(&mut TabContext<'_>) -> R,
    ) -> R {
        let painter = self.ctx.layer_painter(Self::layer());
        let snapshot = PipBoySnapshot {
            model,
            vitals: self.vitals,
        };
        let mut cx = TabContext {
            painter: &painter,
            snapshot: &snapshot,
            art_library: &self.library,
            layout: &self.layout,
            art: &mut self.art,
            world_map_texture: self.world_map,
            perk_grid: None,
            tint: egui::Color32::WHITE,
            seconds_open: self.seconds_open,
            actions: &mut self.actions,
            requests: &mut self.requests,
        };
        run(&mut cx)
    }

    pub fn send(&mut self, tab: &mut dyn Tab, model: &PipBoyModel, input: NavInput) -> InputResult {
        self.with_context(model, |cx| tab.handle_input(input, cx))
    }

    /// Paints one frame of `tab`; the shapes go nowhere, but every layout and state update of a
    /// real draw happens.
    pub fn draw(&mut self, tab: &mut dyn Tab, model: &PipBoyModel) {
        let ctx = self.ctx.clone();
        let mut output = ctx.run_ui(RawInput::default(), |_| {
            self.with_context(model, |cx| tab.draw(cx));
        });
        output.textures_delta.clear();
    }

    pub fn take_actions(&mut self) -> Vec<PipBoyAction> {
        std::mem::take(&mut self.actions)
    }

    pub fn take_requests(&mut self) -> Vec<ShellRequest> {
        std::mem::take(&mut self.requests)
    }
}
