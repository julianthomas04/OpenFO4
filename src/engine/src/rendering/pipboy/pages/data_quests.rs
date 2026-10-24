//! DATA / QUESTS: the quest list, the selected quest's Vault Boy art and objectives, or its
//! summary text.

use egui::{Pos2, Rect, Vec2};

use crate::assets::pipboy_art::{ArtClip, FixedArt, Icon};
use crate::rendering::pipboy::paint::fill;
use crate::rendering::pipboy::scaleform_text::{FieldSpec, TextAutoSize, text_height};
use crate::rendering::pipboy::scrolling_list::{
    ListEvent, ListKeys, ListLayout, ListState, ListView, Row, brightness, list_keys,
};
use crate::rendering::pipboy::shapes::{OutlinedShape, rect_of};
use crate::rendering::pipboy::tab::{
    BottomBar, ButtonHint, HintKey, InputResult, NavInput, ShellRequest, Tab, TabContext,
};
use crate::sim::pipboy::quests::{Quest, QuestObjective, QuestStatus};
use crate::sim::pipboy::{PipBoyAction, PipBoySnapshot};
use crate::ui_layout::{
    BoundsKey, LiteralKey, ModuleKeys, NumberKey, PlacementKey, ShapeKey, TextKey, TranslateKey,
    UiLayout, UiLayoutError, key_check_fields,
};

struct QuestArt<'a> {
    clip: &'a ArtClip,
    scale: f32,
    ends_in_summary: bool,
}

const MOVIE: &str = "Pipboy_DataPage";

pub(crate) static QUESTS: ListKeys = list_keys!(
    movie: "Pipboy_DataPage",
    list: "QuestsTab_mc/QuestsList_mc",
    set_prop: "QuestsTab.__setProp_QuestsList_mc_QuestsTab_Quests_0",
    member: "QuestsList_mc",
    entry: "QuestsListEntry",
    border: "@1",
    arrows: "@3",
);

pub(crate) static OBJECTIVES: ListKeys = list_keys!(
    movie: "Pipboy_DataPage",
    list: "QuestsTab_mc/ObjectivesList_mc",
    set_prop: "QuestsTab.__setProp_ObjectivesList_mc_QuestsTab_Objectives_0",
    member: "ObjectivesList_mc",
    entry: "ObjectivesListEntry",
    border: "@1",
    arrows: "@3",
);

struct QuestKeys {
    faded: LiteralKey,
    dimmed_alpha: NumberKey,
    tracked_square: ShapeKey,
    status_icon: PlacementKey,
    divider: ShapeKey,
    objective_square: ShapeKey,
    objective_status_icon: PlacementKey,
    objective_background: ShapeKey,
    objectives_area: BoundsKey,
    description: TextKey,
    objectives_buffer: NumberKey,
    vault_boy: PlacementKey,
    vault_boy_scale: LiteralKey,
    summary_label: TranslateKey,
    hide_summary_label: TranslateKey,
    show_on_map_label: TranslateKey,
}

static KEYS: QuestKeys = QuestKeys {
    faded: LiteralKey::new(MOVIE, "QuestsListEntry.<init>#FadedColor.brightness=", 1),
    dimmed_alpha: NumberKey::new(MOVIE, "Shared.GlobalFunc.DIMMED_ALPHA"),
    tracked_square: ShapeKey::new(MOVIE, "[QuestsListEntry]/EquipIcon_mc:active/@1"),
    status_icon: PlacementKey::new(MOVIE, "[QuestsListEntry]/EquipIcon_mc"),
    divider: ShapeKey::new(MOVIE, "[QuestsListEntry]:divider/@3"),
    objective_square: ShapeKey::new(MOVIE, "[ObjectivesListEntry]/EquipIcon_mc:active/@1"),
    objective_status_icon: PlacementKey::new(MOVIE, "[ObjectivesListEntry]/EquipIcon_mc"),
    objective_background: ShapeKey::new(MOVIE, "[ObjectivesListEntry]/Background_mc/@1"),
    objectives_area: BoundsKey::new(MOVIE, "QuestsTab_mc/ObjectivesList_mc/border"),
    description: TextKey::new(MOVIE, "QuestsTab_mc/Description_tf"),
    objectives_buffer: NumberKey::new(MOVIE, "QuestsTab._ObjectivesListBuffer"),
    vault_boy: PlacementKey::new(MOVIE, "QuestsTab_mc/VaultBoyHolder_mc"),
    vault_boy_scale: LiteralKey::new(
        MOVIE,
        "QuestsTab.redrawUIComponent#Quest_VaultBoyLoader.contentLoaderInfo.content.scaleX=",
        1,
    ),
    summary_label: TranslateKey::text("$ShowQuestSummary"),
    hide_summary_label: TranslateKey::text("$HideQuestSummary"),
    show_on_map_label: TranslateKey::text("$Show on Map"),
};

