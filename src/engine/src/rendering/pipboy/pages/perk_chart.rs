//! The full-screen perk chart (1280 x 720 stage): the Vault-Tec poster with the vault boy cards on
//! it, the SPECIAL headers, the info panel and the level box, laid out from the movie's own
//! placements. STAT/PERKS opens it with `ShellRequest::OpenPerkChart`; Tab closes it.

mod filter;
#[cfg(test)]
mod fixture;
mod frame;
mod grid;
mod hints;
mod info;
mod layout;
mod poster;
mod requirements;
mod stars;
mod state;
mod xp;

use std::sync::Arc;

use egui::Vec2;

#[cfg(test)]
pub use fixture::{TRANSLATIONS as LEVEL_UP_TRANSLATIONS, level_up_menu_movie};
pub use layout::{LAYOUT_KEYS, MOVIE};

use self::info::{PanelContent, PanelHistory, PlanInput};
use self::layout::ChartLayout;
use self::state::{ChartState, InputEnv};
use crate::rendering::pipboy::tab::{
    BottomBar, ButtonHint, InputResult, NavInput, PointerButton, Tab, TabContext,
};
use crate::sim::pipboy::{PipBoyModel, PipBoySnapshot};
use crate::ui_layout::{UiLayout, UiLayoutError};

pub struct PerkChartPage {
    layout: Arc<ChartLayout>,
    state: ChartState,
    panel: PanelHistory,
}

impl PerkChartPage {
    /// Reads the chart's layout before the first frame, so a layout the chart cannot use fails
    /// the load instead of the first draw.
    pub fn load(document: &UiLayout) -> Result<Self, UiLayoutError> {
        let layout = ChartLayout::load(document)?;
        Ok(Self {
            state: ChartState::new(&layout.grid),
            layout: Arc::new(layout),
            panel: PanelHistory::default(),
        })
    }

    /// Drops what must not outlive the chart being hidden: a drag whose end was never seen and the
    /// movie frame the grid last advanced at.
    pub fn resume(&mut self) {
        self.state.resume();
    }

    /// The pointer moved by `delta` with `button` held; only the right button pans the grid.
    pub fn dragged(&mut self, button: PointerButton, delta: Vec2, cx: &mut TabContext<'_>) {
        let Self { layout, state, .. } = self;
        state.dragged(button, delta, &input_env(layout, cx));
    }
}

fn input_env<'a>(layout: &'a ChartLayout, cx: &'a mut TabContext<'_>) -> InputEnv<'a> {
    InputEnv {
        constants: &layout.grid,
        cells: &layout.cells,
        error_seconds: layout.info.error_seconds,
        model: cx.snapshot.model,
        now: cx.seconds_open,
        actions: &mut *cx.actions,
        requests: &mut *cx.requests,
    }
}

fn draw_info_panel(
    cx: &mut TabContext<'_>,
    layout: &ChartLayout,
    state: &ChartState,
    panel: &mut PanelHistory,
    model: &PipBoyModel,
) {
    let Some(selection) = state.selection(model) else {
        return;
    };
    let rules = &layout.info.rules;
    let rank = state.viewed_rank(&selection);
    let Some(rank_info) = selection.perk.ranks.get(usize::from(rank) - 1) else {
        return;
    };
    let Some(selected_stage_y) = state.selected_stage_y(&layout.cells, rules.entry_y_offset) else {
        return;
    };
    let (held, available_slot) = state.held_ranks(&selection);
    let stars = info::panel_stars(selection.max_rank, held, available_slot);
    let plan = info::plan(
        rules,
        &PlanInput {
            measure: info::measure(cx, &layout.info, &rank_info.description),
            star_count: stars.len(),
            star_width: layout.rank_star_size.x * rules.star_scale,
            selected_stage_y,
        },
        panel.before((selection.cell, rank), rules),
    );
    panel.record(plan.memory);

    let confirming = state.is_confirming();
    let error_shown = state.error_visible(cx.seconds_open);
    let requirements = if !confirming && !error_shown && (!selection.maxed() || state.is_browsing())
    {
        requirements::requirement_line(
            &layout.xp.level_word,
            &layout.headers[selection.cell.column].label,
            rank_info,
            selection.perk.chart,
        )
    } else {
        String::new()
    };
    let notice = if confirming {
        Some(layout.info.confirm_prompt.as_str())
    } else if error_shown {
        Some(layout.info.requirements_not_met.as_str())
    } else {
        None
    };
    let name = selection.perk.name.to_uppercase();
    info::draw(
        cx,
        layout,
        &plan,
        &PanelContent {
            name: &name,
            description: &rank_info.description,
            requirements: &requirements,
            notice,
            stars: &stars,
        },
    );
}

impl Tab for PerkChartPage {
    fn handle_input(&mut self, input: NavInput, cx: &mut TabContext<'_>) -> InputResult {
        let Self { layout, state, .. } = self;
        state.handle_input(input, &mut input_env(layout, cx))
    }

