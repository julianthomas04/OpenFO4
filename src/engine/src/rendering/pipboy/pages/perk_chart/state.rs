//! What the perk chart remembers between frames and how input changes it: the grid view, the
//! hovered cell, the rank the info panel shows and the confirmation of a perk about to be taken.

use egui::{Pos2, Rect, Vec2};

use super::layout::CellLayout;
use crate::rendering::pipboy::input::WHEEL_DELTA_PER_NOTCH;
use crate::rendering::pipboy::pages::perk_chart_layout::{
    Cell, GridConstants, GridView, can_view_next, can_view_previous, initial_offset, perk_in_cell,
    shown_rank,
};
use crate::rendering::pipboy::tab::{HintKey, InputResult, NavInput, PointerButton, ShellRequest};
use crate::sim::pipboy::perks::PerkDefinition;
use crate::sim::pipboy::{PipBoyAction, PipBoyModel};

/// The perk under the pointer with what the chart asks about it.
#[derive(Clone, Copy)]
pub struct Selection<'a> {
    pub cell: Cell,
    pub perk: &'a PerkDefinition,
    pub owned: u8,
    pub max_rank: u8,
    pub unspent_points: u32,
    /// The next rank meets its level and SPECIAL requirements, whatever the points.
    pub available: bool,
}

impl<'a> Selection<'a> {
    pub fn of(model: &'a PipBoyModel, cell: Cell) -> Option<Self> {
        let perk = perk_in_cell(&model.catalog.perks, model.character.sex, cell)?;
        Some(Self {
            cell,
            perk,
            owned: model.perks.rank(&perk.id),
            max_rank: u8::try_from(perk.ranks.len()).unwrap_or(u8::MAX),
            unspent_points: model.character.progression.unspent_perk_points,
            available: model.perk_next_rank_requirements_met(&perk.id),
        })
    }

    pub fn maxed(&self) -> bool {
        self.owned >= self.max_rank
    }

    /// The perk can be taken now: its next rank meets its requirements and there is a point to
    /// spend, whichever rank the panel shows.
    pub fn can_acquire(&self) -> bool {
        self.available && self.unspent_points > 0 && !self.maxed()
    }
}

/// The pan a drag with `button` makes: the right button pans by the movement times the menu's
/// drag speed, every other button does not pan.
pub fn pan_of(button: PointerButton, delta: Vec2, speed: f32) -> Option<Vec2> {
    (button == PointerButton::Secondary).then_some(delta * speed)
}

/// What a chart input needs from the rest of the game.
pub struct InputEnv<'a> {
    pub constants: &'a GridConstants,
    pub cells: &'a [CellLayout],
    pub error_seconds: f32,
    pub model: &'a PipBoyModel,
    pub now: f64,
    pub actions: &'a mut Vec<PipBoyAction>,
    pub requests: &'a mut Vec<ShellRequest>,
}

pub struct ChartState {
    pub view: GridView,
    selected: Option<Cell>,
    /// The rank step the player took with the rank keys; `None` shows the rank the selection starts
    /// on.
    browsed: Option<i8>,
    confirming: bool,
    error_until: Option<f64>,
    pointer: Option<Pos2>,
    /// The right button is held and has moved the pointer since it went down.
    dragging: bool,
    /// The movie frame the grid last advanced at.
    last_frame: Option<i64>,
}

impl ChartState {
    pub fn new(constants: &GridConstants) -> Self {
        Self {
            view: GridView::initial(constants),
            selected: None,
            browsed: None,
            confirming: false,
            error_until: None,
            pointer: None,
            dragging: false,
            last_frame: None,
        }
    }

