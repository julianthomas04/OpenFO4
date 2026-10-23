//! The authoring stages of the Pip-Boy's two movies and their uniform fit onto the window.

use egui::{Pos2, Rect, Vec2};

use super::nav::Screen;
use crate::ui_layout::{ModuleKeys, StageKey, UiLayout, UiLayoutError};

const SHELL_MOVIE: &str = "PipboyMenu";
const PERK_CHART_MOVIE: &str = "LevelUpMenu";

static SHELL: StageKey = StageKey::of(SHELL_MOVIE);
static PERK_CHART: StageKey = StageKey::of(PERK_CHART_MOVIE);

fn validate(layout: &UiLayout) -> Result<(), UiLayoutError> {
    Stages::resolve(layout).map(drop)
}

/// The stage keys of both movies, resolved at boot.
pub static LAYOUT_KEYS: ModuleKeys = ModuleKeys {
    keys: &[&SHELL, &PERK_CHART],
    resolve: validate,
};

/// Share of the window the shell stage may span, a stand-in until the 3D Pip-Boy screen sizes it.
pub const SHELL_WINDOW_FRACTION: f32 = 0.4;
/// The perk chart is a full-screen menu.
pub const PERK_CHART_WINDOW_FRACTION: f32 = 1.0;

/// The size of the shell's movie and of the perk chart's, as their documents state them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stages {
    shell: Vec2,
    perk_chart: Vec2,
}

fn size_of(layout: &UiLayout, key: &StageKey) -> Result<Vec2, UiLayoutError> {
    let [xmin, ymin, xmax, ymax] = layout.get(key)?.rect;
    if xmin != 0.0 || ymin != 0.0 || !(xmax > 0.0 && ymax > 0.0) {
        return Err(UiLayoutError::unresolved(
            key.movie,
            "stage",
            format!(
                "the stage [{xmin}, {ymin}, {xmax}, {ymax}] does not start at the origin or is empty"
            ),
        ));
    }
    Ok(Vec2::new(xmax, ymax))
}

/// The size of the perk chart's stage, the one place it is read from.
pub fn perk_chart_size(layout: &UiLayout) -> Result<Vec2, UiLayoutError> {
    size_of(layout, &PERK_CHART)
}

impl Stages {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        Ok(Self {
            shell: size_of(layout, &SHELL)?,
            perk_chart: perk_chart_size(layout)?,
        })
    }

    /// The stage of the movie `screen` shows and the share of the window it spans.
    pub fn of(&self, screen: Screen) -> (Vec2, f32) {
        match screen {
            Screen::Shell(_) => (self.shell, SHELL_WINDOW_FRACTION),
            Screen::PerkChart => (self.perk_chart, PERK_CHART_WINDOW_FRACTION),
        }
    }

    pub fn shell(&self) -> Vec2 {
        self.shell
    }
}

/// A stage scaled uniformly to fit a centred fraction of the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageFit {
    /// Window pixels per stage pixel; becomes egui's `pixels_per_point`.
    pub scale: f32,
    /// Top-left of the stage in window pixels.
    pub origin_px: Pos2,
}

impl StageFit {
    pub fn new(stage: Vec2, window_px: Vec2, window_fraction: f32) -> Self {
        let scale = (window_px.x / stage.x).min(window_px.y / stage.y) * window_fraction;
        let drawn = stage * scale;
        Self {
            scale,
            origin_px: Pos2::new((window_px.x - drawn.x) / 2.0, (window_px.y - drawn.y) / 2.0),
        }
    }

    /// The stage's top-left in egui points, where the painter's stage space starts.
    pub fn origin_points(&self) -> Vec2 {
        self.origin_px.to_vec2() / self.scale
    }

    /// A window-pixel position (cursor) in stage pixels.
    pub fn window_to_stage(&self, window_px: Pos2) -> Pos2 {
        Pos2::ZERO + (window_px - self.origin_px) / self.scale
    }