key_check_fields!(QuestKeys {
    faded,
    dimmed_alpha,
    tracked_square,
    status_icon,
    divider,
    objective_square,
    objective_status_icon,
    objective_background,
    objectives_area,
    description,
    objectives_buffer,
    vault_boy,
    vault_boy_scale,
    summary_label,
    hide_summary_label,
    show_on_map_label
});

fn validate(layout: &UiLayout) -> Result<(), UiLayoutError> {
    QuestsLayout::resolve(layout).map(drop)
}

pub static LAYOUT_KEYS: ModuleKeys = ModuleKeys {
    keys: &[&QUESTS, &OBJECTIVES, &KEYS],
    resolve: validate,
};

/// A row of the quest list: a quest, or the rule between the enabled and the disabled ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    Quest(usize),
    Divider,
}

/// The list rows: enabled quests first, then a divider (only when something precedes it) and
/// the disabled ones, each group in model order, which stands in for the game's `sortval`.
fn entries(quests: &[Quest]) -> Vec<Entry> {
    let enabled = |quest: &Quest| quest.status == QuestStatus::Active;
    let (open, closed): (Vec<usize>, Vec<usize>) =
        (0..quests.len()).partition(|&index| enabled(&quests[index]));
    let divider = (!open.is_empty() && !closed.is_empty()).then_some(Entry::Divider);
    open.into_iter()
        .map(Entry::Quest)
        .chain(divider)
        .chain(closed.into_iter().map(Entry::Quest))
        .collect()
}

/// Which icon a quest or objective row shows and whether it is dimmed. The model has no
/// per-objective active/enabled flags, so an objective is active exactly while it is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RowState {
    active: bool,
    enabled: bool,
    completed: bool,
}

impl RowState {
    fn of_quest(quest: &Quest) -> Self {
        let enabled = quest.status == QuestStatus::Active;
        Self {
            active: enabled && quest.tracked,
            enabled,
            completed: !enabled,
        }
    }

    fn of_objective(objective: &QuestObjective) -> Self {
        Self {
            active: !objective.completed,
            enabled: !objective.completed,
            completed: objective.completed,
        }
    }
}

pub struct QuestsLayout {
    quests: ListLayout,
    objectives: ListLayout,
    fade: f32,
    dimmed_alpha: f32,
    tracked_square: OutlinedShape,
    status_icon: Pos2,
    divider: OutlinedShape,
    objective_square: OutlinedShape,
    objective_status_icon: Pos2,
    objective_background: OutlinedShape,
    objectives_area: Rect,
    description: FieldSpec,
    objectives_shift: ObjectivesShift,
    vault_boy: Pos2,
    vault_boy_scale: f32,
    summary_label: String,
    hide_summary_label: String,
    show_on_map_label: String,
}

/// Where the objectives list goes below the selected quest's description. The list keeps its
/// last top that lay below its authored one: a description that ends above the authored top
/// leaves it where it is, and only a new Pip-Boy session starts from the authored top again.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ObjectivesShift {
    default_top: f32,
    description_top: f32,
    /// The buffer in stage units.
    buffer: f32,
}

impl ObjectivesShift {
    fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let list = layout.get(&OBJECTIVES.list)?;
        let parent_scale = list.world.matrix.d / list.local.matrix.d;
        let description = layout.get(&KEYS.description)?.placement.world.matrix;
        Ok(Self {
            default_top: list.world.matrix.ty,
            description_top: description.ty,
            buffer: layout.get(&KEYS.objectives_buffer)? as f32 * parent_scale,
        })
    }

    /// The list's top after a description `text_height` tall (0 when it is empty), given its
    /// `current` top.
    fn top_after(self, text_height: f32, current: f32) -> f32 {
        let below_description = self.description_top + text_height + self.buffer;
        if below_description > self.default_top {
            below_description
        } else {
            current
        }
    }

    fn offset(self, top: f32) -> Vec2 {
        Vec2::new(0.0, top - self.default_top)
    }
}

fn registration(layout: &UiLayout, key: &PlacementKey) -> Result<Pos2, UiLayoutError> {
    let matrix = layout.get(key)?.world.matrix;
    Ok(Pos2::new(matrix.tx, matrix.ty))
}

