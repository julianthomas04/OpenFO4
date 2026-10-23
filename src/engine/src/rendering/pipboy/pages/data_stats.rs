//! DATA / STATS: the category list and the selected category's label and value rows.

use egui::{Pos2, Rect};

use crate::rendering::pipboy::scaleform_text::{FieldSpec, TextAutoSize};
use crate::rendering::pipboy::scrolling_list::{
    ListKeys, ListLayout, ListState, ListView, list_keys,
};
use crate::rendering::pipboy::shapes::{OutlinedShape, rect_of};
use crate::rendering::pipboy::tab::{
    BottomBar, ButtonHint, HintKey, InputResult, NavInput, Tab, TabContext,
};
use crate::sim::pipboy::{PipBoySnapshot, StatCategory};
use crate::ui_layout::{
    BoundsKey, ModuleKeys, ShapeKey, TextKey, UiLayout, UiLayoutError, key_check_fields,
};

const MOVIE: &str = "Pipboy_DataPage";

pub(crate) static CATEGORIES: ListKeys = list_keys!(
    movie: "Pipboy_DataPage",
    list: "StatsTab_mc/CategoryList_mc",
    set_prop: "StatsTab.__setProp_CategoryList_mc_StatsTab_Categories_0",
    member: "CategoryList_mc",
    entry: "Stats_CategoriesListEntry",
    border: "@1",
    arrows: "@3",
);

pub(crate) static VALUES: ListKeys = list_keys!(
    movie: "Pipboy_DataPage",
    list: "StatsTab_mc/ValueList_mc",
    set_prop: "StatsTab.__setProp_ValueList_mc_StatsTab_Values_0",
    member: "ValueList_mc",
    entry: "Stats_ValuesListEntry",
    border: "@1",
    arrows: "@3",
);

struct StatsKeys {
    value: TextKey,
    value_bar: ShapeKey,
    values_area: BoundsKey,
}

static KEYS: StatsKeys = StatsKeys {
    value: TextKey::new(MOVIE, "[Stats_ValuesListEntry]/Value_tf"),
    value_bar: ShapeKey::new(MOVIE, "[Stats_ValuesListEntry]/border/@1"),
    values_area: BoundsKey::new(MOVIE, "StatsTab_mc/ValueList_mc/border"),
};

key_check_fields!(StatsKeys {
    value,
    value_bar,
    values_area
});

fn validate(layout: &UiLayout) -> Result<(), UiLayoutError> {
    StatsLayout::resolve(layout).map(drop)
}

pub static LAYOUT_KEYS: ModuleKeys = ModuleKeys {
    keys: &[&CATEGORIES, &VALUES, &KEYS],
    resolve: validate,
};

pub struct StatsLayout {
    categories: ListLayout,
    values: ListLayout,
    value: FieldSpec,
    value_bar: OutlinedShape,
    values_area: Rect,
}

impl StatsLayout {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        Ok(Self {
            categories: ListLayout::resolve(layout, &CATEGORIES)?,
            values: ListLayout::resolve(layout, &VALUES)?,
            value: FieldSpec::resolve(layout, &KEYS.value, TextAutoSize::None)?,
            value_bar: OutlinedShape::resolve(layout, &KEYS.value_bar)?,
            values_area: rect_of(layout.get(&KEYS.values_area)?),
        })
    }
}

pub struct StatsPage {
    layout: StatsLayout,
    categories: ListState,
    values: ListState,
    pointer: Option<Pos2>,
}

impl StatsPage {
    pub fn new(layout: StatsLayout) -> Self {
        Self {
            layout,
            categories: ListState::default(),
            values: ListState::default(),
            pointer: None,
        }
    }