    /// The window rectangle in egui points, for the context's screen rect.
    pub fn screen_rect_points(&self, window_px: Vec2) -> Rect {
        Rect::from_min_size(Pos2::ZERO, window_px / self.scale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::pipboy::nav::TabId;

    /// A stage and window of arbitrary sizes: a 4:3 stage in a 16:9 window.
    const STAGE: Vec2 = Vec2::new(400.0, 300.0);

    fn fit(stage: Vec2, w: f32, h: f32) -> StageFit {
        StageFit::new(stage, Vec2::new(w, h), 1.0)
    }

    #[test]
    fn a_stage_spans_its_window_fraction_and_stays_centred() {
        let fit = StageFit::new(STAGE, Vec2::new(1920.0, 1080.0), 0.5);
        assert!((STAGE.y * fit.scale - 0.5 * 1080.0).abs() < 1e-3);
        assert!((fit.origin_px.x * 2.0 + STAGE.x * fit.scale - 1920.0).abs() < 1e-3);
        assert!((fit.origin_px.y * 2.0 + STAGE.y * fit.scale - 1080.0).abs() < 1e-3);
    }

    #[test]
    fn a_wide_window_is_pillarboxed_and_centred() {
        let fit = fit(STAGE, 1920.0, 1080.0);
        assert!((fit.scale - 3.6).abs() < 1e-5);
        assert!((fit.origin_px.x - 240.0).abs() < 1e-3);
        assert!(fit.origin_px.y.abs() < 1e-3);
    }

    #[test]
    fn a_tall_window_is_letterboxed_by_the_width() {
        let fit = fit(STAGE, 400.0, 500.0);
        assert!((fit.scale - 1.0).abs() < 1e-6);
        assert!((fit.origin_px.y - 100.0).abs() < 1e-4);
        assert!(fit.origin_px.x.abs() < 1e-4);
    }

    #[test]
    fn a_stage_of_the_windows_proportions_fills_it_exactly() {
        let fit = fit(Vec2::new(320.0, 180.0), 1920.0, 1080.0);
        assert!((fit.scale - 6.0).abs() < 1e-6);
        assert!(fit.origin_px.x.abs() < 1e-3 && fit.origin_px.y.abs() < 1e-3);
    }

    #[test]
    fn window_positions_map_back_to_stage_pixels() {
        let fit = fit(STAGE, 1920.0, 1080.0);
        let stage = fit.window_to_stage(Pos2::new(240.0 + 3.6 * 100.0, 3.6 * 50.0));
        assert!((stage.x - 100.0).abs() < 1e-2 && (stage.y - 50.0).abs() < 1e-2);
    }

    #[test]
    fn the_screen_rect_in_points_spans_the_window_at_the_stage_scale() {
        let fit = fit(STAGE, 1920.0, 1080.0);
        let rect = fit.screen_rect_points(Vec2::new(1920.0, 1080.0));
        assert!((rect.width() - 1920.0 / 3.6).abs() < 1e-2);
        assert!((fit.origin_points().x - 240.0 / 3.6).abs() < 1e-2);
    }

    #[test]
    fn each_screen_takes_the_stage_of_its_own_movie() {
        let layout = crate::rendering::pipboy::test_layout::pipboy_layout();
        let stages = Stages::resolve(&layout).unwrap();
        let shell = layout.get(&SHELL).unwrap().rect;
        let chart = layout.get(&PERK_CHART).unwrap().rect;
        assert_eq!(
            stages.of(Screen::Shell(TabId::Map)),
            (Vec2::new(shell[2], shell[3]), SHELL_WINDOW_FRACTION)
        );
        assert_eq!(
            stages.of(Screen::PerkChart),
            (Vec2::new(chart[2], chart[3]), PERK_CHART_WINDOW_FRACTION)
        );
    }

    #[test]
    fn a_stage_that_does_not_start_at_the_origin_is_refused() {
        let layout = crate::ui_layout::test_support::layout_with_movie(
            SHELL_MOVIE,
            &crate::ui_layout::test_support::movie_json(
                SHELL_MOVIE,
                &serde_json::json!({"stage": [10, 0, 100, 100]}),
            ),
        );
        let error = size_of(&layout, &SHELL).unwrap_err();
        assert!(error.to_string().contains("origin"), "{error}");
    }
}