    pub fn selection<'a>(&self, model: &'a PipBoyModel) -> Option<Selection<'a>> {
        self.selected.and_then(|cell| Selection::of(model, cell))
    }

    pub fn is_confirming(&self) -> bool {
        self.confirming
    }

    pub fn is_browsing(&self) -> bool {
        self.browsed.is_some()
    }

    pub fn error_visible(&self, now: f64) -> bool {
        self.error_until.is_some_and(|until| now < until)
    }

    /// How far from the owned rank the info panel looks.
    pub fn offset(&self, selection: &Selection<'_>) -> i8 {
        self.browsed.unwrap_or_else(|| {
            initial_offset(
                selection.owned,
                selection.max_rank,
                selection.unspent_points,
            )
        })
    }

    pub fn viewed_rank(&self, selection: &Selection<'_>) -> u8 {
        shown_rank(selection.owned, self.offset(selection), selection.max_rank)
    }

    /// The rank count the info panel's stars show as held, and whether the next rank may blink.
    pub fn held_ranks(&self, selection: &Selection<'_>) -> (u8, bool) {
        match self.browsed {
            Some(offset) => (
                u8::try_from(
                    (i16::from(selection.owned) + i16::from(offset))
                        .clamp(0, i16::from(selection.max_rank)),
                )
                .unwrap_or(0),
                false,
            ),
            None => (
                selection.owned,
                selection.available && selection.unspent_points > 0,
            ),
        }
    }

    pub fn can_view_previous(&self, selection: &Selection<'_>) -> bool {
        can_view_previous(selection.owned, self.offset(selection), selection.max_rank)
    }

    pub fn can_view_next(&self, selection: &Selection<'_>) -> bool {
        can_view_next(selection.owned, self.offset(selection), selection.max_rank)
    }

    /// The choose hint is lit: the perk can be taken and the panel is not showing some other rank
    /// than the next.
    pub fn accept_hint_enabled(&self, selection: &Selection<'_>) -> bool {
        selection.can_acquire() && (!self.is_browsing() || self.offset(selection) == 1)
    }

    pub fn cell_at(&self, cells: &[CellLayout], at: Pos2) -> Option<Cell> {
        Cell::ALL.into_iter().find(|cell| {
            let border = cells[cell.index()].border;
            Rect::from_min_max(
                self.view.to_stage(border.min),
                self.view.to_stage(border.max),
            )
            .contains(at)
        })
    }

    fn select(&mut self, cell: Option<Cell>) {
        if cell != self.selected {
            self.selected = cell;
            self.browsed = None;
            if cell.is_none() {
                self.error_until = None;
            }
        }
    }

    /// Selects what is under the pointer now; the grid has no input while confirming.
    fn reselect(&mut self, cells: &[CellLayout]) {
        if let (false, Some(at)) = (self.confirming, self.pointer) {
            self.select(self.cell_at(cells, at));
        }
    }

    fn step_rank(&mut self, model: &PipBoyModel, step: i8) {
        let Some(selection) = self.selection(model) else {
            return;
        };
        let allowed = if step < 0 {
            self.can_view_previous(&selection)
        } else {
            self.can_view_next(&selection)
        };
        if allowed {
            self.browsed = Some(self.offset(&selection) + step);
        }
    }

    /// The menu's rule for taking the selected perk with the mouse or the accept key: a perk
    /// whose next rank is not available shows the refusal (unless it is maxed), an available one
    /// asks to confirm.
    fn try_to_acquire(&mut self, env: &InputEnv<'_>) {
        let Some(selection) = self.selection(env.model) else {
            return;
        };
        if selection.unspent_points == 0 || self.confirming {
            return;
        }
        if selection.available {
            self.confirming = true;
        } else if !selection.maxed() && !self.error_visible(env.now) {
            self.error_until = Some(env.now + f64::from(env.error_seconds));
        }
    }

    /// The accept key: takes the perk once the confirmation is open, otherwise asks for it.
    fn accept(&mut self, env: &mut InputEnv<'_>) {
        if !self.confirming {
            self.try_to_acquire(env);
            return;
        }
        let Some(selection) = self.selection(env.model) else {
            return;
        };
        if selection.can_acquire() {
            env.actions
                .push(PipBoyAction::RankUpPerk(selection.perk.id.clone()));
            self.confirming = false;
            self.browsed = None;
        }
    }

    /// A drag of the pointer with `button` held. Only the right button pans the grid, by the
    /// menu's drag speed; the grid takes no input while confirming.
    pub fn dragged(&mut self, button: PointerButton, delta: Vec2, env: &InputEnv<'_>) {
        if self.confirming {
            return;
        }
        if let Some(pan) = pan_of(button, delta, env.constants.drag_speed) {
            self.dragging |= delta != Vec2::ZERO;
            self.view.move_by(env.constants, pan);
            self.reselect(env.cells);
        }
    }

    /// The wheel zooms about the pointer, a constant step per notch; the grid takes no input
    /// while confirming.
    fn zoom(&mut self, scrolled: f32, env: &InputEnv<'_>) {
        let (false, Some(pointer)) = (self.confirming, self.pointer) else {
            return;
        };
        let notches = scrolled / WHEEL_DELTA_PER_NOTCH;
        let delta = notches * env.constants.zoom_per_notch();
        if self.view.zoom(env.constants, delta, pointer) {
            self.reselect(env.cells);
        }
    }

    /// Forgets the drag and the frame clock, as when the chart is shown again.
    pub fn resume(&mut self) {
        self.dragging = false;
        self.last_frame = None;
    }

    /// Applies one movie frame's scroll when at least one movie frame is due at `now` seconds:
    /// while the pointer rests in an edge zone and no drag or confirmation is active, the grid
    /// moves toward the centre by the zone's speed. Slow frames are not caught up.
    pub fn advance(&mut self, now: f64, env: &InputEnv<'_>) {
        let frame = (now * f64::from(env.constants.frame_rate)).floor() as i64;
        let due = self
            .last_frame
            .replace(frame)
            .map_or(0, |last| frame - last);
        let (false, false, Some(pointer)) = (self.confirming, self.dragging, self.pointer) else {
            return;
        };
        let step = env.constants.hover.step(pointer);
        if due > 0 && step != Vec2::ZERO {
            self.view.move_by(env.constants, step);
            self.reselect(env.cells);
        }
    }

    pub fn handle_input(&mut self, input: NavInput, env: &mut InputEnv<'_>) -> InputResult {
        match input {
            NavInput::PointerMoved(at) => {
                self.pointer = Some(at);
                self.reselect(env.cells);
            }
            NavInput::Clicked(at) => {
                if !self.confirming {
                    self.select(self.cell_at(env.cells, at));
                    self.try_to_acquire(env);
                }
            }
            NavInput::Scrolled(scrolled) => self.zoom(scrolled, env),
            NavInput::DragEnded(PointerButton::Secondary) => self.dragging = false,
            NavInput::PointerLeft => {
                self.pointer = None;
                self.resume();
            }
            NavInput::Hint(HintKey::Enter) => self.accept(env),
            NavInput::Hint(HintKey::Ctrl) => self.step_rank(env.model, -1),
            NavInput::Hint(HintKey::Alt) => self.step_rank(env.model, 1),
            NavInput::Hint(HintKey::Tab) => {
                if self.confirming {
                    self.confirming = false;
                } else {
                    env.requests.push(ShellRequest::ClosePerkChart);
                }
            }
            NavInput::Up
            | NavInput::Down
            | NavInput::PrevSubTab
            | NavInput::NextSubTab
            | NavInput::DragEnded(_) => {}
            _ => return InputResult::Ignored,
        }
        InputResult::Consumed
    }

    /// The stage y the info panel's side of the screen follows: the selected card's, moved down by
    /// `grid_offset` grid units.
    pub fn selected_stage_y(&self, cells: &[CellLayout], grid_offset: f32) -> Option<f32> {
        self.selected.map(|cell| {
            self.view.y + (cells[cell.index()].origin.y + grid_offset) * self.view.scale
        })
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "exact literals")]
pub(super) mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::rendering::pipboy::pages::perk_chart_layout::{COLUMNS, HoverScroll, ROWS};
    use crate::sim::pipboy::perks::{PerkChartSlot, PerkRank};
    use crate::sim::pipboy::{PerkId, PipBoyCatalog, Special};

    pub fn constants() -> GridConstants {
        GridConstants {
            min_scale: 0.5,
            max_scale: 1.0,
            close_enough: 0.001,
            frame_rate: 30.0,
            hover: HoverScroll {
                x_min: 30.0,
                x_max: 1200.0,
                y_min: 12.0,
                y_max: 700.0,
                x_speed: 20.0,
                y_speed: 10.0,
            },
            max_x: 0.0,
            max_y: 0.0,
            texture_width: 2560.0,
            top_height: 1500.0,
            content_height: 3000.0,
            border_width: 1280.0,
            border_height: 720.0,
            drag_speed: 1.0,
        }
    }

    /// Cards 240 x 215 grid units apart with 210 x 160 hit boxes.
    pub fn cells() -> Vec<CellLayout> {
        Cell::ALL
            .iter()
            .map(|cell| {
                let origin = Pos2::new(cell.column as f32 * 240.0, cell.row as f32 * 215.0);
                CellLayout {
                    origin,
                    scale: Vec2::splat(0.5),
                    border: Rect::from_min_size(origin, Vec2::new(210.0, 160.0)),
                }
            })
            .collect()
    }

    fn perk(id: &str, special: Special, row: u8, ranks: usize, level: u32) -> PerkDefinition {
        PerkDefinition {
            id: PerkId(id.to_owned()),
            name: id.to_owned(),
            ranks: (1..=ranks)
                .map(|number| PerkRank {
                    form_key: format!("{id}{number}"),
                    description: format!("{id} rank {number}"),
                    required_level: level,
                })
                .collect(),
            vault_boy_swf: None,
            chart: Some(PerkChartSlot {
                special,
                required_special: row,
            }),
            sex: None,
        }
    }

    /// Iron Fist (STR 1, three ranks) in (0, 1), Big Leagues (STR 2, one rank, level 50) in
    /// (0, 2); one unspent point.
    pub fn model() -> PipBoyModel {
        let catalog = PipBoyCatalog {
            perks: vec![
                perk("iron-fist", Special::Strength, 1, 3, 1),
                perk("big-leagues", Special::Strength, 2, 1, 50),
            ],
            ..PipBoyCatalog::default()
        };
        let mut model = PipBoyModel::new(Arc::new(catalog));
        model.character.progression.unspent_perk_points = 1;
        model
    }

    fn iron_fist() -> PerkId {
        PerkId("iron-fist".to_owned())
    }

    const IRON_FIST: Cell = Cell { column: 0, row: 1 };
    const BIG_LEAGUES: Cell = Cell { column: 0, row: 2 };

    struct Rig {
        state: ChartState,
        cells: Vec<CellLayout>,
        constants: GridConstants,
        actions: Vec<PipBoyAction>,
        requests: Vec<ShellRequest>,
        now: f64,
    }

    impl Rig {
        fn new() -> Self {
            let constants = constants();
            Self {
                state: ChartState::new(&constants),
                cells: cells(),
                constants,
                actions: Vec::new(),
                requests: Vec::new(),
                now: 0.0,
            }
        }

        fn send(&mut self, model: &PipBoyModel, input: NavInput) -> InputResult {
            let mut env = InputEnv {
                constants: &self.constants,
                cells: &self.cells,
                error_seconds: 2.0,
                model,
                now: self.now,
                actions: &mut self.actions,
                requests: &mut self.requests,
            };
            self.state.handle_input(input, &mut env)
        }

        fn drag(&mut self, model: &PipBoyModel, button: PointerButton, delta: Vec2) {
            let env = InputEnv {
                constants: &self.constants,
                cells: &self.cells,
                error_seconds: 2.0,
                model,
                now: self.now,
                actions: &mut self.actions,
                requests: &mut self.requests,
            };
            self.state.dragged(button, delta, &env);
        }

        fn advance(&mut self, model: &PipBoyModel, now: f64) {
            self.now = now;
            let env = InputEnv {
                constants: &self.constants,
                cells: &self.cells,
                error_seconds: 2.0,
                model,
                now,
                actions: &mut self.actions,
                requests: &mut self.requests,
            };
            self.state.advance(now, &env);
        }

        fn centre(&self, cell: Cell) -> Pos2 {
            self.state
                .view
                .to_stage(self.cells[cell.index()].border.center())
        }

        fn hover(&mut self, model: &PipBoyModel, cell: Cell) {
            self.send(model, NavInput::PointerMoved(self.centre(cell)));
        }

        fn click(&mut self, model: &PipBoyModel, cell: Cell) {
            self.send(model, NavInput::Clicked(self.centre(cell)));
        }

        fn selected(&self) -> Option<Cell> {
            self.state.selected
        }
    }

    #[test]
    fn hovering_a_card_selects_it_and_leaving_every_card_clears_the_selection() {
        let model = model();
        let mut rig = Rig::new();

        rig.hover(&model, IRON_FIST);
        assert_eq!(rig.selected(), Some(IRON_FIST));
        rig.send(&model, NavInput::PointerMoved(Pos2::new(1270.0, 710.0)));

        assert_eq!(rig.selected(), None);
    }

    #[test]
    fn hovering_changes_nothing_but_the_selection() {
        let model = model();
        let mut rig = Rig::new();

        rig.hover(&model, IRON_FIST);

        assert!(!rig.state.is_confirming());
        assert!(rig.actions.is_empty() && rig.requests.is_empty());
    }

    #[test]
    fn clicking_a_takeable_perk_asks_for_confirmation_and_enter_takes_it_once() {
        let model = model();
        let mut rig = Rig::new();

        rig.click(&model, IRON_FIST);
        assert!(rig.state.is_confirming());
        assert!(rig.actions.is_empty(), "choosing alone changes nothing");
        rig.send(&model, NavInput::Hint(HintKey::Enter));

        assert_eq!(rig.actions, vec![PipBoyAction::RankUpPerk(iron_fist())]);
        assert!(!rig.state.is_confirming());
    }

    #[test]
    fn enter_with_no_selection_does_nothing() {
        let model = model();
        let mut rig = Rig::new();

        rig.send(&model, NavInput::Hint(HintKey::Enter));

        assert!(!rig.state.is_confirming() && rig.actions.is_empty());
    }

    #[test]
    fn while_confirming_the_selection_stays_and_a_click_does_nothing() {
        let model = model();
        let mut rig = Rig::new();
        rig.click(&model, IRON_FIST);

        rig.hover(&model, BIG_LEAGUES);
        rig.click(&model, BIG_LEAGUES);
        assert_eq!(rig.selected(), Some(IRON_FIST));
        rig.click(&model, IRON_FIST);

        assert!(rig.actions.is_empty() && rig.state.is_confirming());
    }

    #[test]
    fn a_perk_with_unmet_requirements_is_refused_for_two_seconds() {
        let model = model();
        let mut rig = Rig::new();
        rig.now = 10.0;

        rig.click(&model, BIG_LEAGUES);

        assert!(!rig.state.is_confirming());
        assert!(rig.state.error_visible(11.9));
        assert!(!rig.state.error_visible(12.0));
    }

    #[test]
    fn a_refusal_that_is_showing_is_not_restarted() {
        let model = model();
        let mut rig = Rig::new();
        rig.now = 10.0;
        rig.click(&model, BIG_LEAGUES);

        rig.now = 11.5;
        rig.click(&model, BIG_LEAGUES);

        assert!(!rig.state.error_visible(12.0), "the first timer runs out");
    }

    #[test]
    fn leaving_every_card_clears_the_refusal() {
        let model = model();
        let mut rig = Rig::new();
        rig.click(&model, BIG_LEAGUES);

        rig.send(&model, NavInput::PointerMoved(Pos2::new(1270.0, 710.0)));

        assert!(!rig.state.error_visible(0.5));
    }

    #[test]
    fn a_maxed_perk_is_neither_confirmed_nor_refused() {
        let mut model = model();
        model.perks.0.insert(iron_fist(), 3);
        let mut rig = Rig::new();

        rig.click(&model, IRON_FIST);

        assert!(!rig.state.is_confirming() && !rig.state.error_visible(0.0));
    }

    #[test]
    fn a_click_without_points_shows_no_refusal() {
        let mut model = model();
        model.character.progression.unspent_perk_points = 0;
        let mut rig = Rig::new();

        rig.click(&model, BIG_LEAGUES);

        assert!(!rig.state.error_visible(0.0));
    }

    #[test]
    fn tab_cancels_a_confirmation_first_and_closes_the_chart_after() {
        let model = model();
        let mut rig = Rig::new();
        rig.click(&model, IRON_FIST);

        rig.send(&model, NavInput::Hint(HintKey::Tab));
        assert!(rig.requests.is_empty() && !rig.state.is_confirming());
        rig.send(&model, NavInput::Hint(HintKey::Tab));

        assert_eq!(rig.requests, vec![ShellRequest::ClosePerkChart]);
    }

    #[test]
    fn the_arrow_keys_do_nothing_but_are_not_left_to_the_shell() {
        let model = model();
        let mut rig = Rig::new();
        rig.hover(&model, IRON_FIST);

        for input in [
            NavInput::Up,
            NavInput::Down,
            NavInput::PrevSubTab,
            NavInput::NextSubTab,
        ] {
            assert_eq!(rig.send(&model, input), InputResult::Consumed);
        }

        assert_eq!(rig.selected(), Some(IRON_FIST));
    }

    #[test]
    fn a_wheel_notch_zooms_by_a_sixth_of_the_scale_range_about_the_pointer() {
        let model = model();
        let mut rig = Rig::new();
        let pointer = Pos2::new(400.0, 300.0);
        rig.send(&model, NavInput::PointerMoved(pointer));
        let anchor = (pointer - Pos2::ZERO) / rig.state.view.scale;

        let result = rig.send(&model, NavInput::Scrolled(1.0));

        assert_eq!(result, InputResult::Consumed);
        let view = rig.state.view;
        let per_notch = (rig.constants.max_scale - rig.constants.min_scale) / 6.0;
        assert!((view.scale - (rig.constants.min_scale + per_notch)).abs() < 1e-6);
        let under_pointer = view.to_stage(Pos2::new(anchor.x, anchor.y));
        assert!((under_pointer - pointer).length() < 1e-3);
    }

    #[test]
    fn six_notches_run_the_whole_range_and_more_change_nothing() {
        let model = model();
        let mut rig = Rig::new();
        rig.send(&model, NavInput::PointerMoved(Pos2::new(400.0, 300.0)));

        for _ in 0..6 {
            rig.send(&model, NavInput::Scrolled(1.0));
        }
        assert!((rig.state.view.scale - rig.constants.max_scale).abs() < 1e-5);
        let full = rig.state.view;
        rig.send(&model, NavInput::Scrolled(1.0));
        assert_eq!(rig.state.view, full);

        for _ in 0..8 {
            rig.send(&model, NavInput::Scrolled(-1.0));
        }
        assert!((rig.state.view.scale - rig.constants.min_scale).abs() < 1e-5);
    }

    #[test]
    fn the_wheel_does_nothing_while_confirming_or_before_the_pointer_is_known() {
        let model = model();
        let mut rig = Rig::new();
        rig.send(&model, NavInput::Scrolled(1.0));
        assert_eq!(rig.state.view, GridView::initial(&rig.constants));

        rig.click(&model, IRON_FIST);
        rig.send(&model, NavInput::Scrolled(1.0));
        assert_eq!(rig.state.view, GridView::initial(&rig.constants));
    }

    #[test]
    fn the_card_under_a_resting_pointer_is_selected_after_a_zoom() {
        let model = model();
        let zoomed_out_selection = |pointer_y: f32| {
            let mut rig = Rig::new();
            rig.state.view = GridView {
                scale: rig.constants.max_scale,
                x: 0.0,
                y: 0.0,
            };
            let pointer = Pos2::new(120.0, pointer_y);
            rig.send(&model, NavInput::PointerMoved(pointer));
            let before = rig.selected();
            rig.send(&model, NavInput::Scrolled(-1.0));
            (
                before,
                rig.selected(),
                rig.state.cell_at(&rig.cells, pointer),
            )
        };

        let changed = (0..700)
            .step_by(5)
            .map(|y| zoomed_out_selection(y as f32))
            .filter(|(before, after, _)| before != after)
            .collect::<Vec<_>>();

        assert!(
            !changed.is_empty(),
            "the clamped zoom moves some card under the pointer"
        );
        assert!(changed.iter().all(|(_, after, under)| after == under));
    }

    #[test]
    fn leaving_the_window_clears_the_pointer_so_nothing_scrolls_or_zooms() {
        let model = model();
        let mut rig = Rig::new();
        let hover = rig.constants.hover;
        rig.send(
            &model,
            NavInput::PointerMoved(Pos2::new(640.0, hover.y_max + 1.0)),
        );
        rig.advance(&model, 0.0);
        rig.send(&model, NavInput::PointerLeft);

        rig.advance(&model, 1.0);
        rig.send(&model, NavInput::Scrolled(1.0));

        assert_eq!(rig.state.view, GridView::initial(&rig.constants));
    }

    #[test]
    fn a_lost_drag_end_is_cleared_by_losing_the_pointer_or_reshowing_the_chart() {
        let model = model();
        for reset in [0, 1] {
            let mut rig = Rig::new();
            let hover = rig.constants.hover;
            let pointer = Pos2::new(640.0, hover.y_max + 1.0);
            rig.send(&model, NavInput::PointerMoved(pointer));
            rig.advance(&model, 0.0);
            rig.drag(&model, PointerButton::Secondary, Vec2::new(0.0, -5.0));
            if reset == 0 {
                rig.send(&model, NavInput::PointerLeft);
                rig.send(&model, NavInput::PointerMoved(pointer));
            } else {
                rig.state.resume();
            }
            let dragged_to = rig.state.view.y;

            rig.advance(&model, 1.0 / 30.0);
            rig.advance(&model, 2.0 / 30.0);

            assert_eq!(rig.state.view.y, dragged_to - hover.y_speed, "case {reset}");
        }
    }

    #[test]
    fn time_spent_away_does_not_scroll_when_the_chart_is_shown_again() {
        let model = model();
        let mut rig = Rig::new();
        let hover = rig.constants.hover;
        rig.send(
            &model,
            NavInput::PointerMoved(Pos2::new(640.0, hover.y_max + 1.0)),
        );
        rig.advance(&model, 0.0);
        rig.state.resume();

        rig.advance(&model, 100.0);

        assert_eq!(
            rig.state.view.y, 0.0,
            "the first frame after resuming only starts the clock"
        );
    }

    #[test]
    fn a_slow_frame_scrolls_one_movie_frame_not_the_frames_it_missed() {
        let model = model();
        let mut rig = Rig::new();
        let hover = rig.constants.hover;
        rig.send(
            &model,
            NavInput::PointerMoved(Pos2::new(640.0, hover.y_max + 1.0)),
        );
        rig.advance(&model, 0.0);

        rig.advance(&model, 5.0 / 30.0);

        assert_eq!(rig.state.view.y, -hover.y_speed);
    }

    #[test]
    fn a_pointer_in_an_edge_zone_scrolls_the_grid_by_the_zone_speed_every_movie_frame() {
        let model = model();
        let mut rig = Rig::new();
        let hover = rig.constants.hover;
        rig.send(
            &model,
            NavInput::PointerMoved(Pos2::new(640.0, hover.y_max + 1.0)),
        );
        rig.advance(&model, 0.0);

        rig.advance(&model, 1.0 / 30.0);
        assert_eq!(rig.state.view.y, -hover.y_speed);
        rig.advance(&model, 2.0 / 30.0);
        assert_eq!(rig.state.view.y, -2.0 * hover.y_speed);
        rig.advance(&model, 2.0 / 30.0 + 0.001);
        assert_eq!(
            rig.state.view.y,
            -2.0 * hover.y_speed,
            "no frame is due yet"
        );
    }

    #[test]
    fn the_edge_scroll_needs_no_stick_input_and_stops_in_the_centre() {
        let model = model();
        let mut rig = Rig::new();
        rig.send(&model, NavInput::PointerMoved(Pos2::new(640.0, 360.0)));
        rig.advance(&model, 0.0);
        rig.advance(&model, 1.0);
        assert_eq!(rig.state.view, GridView::initial(&rig.constants));
    }

    #[test]
    fn the_edge_scroll_waits_while_a_right_drag_is_active_and_resumes_after_it() {
        let model = model();
        let mut rig = Rig::new();
        let hover = rig.constants.hover;
        rig.send(
            &model,
            NavInput::PointerMoved(Pos2::new(640.0, hover.y_max + 1.0)),
        );
        rig.advance(&model, 0.0);
        rig.drag(&model, PointerButton::Secondary, Vec2::new(0.0, -5.0));
        let dragged_to = rig.state.view.y;

        rig.advance(&model, 2.0 / 30.0);
        assert_eq!(rig.state.view.y, dragged_to);

        rig.send(&model, NavInput::DragEnded(PointerButton::Secondary));
        rig.advance(&model, 3.0 / 30.0);
        assert_eq!(rig.state.view.y, dragged_to - hover.y_speed);
    }

    #[test]
    fn a_held_but_unmoved_right_button_or_a_left_drag_does_not_stop_the_edge_scroll() {
        let model = model();
        let mut rig = Rig::new();
        let hover = rig.constants.hover;
        rig.send(
            &model,
            NavInput::PointerMoved(Pos2::new(640.0, hover.y_max + 1.0)),
        );
        rig.advance(&model, 0.0);
        rig.drag(&model, PointerButton::Secondary, Vec2::ZERO);
        rig.drag(&model, PointerButton::Primary, Vec2::new(0.0, 4.0));

        rig.advance(&model, 1.0 / 30.0);

        assert_eq!(rig.state.view.y, -hover.y_speed);
    }

    #[test]
    fn the_edge_scroll_waits_while_confirming() {
        let model = model();
        let mut rig = Rig::new();
        rig.click(&model, IRON_FIST);
        rig.send(&model, NavInput::PointerMoved(Pos2::new(640.0, 710.0)));
        rig.advance(&model, 0.0);
        rig.advance(&model, 1.0);
        assert_eq!(rig.state.view.y, 0.0);
    }

    #[test]
    fn the_card_under_a_resting_pointer_is_selected_after_an_edge_scroll() {
        let model = model();
        let mut rig = Rig::new();
        let hover = rig.constants.hover;
        let pointer = Pos2::new(120.0, hover.y_max + 1.0);
        rig.send(&model, NavInput::PointerMoved(pointer));
        rig.advance(&model, 0.0);
        rig.advance(&model, 20.0 / 30.0);

        assert!(rig.state.view.y < 0.0);
        assert_eq!(rig.selected(), rig.state.cell_at(&rig.cells, pointer));
    }

    #[test]
    fn a_right_button_drag_pans_the_grid_by_the_drag_speed_inside_the_poster() {
        let model = model();
        let mut rig = Rig::new();
        rig.constants.drag_speed = 2.0;

        rig.drag(&model, PointerButton::Secondary, Vec2::new(30.0, -50.0));

        assert_eq!(rig.state.view.x, 0.0, "the poster is as wide as the stage");
        assert_eq!(rig.state.view.y, -100.0);
    }

    #[test]
    fn only_the_right_button_pans() {
        assert_eq!(
            pan_of(PointerButton::Secondary, Vec2::new(1.0, 2.0), 3.0),
            Some(Vec2::new(3.0, 6.0))
        );
        assert_eq!(
            pan_of(PointerButton::Primary, Vec2::new(1.0, 2.0), 3.0),
            None
        );
    }

    #[test]
    fn a_left_button_drag_does_not_pan() {
        let model = model();
        let mut rig = Rig::new();

        rig.drag(&model, PointerButton::Primary, Vec2::new(0.0, -100.0));

        assert_eq!(rig.state.view.y, 0.0);
    }

    #[test]
    fn the_grid_does_not_pan_while_confirming() {
        let model = model();
        let mut rig = Rig::new();
        rig.click(&model, IRON_FIST);

        rig.drag(&model, PointerButton::Secondary, Vec2::new(0.0, -100.0));

        assert_eq!(rig.state.view.y, 0.0);
    }

    #[test]
    fn hover_follows_the_cards_after_the_grid_scrolls() {
        let model = model();
        let mut rig = Rig::new();
        rig.drag(&model, PointerButton::Secondary, Vec2::new(0.0, -100.0));

        rig.hover(&model, IRON_FIST);

        assert_eq!(rig.selected(), Some(IRON_FIST));
    }

    #[test]
    fn the_card_under_a_resting_pointer_is_selected_after_a_pan() {
        let model = model();
        let mut rig = Rig::new();
        rig.hover(&model, IRON_FIST);
        let distance = rig.centre(BIG_LEAGUES).y - rig.centre(IRON_FIST).y;

        rig.drag(&model, PointerButton::Secondary, Vec2::new(0.0, -distance));

        assert_eq!(rig.selected(), Some(BIG_LEAGUES));
    }

    #[test]
    fn a_fresh_selection_views_the_next_rank_while_points_remain() {
        let model = model();
        let mut rig = Rig::new();
        rig.hover(&model, IRON_FIST);

        let selection = rig.state.selection(&model).unwrap();

        assert_eq!(rig.state.viewed_rank(&selection), 1);
        assert_eq!(rig.state.held_ranks(&selection), (0, true));
    }

    #[test]
    fn the_rank_keys_step_through_the_ranks_and_stop_at_both_ends() {
        let model = model();
        let mut rig = Rig::new();
        rig.hover(&model, IRON_FIST);

        for _ in 0..5 {
            rig.send(&model, NavInput::Hint(HintKey::Alt));
        }
        let selection = rig.state.selection(&model).unwrap();
        assert_eq!(rig.state.viewed_rank(&selection), 3);
        assert_eq!(
            rig.state.held_ranks(&selection),
            (3, false),
            "no blinking while browsing"
        );
        for _ in 0..5 {
            rig.send(&model, NavInput::Hint(HintKey::Ctrl));
        }

        let selection = rig.state.selection(&model).unwrap();
        assert_eq!(rig.state.viewed_rank(&selection), 1);
    }

    #[test]
    fn an_available_perk_is_confirmed_while_another_rank_is_shown() {
        let model = model();
        let mut rig = Rig::new();
        rig.hover(&model, IRON_FIST);
        rig.send(&model, NavInput::Hint(HintKey::Alt));

        rig.send(&model, NavInput::Hint(HintKey::Enter));

        assert!(rig.state.is_confirming());
    }

    #[test]
    fn moving_to_another_card_forgets_the_browsed_rank() {
        let model = model();
        let mut rig = Rig::new();
        rig.hover(&model, IRON_FIST);
        rig.send(&model, NavInput::Hint(HintKey::Alt));
        assert!(rig.state.is_browsing());

        rig.hover(&model, BIG_LEAGUES);

        assert!(!rig.state.is_browsing());
    }

    #[test]
    fn the_rank_keys_work_while_confirming() {
        let model = model();
        let mut rig = Rig::new();
        rig.click(&model, IRON_FIST);

        rig.send(&model, NavInput::Hint(HintKey::Alt));

        assert!(rig.state.is_browsing());
    }

    #[test]
    fn after_taking_a_perk_the_shown_rank_starts_over() {
        let model = model();
        let mut rig = Rig::new();
        rig.hover(&model, IRON_FIST);
        rig.send(&model, NavInput::Hint(HintKey::Alt));
        rig.send(&model, NavInput::Hint(HintKey::Ctrl));
        rig.click(&model, IRON_FIST);
        rig.send(&model, NavInput::Hint(HintKey::Enter));

        assert!(!rig.state.is_browsing());
    }

    #[test]
    fn the_fixture_has_a_hit_box_for_every_cell() {
        assert_eq!(cells().len(), COLUMNS * ROWS);
    }
}