impl QuestsLayout {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        Ok(Self {
            quests: ListLayout::resolve(layout, &QUESTS)?,
            objectives: ListLayout::resolve(layout, &OBJECTIVES)?,
            fade: layout.lit_number(&KEYS.faded, 0)? as f32,
            dimmed_alpha: layout.get(&KEYS.dimmed_alpha)? as f32,
            tracked_square: OutlinedShape::resolve(layout, &KEYS.tracked_square)?,
            status_icon: registration(layout, &KEYS.status_icon)?,
            divider: OutlinedShape::resolve(layout, &KEYS.divider)?,
            objective_square: OutlinedShape::resolve(layout, &KEYS.objective_square)?,
            objective_status_icon: registration(layout, &KEYS.objective_status_icon)?,
            objective_background: OutlinedShape::resolve(layout, &KEYS.objective_background)?,
            objectives_area: rect_of(layout.get(&KEYS.objectives_area)?),
            description: FieldSpec::resolve(layout, &KEYS.description, TextAutoSize::None)?,
            objectives_shift: ObjectivesShift::resolve(layout)?,
            vault_boy: registration(layout, &KEYS.vault_boy)?,
            vault_boy_scale: layout.lit_number(&KEYS.vault_boy_scale, 0)? as f32,
            summary_label: layout.get(&KEYS.summary_label)?,
            hide_summary_label: layout.get(&KEYS.hide_summary_label)?,
            show_on_map_label: layout.get(&KEYS.show_on_map_label)?,
        })
    }

    /// A row's text colour: dimmed when disabled.
    fn row_colour(&self, row: &Row, state: RowState) -> egui::Color32 {
        if state.enabled {
            row.text_colour
        } else {
            brightness(row.text_colour, self.fade)
        }
    }
}

pub struct QuestsPage {
    layout: QuestsLayout,
    quests: ListState,
    objectives: ListState,
    summary: bool,
    art_started: f64,
    art_finished: bool,
    objectives_top: f32,
    pointer: Option<Pos2>,
}

impl QuestsPage {
    pub fn new(layout: QuestsLayout) -> Self {
        let objectives_top = layout.objectives_shift.default_top;
        Self {
            layout,
            quests: ListState::default(),
            objectives: ListState::default(),
            summary: false,
            art_started: 0.0,
            art_finished: false,
            objectives_top,
            pointer: None,
        }
    }

    fn selected_quest<'a>(&self, quests: &'a [Quest], rows: &[Entry]) -> Option<&'a Quest> {
        match rows.get(self.quests.selected()?)? {
            Entry::Quest(index) => quests.get(*index),
            Entry::Divider => None,
        }
    }

    fn pointer_over_objectives(&self, offset: Vec2) -> bool {
        self.pointer
            .is_some_and(|at| self.layout.objectives_area.translate(offset).contains(at))
    }

    fn description_height(&self, cx: &TabContext<'_>, quest: &Quest) -> f32 {
        let layout = &self.layout;
        let shown = layout.quests.entry_text(cx.layout, &quest.description);
        if shown.is_empty() {
            return 0.0;
        }
        let field = &layout.description;
        text_height(
            cx.painter,
            &shown,
            &field.geometry,
            field.style(egui::Color32::WHITE),
        )
    }

    /// Lets the selected quest's description (none when nothing is selected) move the
    /// objectives list, which the menu does on every redraw whether or not the summary shows.
    fn settle_objectives_top(&mut self, cx: &TabContext<'_>, quest: Option<&Quest>) {
        let height = quest.map_or(0.0, |quest| self.description_height(cx, quest));
        self.objectives_top = self
            .layout
            .objectives_shift
            .top_after(height, self.objectives_top);
    }

    fn objectives_offset(&self) -> Vec2 {
        self.layout.objectives_shift.offset(self.objectives_top)
    }

    /// The first open objective with a map target: what SHOW ON MAP flies to.
    fn map_target(quest: &Quest) -> Option<glam::Vec3> {
        quest
            .objectives
            .iter()
            .filter(|objective| !objective.completed)
            .find_map(|objective| objective.target)
    }

    /// Hands `input` to the quest list; the selection never rests on the divider, and a new
    /// selection starts its objectives from the top with the summary closed.
    fn navigate(&mut self, input: NavInput, rows: &[Entry], now: f64) -> ListEvent {
        let before = self.quests.selected();
        let view = ListView::new(&self.layout.quests, rows.len());
        let mut event = self.quests.handle(input, &view);
        let on_divider = |state: &ListState| {
            state.selected().and_then(|at| rows.get(at)) == Some(&Entry::Divider)
        };
        if on_divider(&self.quests) {
            if matches!(input, NavInput::Up | NavInput::Down) {
                event = self.quests.handle(input, &view);
            }
            if on_divider(&self.quests) {
                if let Some(previous) = before {
                    self.quests.select(&view, previous);
                }
                event = ListEvent::None;
            }
        }
        if self.quests.selected() != before {
            self.objectives = ListState::default();
            self.summary = false;
            self.art_started = now;
            self.art_finished = false;
        }
        event
    }

    fn toggle_tracked(quest: &Quest, actions: &mut Vec<PipBoyAction>) {
        actions.push(PipBoyAction::ToggleQuestTracked(quest.id.clone()));
    }

    fn objective_heights(&self, cx: &TabContext<'_>, quest: &Quest) -> (Vec<String>, Vec<f32>) {
        let shown: Vec<String> = quest
            .objectives
            .iter()
            .map(|objective| {
                self.layout
                    .objectives
                    .entry_text(cx.layout, &objective.text)
            })
            .collect();
        let heights = shown
            .iter()
            .map(|text| self.layout.objectives.multiline_height(cx.painter, text))
            .collect();
        (shown, heights)
    }

    fn scroll_objectives(&mut self, input: NavInput, cx: &TabContext<'_>, quest: &Quest) {
        let (_, heights) = self.objective_heights(cx, quest);
        let height = |index: usize| heights[index];
        let view = ListView::new(&self.layout.objectives, heights.len())
            .display_only()
            .offset(self.objectives_offset())
            .with_heights(&height);
        self.objectives.handle(input, &view);
    }
}