    fn selected<'a>(&self, stats: &'a [StatCategory]) -> Option<&'a StatCategory> {
        self.categories
            .selected()
            .and_then(|index| stats.get(index))
    }

    fn pointer_over_values(&self) -> bool {
        self.pointer
            .is_some_and(|at| self.layout.values_area.contains(at))
    }

    fn handle_categories(&mut self, input: NavInput, len: usize) {
        let before = self.categories.selected();
        let view = ListView::new(&self.layout.categories, len);
        self.categories.handle(input, &view);
        if self.categories.selected() != before {
            self.values = ListState::default();
        }
    }
}

impl Tab for StatsPage {
    fn handle_input(&mut self, input: NavInput, cx: &mut TabContext<'_>) -> InputResult {
        let stats = &cx.snapshot.model.stats;
        let values_len = self
            .selected(stats)
            .map_or(0, |category| category.entries.len());
        match input {
            NavInput::Scrolled(_) if self.pointer_over_values() => {
                let view = ListView::new(&self.layout.values, values_len).display_only();
                self.values.handle(input, &view);
            }
            NavInput::Hint(HintKey::Ctrl) => {
                let view = ListView::new(&self.layout.values, values_len).display_only();
                self.values.handle(NavInput::Up, &view);
            }
            NavInput::Hint(HintKey::Alt) => {
                let view = ListView::new(&self.layout.values, values_len).display_only();
                self.values.handle(NavInput::Down, &view);
            }
            NavInput::PointerMoved(at) | NavInput::Clicked(at) => {
                self.pointer = Some(at);
                if matches!(input, NavInput::Clicked(_)) {
                    let view = ListView::new(&self.layout.values, values_len).display_only();
                    self.values.handle(input, &view);
                }
                self.handle_categories(input, stats.len());
            }
            NavInput::Up | NavInput::Down | NavInput::Scrolled(_) => {
                self.handle_categories(input, stats.len());
            }
            _ => return InputResult::Ignored,
        }
        InputResult::Consumed
    }

    fn draw(&mut self, cx: &mut TabContext<'_>) {
        let stats = &cx.snapshot.model.stats;
        let (tint, ui_layout) = (cx.tint, cx.layout);
        let view = ListView::new(&self.layout.categories, stats.len());
        self.categories.invalidate(&view);
        if self.categories.selected().is_none() {
            self.categories.select(&view, self.categories.scroll());
        }
        let layout = &self.layout;
        self.categories
            .paint(cx.painter, tint, &view, |painter, row| {
                let name = layout
                    .categories
                    .entry_text(ui_layout, &stats[row.index].name);
                layout.categories.paint_text(painter, row, &name);
            });
        self.categories.paint_arrows(cx, &view);
        let Some(category) = self.selected(stats) else {
            return;
        };
        let view = ListView::new(&layout.values, category.entries.len()).display_only();
        self.values.invalidate(&view);
        self.values.paint(cx.painter, tint, &view, |painter, row| {
            let entry = &category.entries[row.index];
            layout.value_bar.translated(row.origin).paint(painter, tint);
            let label = layout.values.entry_text(ui_layout, &entry.label);
            layout.values.paint_text(painter, row, &label);
            let value = layout
                .values
                .rules
                .apply(ui_layout, &entry.value, false, false);
            layout
                .value
                .translated(row.origin)
                .paint(painter, &value, tint);
        });
        self.values.paint_arrows(cx, &view);
    }

    fn hints(&self, _snapshot: &PipBoySnapshot<'_>) -> Vec<ButtonHint> {
        Vec::new()
    }

    fn bottom_bar(&self) -> BottomBar {
        BottomBar::DateTime {
            with_location: false,
        }
    }