    fn draw(&mut self, cx: &mut TabContext<'_>) {
        let model = cx.snapshot.model;
        let layout = Arc::clone(&self.layout);
        let now = cx.seconds_open;
        self.state.advance(now, &input_env(&layout, cx));
        let stage = layout.stage;
        let view = self.state.view;
        if let Some(textures) = cx.perk_grid {
            poster::draw_poster(cx.painter, &textures, &layout.grid, &view, stage);
        }
        grid::draw_cards(cx, &layout, &self.state, model, stage);
        grid::draw_headers(cx, &layout, &self.state, model);
        if let Some(textures) = cx.perk_grid {
            poster::draw_overlay(
                cx.painter,
                &textures,
                &layout.grid,
                &view,
                stage,
                layout.overlay.alpha,
            );
        }
        draw_info_panel(cx, &layout, &self.state, &mut self.panel, model);
        xp::draw(cx, &layout, model);
        let bar = hints::hints(&layout.hints, &self.state, model);
        hints::draw_bar(cx, &layout, &bar);
    }

    fn hints(&self, snapshot: &PipBoySnapshot<'_>) -> Vec<ButtonHint> {
        hints::hints(&self.layout.hints, &self.state, snapshot.model)
    }

    fn bottom_bar(&self) -> BottomBar {
        BottomBar::Stat
    }

    fn on_open(&mut self) {
        self.state = ChartState::new(&self.layout.grid);
        self.panel = PanelHistory::default();
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "exact literals")]
mod tests {
    use egui::{Pos2, Vec2};

    use super::*;
    use crate::rendering::pipboy::pages::perk_chart::state::tests::model;
    use crate::rendering::pipboy::pages::perk_chart_layout::{Cell, GridView};
    use crate::rendering::pipboy::pages::test_harness::Harness;
    use crate::rendering::pipboy::tab::{HintKey, ShellRequest};
    use crate::sim::pipboy::{PerkId, PipBoyAction};

    fn chart_layout() -> ChartLayout {
        ChartLayout::load(&crate::rendering::pipboy::test_layout::pipboy_layout()).unwrap()
    }

    fn chart_page() -> PerkChartPage {
        PerkChartPage::load(&crate::rendering::pipboy::test_layout::pipboy_layout()).unwrap()
    }

    fn card_centre(cell: Cell) -> Pos2 {
        let layout = chart_layout();
        let view = GridView::initial(&layout.grid);
        view.to_stage(layout.cell(cell).border.center())
    }

    const IRON_FIST: Cell = Cell { column: 0, row: 1 };

    #[test]
    fn the_chart_draws_empty_hovered_confirming_and_without_a_catalog() {
        let mut harness = Harness::new();
        let model = model();
        let mut page = chart_page();
        harness.draw(&mut page, &model);
        harness.send(
            &mut page,
            &model,
            NavInput::PointerMoved(card_centre(IRON_FIST)),
        );
        harness.draw(&mut page, &model);
        harness.send(&mut page, &model, NavInput::Hint(HintKey::Enter));
        harness.draw(&mut page, &model);
        harness.draw(&mut chart_page(), &PipBoyModel::new(Arc::default()));
    }

    #[test]
    fn choosing_a_perk_and_accepting_asks_the_model_once() {
        let mut harness = Harness::new();
        let model = model();
        let mut page = chart_page();

        harness.send(&mut page, &model, NavInput::Clicked(card_centre(IRON_FIST)));
        assert_eq!(harness.take_actions(), []);
        harness.send(&mut page, &model, NavInput::Hint(HintKey::Enter));

        assert_eq!(
            harness.take_actions(),
            vec![PipBoyAction::RankUpPerk(PerkId("iron-fist".to_owned()))]
        );
    }

    #[test]
    fn tab_closes_the_chart() {
        let mut harness = Harness::new();
        let model = model();
        let mut page = chart_page();

        harness.send(&mut page, &model, NavInput::Hint(HintKey::Tab));

        assert_eq!(harness.take_requests(), vec![ShellRequest::ClosePerkChart]);
    }

    #[test]
    fn the_hint_bar_is_valid_before_any_draw() {
        let model = model();
        let page = chart_page();
        let snapshot = PipBoySnapshot {
            model: &model,
            clock: crate::sim::WorldClock::fixture().date(),
            vitals: crate::rendering::pipboy::pages::test_harness::vitals(),
        };

        let labels: Vec<_> = page
            .hints(&snapshot)
            .into_iter()
            .map(|hint| hint.label)
            .collect();
        assert_eq!(
            labels,
            [
                "Hint choose (1)",
                "Hint close",
                "Hint previous",
                "Hint next"
            ]
        );
    }

    #[test]
    fn a_layout_without_the_chart_fails_the_load_instead_of_panicking() {
        let layout = crate::ui_layout::test_support::layout_with_movie(
            "Unrelated",
            &crate::ui_layout::test_support::empty_movie(),
        );

        assert!(PerkChartPage::load(&layout).is_err());
    }

    #[test]
    fn reopening_puts_the_grid_back_in_its_opening_view() {
        let mut harness = Harness::new();
        let model = model();
        let mut page = chart_page();
        harness.with_context(&model, |cx| {
            page.dragged(PointerButton::Secondary, Vec2::new(0.0, -300.0), cx);
        });
        let panned = page.state.view;
        assert_ne!(panned.y, 0.0);

        page.on_open();

        assert_eq!(page.state.view.y, 0.0);
    }
}