impl Tab for QuestsPage {
    fn handle_input(&mut self, input: NavInput, cx: &mut TabContext<'_>) -> InputResult {
        let snapshot = *cx.snapshot;
        let quests = &snapshot.model.quests;
        let rows = entries(quests);
        let selected = self.selected_quest(quests, &rows);
        match input {
            NavInput::Scrolled(_)
                if selected.is_some() && self.pointer_over_objectives(self.objectives_offset()) =>
            {
                if let Some(quest) = selected {
                    self.scroll_objectives(input, cx, quest);
                }
            }
            NavInput::Hint(HintKey::Ctrl | HintKey::Alt) => {
                if let Some(quest) = selected {
                    let step = if input == NavInput::Hint(HintKey::Ctrl) {
                        NavInput::Up
                    } else {
                        NavInput::Down
                    };
                    self.scroll_objectives(step, cx, quest);
                }
            }
            NavInput::PointerMoved(at) => {
                self.pointer = Some(at);
                self.navigate(input, &rows, cx.seconds_open);
            }
            NavInput::Clicked(at) => {
                self.pointer = Some(at);
                let event = self.navigate(input, &rows, cx.seconds_open);
                if let ListEvent::Activated(position) = event
                    && let Some(Entry::Quest(index)) = rows.get(position)
                {
                    Self::toggle_tracked(&quests[*index], cx.actions);
                }
            }
            NavInput::Up | NavInput::Down | NavInput::Scrolled(_) => {
                self.navigate(input, &rows, cx.seconds_open);
            }
            NavInput::Hint(HintKey::Enter | HintKey::E) => {
                if let Some(quest) = selected {
                    Self::toggle_tracked(quest, cx.actions);
                }
            }
            NavInput::Hint(HintKey::C) => {
                if selected.is_some() {
                    self.summary = !self.summary;
                }
            }
            NavInput::Hint(HintKey::R) => {
                let target = selected
                    .filter(|quest| quest.status == QuestStatus::Active)
                    .and_then(Self::map_target);
                if let Some(position) = target {
                    cx.requests.push(ShellRequest::ShowOnMap(position));
                }
            }
            _ => return InputResult::Ignored,
        }
        InputResult::Consumed
    }