    fn select_row(&mut self, row: usize) {
        let view = ListView::new(&self.layout.categories, row + 1);
        self.categories.select(&view, row);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::rendering::pipboy::pages::test_harness::Harness;
    use crate::rendering::pipboy::test_layout::pipboy_layout;
    use crate::sim::pipboy::stats::StatEntry;
    use crate::sim::pipboy::{PipBoyCatalog, PipBoyModel};

    fn category(name: &str, entries: usize) -> StatCategory {
        StatCategory {
            name: name.to_owned(),
            entries: (0..entries)
                .map(|index| StatEntry {
                    label: format!("{name} {index}"),
                    value: index.to_string(),
                })
                .collect(),
        }
    }

    fn model(categories: Vec<StatCategory>) -> PipBoyModel {
        let mut model = PipBoyModel::new(Arc::new(PipBoyCatalog::default()));
        model.stats = categories;
        model
    }

    fn page() -> StatsPage {
        StatsPage::new(StatsLayout::resolve(&pipboy_layout()).unwrap())
    }

    /// A point on the value list, which the test layout stacks on the category list.
    const OVER_VALUES: Pos2 = Pos2::new(5.0, 5.0);
    const ELSEWHERE: Pos2 = Pos2::new(500.0, 500.0);

    #[test]
    fn the_scroll_keys_move_the_values_and_stop_at_both_ends() {
        let model = model(vec![category("A", 4)]);
        let mut harness = Harness::new();
        let mut page = page();
        harness.draw(&mut page, &model);

        for _ in 0..9 {
            harness.send(&mut page, &model, NavInput::Hint(HintKey::Alt));
        }
        let end = page.values.scroll();
        assert!(end > 0);
        for _ in 0..9 {
            harness.send(&mut page, &model, NavInput::Hint(HintKey::Ctrl));
        }
        assert_eq!(page.values.scroll(), 0);
        assert!(
            page.values.selected().is_none(),
            "values are never selected"
        );
    }

    #[test]
    fn choosing_another_category_restarts_its_values_at_the_top() {
        let model = model(vec![category("A", 20), category("B", 20)]);
        let mut harness = Harness::new();
        let mut page = page();
        harness.draw(&mut page, &model);
        harness.send(&mut page, &model, NavInput::Hint(HintKey::Alt));
        assert!(page.values.scroll() > 0);

        harness.send(&mut page, &model, NavInput::Down);

        assert_eq!(page.values.scroll(), 0);
    }

    #[test]
    fn the_wheel_scrolls_the_values_only_while_the_pointer_is_over_them() {
        let model = model(vec![category("A", 20), category("B", 20)]);
        let mut harness = Harness::new();
        let mut page = page();
        harness.draw(&mut page, &model);

        harness.send(&mut page, &model, NavInput::PointerMoved(OVER_VALUES));
        harness.send(&mut page, &model, NavInput::Scrolled(-1.0));
        assert_eq!(page.values.scroll(), 1);

        harness.send(&mut page, &model, NavInput::PointerMoved(ELSEWHERE));
        harness.send(&mut page, &model, NavInput::Scrolled(-1.0));
        assert_eq!(
            page.values.scroll(),
            1,
            "the wheel away from the values leaves them alone"
        );
    }

    #[test]
    fn hovering_the_values_never_selects_a_value_row() {
        let model = model(vec![category("A", 20)]);
        let mut harness = Harness::new();
        let mut page = page();
        harness.draw(&mut page, &model);
        harness.send(&mut page, &model, NavInput::PointerMoved(OVER_VALUES));
        harness.send(&mut page, &model, NavInput::Scrolled(-1.0));
        assert!(page.values.selected().is_none());
    }

    #[test]
    fn the_page_draws_with_a_scrolled_value_list_and_without_categories() {
        let mut harness = Harness::new();
        let mut page = page();
        let model_with = model(vec![category("A", 20)]);
        harness.send(&mut page, &model_with, NavInput::Hint(HintKey::Alt));
        harness.draw(&mut page, &model_with);
        harness.draw(&mut page, &model(Vec::new()));
    }

    #[test]
    fn the_page_offers_no_hints_of_its_own() {
        let model = model(vec![category("A", 20)]);
        let page = page();
        let snapshot = PipBoySnapshot {
            model: &model,
            vitals: crate::rendering::pipboy::pages::test_harness::vitals(),
        };
        assert!(page.hints(&snapshot).is_empty());
    }
}