    fn draw(&mut self, cx: &mut TabContext<'_>) {
        let snapshot = *cx.snapshot;
        let quests = &snapshot.model.quests;
        let rows = entries(quests);
        let view = ListView::new(&self.layout.quests, rows.len());
        self.quests.invalidate(&view);
        if self.quests.selected().is_none() && !rows.is_empty() {
            self.quests.select(&view, self.quests.scroll());
        }
        let (tint, ui_layout) = (cx.tint, cx.layout);
        let icons = self.paint_quest_rows(cx, quests, &rows);
        self.quests.paint_arrows(cx, &view);
        for (icon, at, colour) in icons {
            cx.draw_icon(icon, at, 1.0, colour);
        }
        let selected = self.selected_quest(quests, &rows);
        self.settle_objectives_top(cx, selected);
        let Some(quest) = selected else {
            return;
        };
        self.open_summary_when_art_ends(cx, quest);
        if self.summary {
            let text = self.layout.quests.entry_text(ui_layout, &quest.description);
            self.layout.description.paint(cx.painter, &text, tint);
        } else {
            self.draw_vault_boy(cx, quest);
        }
        self.draw_objectives(cx, quest);
    }

    fn hints(&self, snapshot: &PipBoySnapshot<'_>) -> Vec<ButtonHint> {
        let quests = &snapshot.model.quests;
        let Some(quest) = self.selected_quest(quests, &entries(quests)) else {
            return Vec::new();
        };
        let has_target = quest.status == QuestStatus::Active && Self::map_target(quest).is_some();
        let summary_label = if self.summary {
            &self.layout.hide_summary_label
        } else {
            &self.layout.summary_label
        };
        vec![
            ButtonHint::new(HintKey::R, self.layout.show_on_map_label.clone()).enabled(has_target),
            ButtonHint::new(HintKey::C, summary_label.clone()),
        ]
    }

    fn bottom_bar(&self) -> BottomBar {
        BottomBar::DateTime {
            with_location: false,
        }
    }

    fn select_row(&mut self, row: usize) {
        let view = ListView::new(&self.layout.quests, row + 1);
        self.quests.select(&view, row);
    }

    fn on_open(&mut self) {
        self.summary = false;
        self.objectives = ListState::default();
        self.objectives_top = self.layout.objectives_shift.default_top;
        self.art_started = 0.0;
        self.art_finished = false;
    }
}

impl QuestsPage {
    /// Paints the quest rows and returns the tick icons to draw over them (art is drawn through
    /// the tab context, which the row closure cannot borrow).
    fn paint_quest_rows(
        &self,
        cx: &TabContext<'_>,
        quests: &[Quest],
        rows: &[Entry],
    ) -> Vec<(Icon, Pos2, egui::Color32)> {
        let layout = &self.layout;
        let view = ListView::new(&layout.quests, rows.len());
        let mut icons = Vec::new();
        self.quests
            .paint(cx.painter, cx.tint, &view, |painter, row| {
                let quest = match rows[row.index] {
                    Entry::Divider => {
                        layout
                            .divider
                            .translated(row.origin)
                            .paint(painter, cx.tint);
                        return;
                    }
                    Entry::Quest(index) => &quests[index],
                };
                let state = RowState::of_quest(quest);
                let colour = layout.row_colour(row, state);
                let name = layout.quests.entry_text(cx.layout, &quest.name);
                let row = Row {
                    text_colour: colour,
                    ..*row
                };
                layout.quests.paint_text(painter, &row, &name);
                if state.active {
                    fill(
                        painter,
                        layout.tracked_square.bounds().translate(row.origin),
                        colour,
                    );
                }
                if state.completed {
                    icons.push((
                        Icon::QuestCompleted,
                        layout.status_icon + row.origin,
                        colour,
                    ));
                }
            });
        icons
    }

    /// The Vault Boy art of `quest`: its own movie's clip at the layout's scale, or the default
    /// boy at its own scale when the quest has no movie or its movie exports no clip. Only the
    /// default boy of a quest that names a movie ends in the auto summary.
    fn art_for<'a>(&self, cx: &TabContext<'a>, quest: &Quest) -> QuestArt<'a> {
        let library = cx.art_library;
        let default_boy = |ends_in_summary| QuestArt {
            clip: library.fixed(FixedArt::QuestDefaultBoy),
            scale: 1.0,
            ends_in_summary,
        };
        match quest.vault_boy_swf.as_deref() {
            None => default_boy(false),
            Some(path) => {
                if let Some(clip) = library.clip_for_swf(path) {
                    return QuestArt {
                        clip,
                        scale: self.layout.vault_boy_scale,
                        ends_in_summary: true,
                    };
                }
                crate::utils::warn_once(&format!("pipboy-quest-art:{path}"), || {
                    format!(
                        "no Pip-Boy Vault Boy art for quest movie '{path}', showing the default"
                    )
                });
                default_boy(true)
            }
        }
    }

    /// Once per selection, when the quest's Vault Boy movie reaches its last frame the menu
    /// toggles the summary (the toggle, so a summary already open closes).
    fn open_summary_when_art_ends(&mut self, cx: &TabContext<'_>, quest: &Quest) {
        if self.art_finished {
            return;
        }
        let art = self.art_for(cx, quest);
        if art.ends_in_summary
            && art
                .clip
                .reaches_last_frame(cx.seconds_open - self.art_started)
        {
            self.art_finished = true;
            self.summary = !self.summary;
        }
    }

    fn draw_vault_boy(&self, cx: &mut TabContext<'_>, quest: &Quest) {
        let art = self.art_for(cx, quest);
        let colour = cx.shade(1.0);
        let elapsed = cx.seconds_open - self.art_started;
        cx.draw_clip(
            art.clip,
            art.clip.frame_at(elapsed, None),
            self.layout.vault_boy,
            art.scale,
            colour,
        );
    }

    fn draw_objectives(&mut self, cx: &mut TabContext<'_>, quest: &Quest) {
        let layout = &self.layout;
        let (shown, heights) = self.objective_heights(cx, quest);
        let height = |index: usize| heights[index];
        let dimmed = |_: usize| true;
        let view = ListView::new(&layout.objectives, heights.len())
            .display_only()
            .offset(self.objectives_offset())
            .dimmed(&dimmed, layout.dimmed_alpha)
            .with_heights(&height);
        self.objectives.invalidate(&view);
        let mut icons = Vec::new();
        self.objectives
            .paint(cx.painter, cx.tint, &view, |painter, row| {
                let objective = &quest.objectives[row.index];
                let state = RowState::of_objective(objective);
                let background = &layout.objective_background;
                let bounds = background.bounds();
                let bar = Rect::from_min_size(
                    bounds.min + row.origin,
                    egui::vec2(bounds.width(), row.height),
                );
                background
                    .fitted_to(bar)
                    .paint_mapped(painter, cx.tint, |colour| colour.gamma_multiply(row.alpha));
                let colour = layout.row_colour(row, state);
                let row = Row {
                    text_colour: colour,
                    ..*row
                };
                layout
                    .objectives
                    .paint_text(painter, &row, &shown[row.index]);
                if state.active {
                    fill(
                        painter,
                        layout.objective_square.bounds().translate(row.origin),
                        colour,
                    );
                }
                if state.completed {
                    icons.push((
                        Icon::QuestCompleted,
                        layout.objective_status_icon + row.origin,
                        colour,
                    ));
                }
            });
        self.objectives.paint_arrows(cx, &view);
        for (icon, at, colour) in icons {
            cx.draw_icon(icon, at, 1.0, colour);
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the fixture arithmetic stays on exactly representable values"
)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::rendering::pipboy::pages::test_harness::{Harness, TWO_FRAME_MOVIE};
    use crate::rendering::pipboy::test_layout::pipboy_layout;
    use crate::sim::pipboy::{PipBoyCatalog, PipBoyModel, QuestId};

    fn quest(name: &str, status: QuestStatus) -> Quest {
        Quest {
            id: QuestId(name.to_owned()),
            name: name.to_owned(),
            description: String::new(),
            status,
            tracked: false,
            objectives: Vec::new(),
            vault_boy_swf: None,
        }
    }

    fn objective(text: &str, completed: bool, target: Option<glam::Vec3>) -> QuestObjective {
        QuestObjective {
            text: text.to_owned(),
            completed,
            target,
        }
    }

    fn model(quests: Vec<Quest>) -> PipBoyModel {
        let mut model = PipBoyModel::new(Arc::new(PipBoyCatalog::default()));
        model.quests = quests;
        model
    }

    fn page() -> QuestsPage {
        QuestsPage::new(QuestsLayout::resolve(&pipboy_layout()).unwrap())
    }

    #[test]
    fn enabled_quests_list_first_and_a_divider_separates_the_disabled_ones() {
        let quests = [
            quest("done", QuestStatus::Completed),
            quest("first", QuestStatus::Active),
            quest("second", QuestStatus::Active),
            quest("past", QuestStatus::Completed),
        ];
        assert_eq!(
            entries(&quests),
            vec![
                Entry::Quest(1),
                Entry::Quest(2),
                Entry::Divider,
                Entry::Quest(0),
                Entry::Quest(3),
            ]
        );
    }

    #[test]
    fn no_divider_leads_the_list_or_follows_a_list_without_disabled_quests() {
        assert_eq!(
            entries(&[quest("past", QuestStatus::Completed)]),
            vec![Entry::Quest(0)]
        );
        assert_eq!(
            entries(&[quest("open", QuestStatus::Active)]),
            vec![Entry::Quest(0)]
        );
        assert_eq!(entries(&[]), []);
    }

    #[test]
    fn a_tracked_quest_shows_the_square_and_a_finished_one_the_tick() {
        let mut tracked = quest("a", QuestStatus::Active);
        tracked.tracked = true;
        assert!(RowState::of_quest(&tracked).active);
        assert!(!RowState::of_quest(&quest("b", QuestStatus::Active)).active);
        let done = RowState::of_quest(&quest("c", QuestStatus::Completed));
        assert!(done.completed && !done.enabled && !done.active);
    }

    #[test]
    fn an_open_objective_is_active_and_a_finished_one_is_dimmed_with_a_tick() {
        let open = RowState::of_objective(&objective("a", false, None));
        assert!(open.active && open.enabled && !open.completed);
        let done = RowState::of_objective(&objective("b", true, None));
        assert!(done.completed && !done.enabled && !done.active);
    }

    #[test]
    fn the_objectives_stay_at_their_top_until_the_description_reaches_past_it() {
        let shift = ObjectivesShift {
            default_top: 100.0,
            description_top: 20.0,
            buffer: 5.0,
        };
        assert_eq!(shift.top_after(0.0, 100.0), 100.0);
        assert_eq!(shift.top_after(75.0, 100.0), 100.0);
        assert_eq!(shift.top_after(90.0, 100.0), 115.0);
        assert_eq!(shift.offset(115.0), Vec2::new(0.0, 15.0));
    }

    #[test]
    fn the_objectives_keep_a_lowered_top_when_a_later_description_ends_above_the_default() {
        let shift = ObjectivesShift {
            default_top: 100.0,
            description_top: 20.0,
            buffer: 5.0,
        };
        let lowered = shift.top_after(90.0, 100.0);
        assert_eq!(shift.top_after(0.0, lowered), lowered);
        assert_eq!(shift.top_after(76.0, lowered), 101.0);
    }

    #[test]
    fn a_disabled_row_is_scaled_by_the_fade_literal_as_authored() {
        let mut page = page();
        page.layout.fade = -0.5;
        let row = Row {
            index: 0,
            origin: Vec2::ZERO,
            height: 10.0,
            alpha: 1.0,
            text_colour: egui::Color32::from_rgb(200, 100, 50),
            selected: false,
        };
        let enabled = RowState::of_quest(&quest("a", QuestStatus::Active));
        let disabled = RowState::of_quest(&quest("b", QuestStatus::Completed));
        assert_eq!(page.layout.row_colour(&row, enabled), row.text_colour);
        assert_eq!(
            page.layout.row_colour(&row, disabled),
            egui::Color32::from_rgb(100, 50, 25)
        );
    }

    #[test]
    fn the_description_lowers_the_objectives_and_the_list_stays_visible_in_summary_mode() {
        let mut long = quest("a", QuestStatus::Active);
        long.description = "word ".repeat(400);
        long.objectives = vec![objective("open", false, None)];
        let model = model(vec![long]);
        let mut harness = Harness::new();
        let mut page = page();
        let default_top = 10.0;
        page.layout.objectives_shift = ObjectivesShift {
            default_top,
            description_top: 0.0,
            buffer: 1.0,
        };
        page.on_open();
        harness.draw(&mut page, &model);
        let lowered = page.objectives_top;
        assert!(lowered > default_top);
        harness.send(&mut page, &model, NavInput::Hint(HintKey::C));
        harness.draw(&mut page, &model);
        assert!(page.summary);
        assert_eq!(page.objectives_top, lowered);
        harness.draw(
            &mut page,
            &self::model(vec![quest("b", QuestStatus::Active)]),
        );
        assert_eq!(page.objectives_top, lowered);
        page.on_open();
        assert_eq!(page.objectives_top, default_top);
    }

    #[test]
    fn the_summary_opens_once_when_the_quest_movie_reaches_its_last_frame() {
        let mut art = quest("a", QuestStatus::Active);
        art.vault_boy_swf = Some(TWO_FRAME_MOVIE.to_owned());
        let model = model(vec![art]);
        let mut harness = Harness::new();
        let mut page = page();
        harness.draw(&mut page, &model);
        assert!(!page.summary);
        harness.seconds_open = 1.0;
        harness.draw(&mut page, &model);
        assert!(page.summary);
        harness.send(&mut page, &model, NavInput::Hint(HintKey::C));
        harness.draw(&mut page, &model);
        assert!(!page.summary);
    }

    #[test]
    fn a_quest_whose_movie_exports_no_clip_ends_its_default_boy_in_the_auto_summary() {
        let mut named = quest("a", QuestStatus::Active);
        named.vault_boy_swf = Some("Interface/missing.swf".to_owned());
        let mut harness = Harness::new();
        let mut page = page();
        let model_named = model(vec![named]);
        harness.seconds_open = 1.0;
        harness.draw(&mut page, &model_named);
        assert!(page.summary, "the one-frame default boy has ended");

        let mut unnamed_page = self::page();
        harness.draw(
            &mut unnamed_page,
            &model(vec![quest("a", QuestStatus::Active)]),
        );
        assert!(
            !unnamed_page.summary,
            "the quest without a movie never auto-summarises"
        );
    }

    #[test]
    fn down_steps_over_the_divider_and_stops_at_the_last_quest() {
        let model = model(vec![
            quest("a", QuestStatus::Active),
            quest("b", QuestStatus::Completed),
        ]);
        let mut harness = Harness::new();
        let mut page = page();
        harness.draw(&mut page, &model);
        harness.send(&mut page, &model, NavInput::Down);
        assert_eq!(page.quests.selected(), Some(2));
        harness.send(&mut page, &model, NavInput::Down);
        assert_eq!(page.quests.selected(), Some(2));
        harness.send(&mut page, &model, NavInput::Up);
        assert_eq!(page.quests.selected(), Some(0));
    }

    #[test]
    fn c_toggles_the_summary_and_a_new_selection_closes_it() {
        let model = model(vec![
            quest("a", QuestStatus::Active),
            quest("b", QuestStatus::Active),
        ]);
        let mut harness = Harness::new();
        let mut page = page();
        harness.draw(&mut page, &model);
        harness.send(&mut page, &model, NavInput::Hint(HintKey::C));
        assert!(page.summary);
        harness.draw(&mut page, &model);
        harness.send(&mut page, &model, NavInput::Down);
        assert!(!page.summary);
    }

    #[test]
    fn enter_toggles_the_selected_quest_and_the_hint_bar_offers_only_map_and_summary() {
        let model = model(vec![quest("a", QuestStatus::Active)]);
        let mut harness = Harness::new();
        let mut page = page();
        harness.draw(&mut page, &model);
        harness.send(&mut page, &model, NavInput::Hint(HintKey::Enter));
        assert_eq!(
            harness.take_actions(),
            vec![PipBoyAction::ToggleQuestTracked(QuestId("a".to_owned()))]
        );
        let snapshot = PipBoySnapshot {
            model: &model,
            vitals: crate::rendering::pipboy::pages::test_harness::vitals(),
        };
        assert_eq!(page.hints(&snapshot).len(), 2);
    }

    #[test]
    fn show_on_map_flies_to_the_first_open_objective_of_an_enabled_quest_only() {
        let target = glam::Vec3::new(1.0, 2.0, 3.0);
        let mut open = quest("a", QuestStatus::Active);
        open.objectives = vec![
            objective("done", true, Some(glam::Vec3::ZERO)),
            objective("open", false, Some(target)),
        ];
        let mut closed = quest("b", QuestStatus::Completed);
        closed.objectives = vec![objective("open", false, Some(target))];
        let model = model(vec![open, closed]);
        let mut harness = Harness::new();
        let mut page = page();
        harness.draw(&mut page, &model);

        harness.send(&mut page, &model, NavInput::Hint(HintKey::R));
        assert_eq!(
            harness.take_requests(),
            vec![ShellRequest::ShowOnMap(target)]
        );

        harness.send(&mut page, &model, NavInput::Down);
        harness.send(&mut page, &model, NavInput::Hint(HintKey::R));
        assert_eq!(harness.take_requests(), []);
    }

    #[test]
    fn the_page_draws_in_both_modes_with_and_without_quests() {
        let mut with_objectives = quest("a", QuestStatus::Active);
        with_objectives.objectives = vec![
            objective("first", false, None),
            objective("second", true, None),
        ];
        with_objectives.description = "details".to_owned();
        let model = model(vec![with_objectives, quest("b", QuestStatus::Completed)]);
        let mut harness = Harness::new();
        let mut page = page();
        harness.draw(&mut page, &model);
        harness.send(&mut page, &model, NavInput::Hint(HintKey::C));
        harness.draw(&mut page, &model);
        harness.draw(&mut page, &self::model(Vec::new()));
    }
}
