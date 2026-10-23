//! DATA / WORKSHOPS: the settlement list and the selected settlement's seven resource rows.

use egui::{Pos2, Vec2};

use crate::assets::pipboy_art::Icon;
use crate::rendering::pipboy::paint::BLACK;
use crate::rendering::pipboy::scaleform_text::{FieldSpec, TextAutoSize};
use crate::rendering::pipboy::scrolling_list::{
    ListKeys, ListLayout, ListState, ListView, Row, list_keys,
};
use crate::rendering::pipboy::shapes::OutlinedShape;
use crate::rendering::pipboy::tab::{
    BottomBar, ButtonHint, HintKey, InputResult, NavInput, ShellRequest, Tab, TabContext,
};
use crate::sim::pipboy::PipBoySnapshot;
use crate::sim::pipboy::workshops::{Settlement, SettlementStats};
use crate::ui_layout::{
    BoundsKey, LiteralKey, ModuleKeys, PlacementKey, ShapeKey, TextKey, TranslateKey, UiLayout,
    UiLayoutError, key_check_fields,
};

const MOVIE: &str = "Pipboy_DataPage";

pub(crate) static LIST: ListKeys = list_keys!(
    movie: "Pipboy_DataPage",
    list: "WorkshopsTab_mc/List_mc",
    set_prop: "WorkshopsTab.__setProp_List_mc_WorkshopsTab_List_0",
    entry: "WorkshopsListEntry",
    border: "@1",
    arrows: "@3",
);

struct CardRowKeys {
    background: ShapeKey,
    icon: PlacementKey,
    label: TextKey,
    value: TextKey,
    alert: PlacementKey,
    label_text: TranslateKey,
}

key_check_fields!(CardRowKeys {
    background,
    icon,
    label,
    value,
    alert,
    label_text
});

macro_rules! card_row {
    ($entry:literal, $label:literal, $background:literal, $icon:literal) => {
        CardRowKeys {
            background: ShapeKey::new(
                MOVIE,
                concat!("WorkshopsTab_mc/ItemCard_mc/", $background, "/@1"),
            ),
            icon: PlacementKey::new(MOVIE, concat!("WorkshopsTab_mc/ItemCard_mc/", $icon)),
            label: TextKey::new(
                MOVIE,
                concat!("WorkshopsTab_mc/ItemCard_mc/", $entry, "/Label_tf"),
            ),
            value: TextKey::new(
                MOVIE,
                concat!("WorkshopsTab_mc/ItemCard_mc/", $entry, "/Value_tf"),
            ),
            alert: PlacementKey::new(
                MOVIE,
                concat!("WorkshopsTab_mc/ItemCard_mc/", $entry, "/Alert_mc:Alert/@1"),
            ),
            label_text: TranslateKey::text($label),
        }
    };
}

struct WorkshopKeys {
    rows: [CardRowKeys; 7],
    alert_icon: PlacementKey,
    alert_bounds: BoundsKey,
    raider_icon: PlacementKey,
    icon_gap: LiteralKey,
    raider_gap: LiteralKey,
    text: TextKey,
    hint_label: TranslateKey,
}

static KEYS: WorkshopKeys = WorkshopKeys {
    rows: [
        card_row!("PeopleEntry_mc", "$WS_People", "@1", "@15"),
        card_row!("FoodEntry_mc", "$WS_Food", "@3", "@17"),
        card_row!("WaterEntry_mc", "$WS_Water", "@5", "@19"),
        card_row!("PowerEntry_mc", "$WS_Power", "@7", "@21"),
        card_row!("SafetyEntry_mc", "$WS_Safety", "@9", "@23"),
        card_row!("BedsEntry_mc", "$WS_Beds", "@11", "@25"),
        card_row!("HappyEntry_mc", "$WS_Happy", "@13", "@27"),
    ],
    alert_icon: PlacementKey::new(MOVIE, "[WorkshopsListEntry]/AlertIcon_mc"),
    alert_bounds: BoundsKey::new(MOVIE, "[WorkshopsListEntry]/AlertIcon_mc"),
    raider_icon: PlacementKey::new(MOVIE, "[WorkshopsListEntry]/RaiderIcon_mc"),
    icon_gap: LiteralKey::new(
        MOVIE,
        "WorkshopsListEntry.SetEntryText#(textField.getLineMetrics().width+textField.x)+",
        1,
    ),
    raider_gap: LiteralKey::new(
        MOVIE,
        "WorkshopsListEntry.SetEntryText#($4+AlertIcon_mc.width)+",
        1,
    ),
    text: TextKey::new(MOVIE, "[WorkshopsListEntry]/textField"),
    hint_label: TranslateKey::text("$Show on Map"),
};

key_check_fields!(WorkshopKeys {
    rows,
    alert_icon,
    alert_bounds,
    raider_icon,
    icon_gap,
    raider_gap,
    text,
    hint_label
});

fn validate(layout: &UiLayout) -> Result<(), UiLayoutError> {
    WorkshopsLayout::resolve(layout).map(drop)
}

pub static LAYOUT_KEYS: ModuleKeys = ModuleKeys {
    keys: &[&LIST, &KEYS],
    resolve: validate,
};

/// The seven resources of a settlement's card, in the card's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Resource {
    People,
    Food,
    Water,
    Power,
    Defense,
    Beds,
    Happiness,
}

impl Resource {
    const ALL: [Resource; 7] = [
        Resource::People,
        Resource::Food,
        Resource::Water,
        Resource::Power,
        Resource::Defense,
        Resource::Beds,
        Resource::Happiness,
    ];

    fn icon(self) -> Icon {
        match self {
            Resource::People => Icon::Population,
            Resource::Food => Icon::Food,
            Resource::Water => Icon::Water,
            Resource::Power => Icon::Power,
            Resource::Defense => Icon::Defense,
            Resource::Beds => Icon::Beds,
            Resource::Happiness => Icon::Happiness,
        }
    }

    fn value(self, stats: &SettlementStats) -> u32 {
        match self {
            Resource::People => stats.population,
            Resource::Food => stats.food,
            Resource::Water => stats.water,
            Resource::Power => stats.power,
            Resource::Defense => stats.defense,
            Resource::Beds => stats.beds,
            Resource::Happiness => stats.happiness,
        }
    }
}

/// Stand-in for the game's native per-resource rating (the card shows its alert for a negative
/// one): food, water, defense and beds below the population, or happiness below
/// [`SettlementRating::UNHAPPY_BELOW`]. The model carries no rating.
struct SettlementRating;

impl SettlementRating {
    const UNHAPPY_BELOW: u32 = 50;

    fn is_alert(resource: Resource, stats: &SettlementStats) -> bool {
        match resource {
            Resource::People | Resource::Power => false,
            Resource::Happiness => stats.happiness < Self::UNHAPPY_BELOW,
            Resource::Food | Resource::Water | Resource::Defense | Resource::Beds => {
                resource.value(stats) < stats.population
            }
        }
    }
}

/// The settlements in list order: by name, comparing characters as they are (upper case first).
fn display_order(settlements: &[Settlement]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..settlements.len()).collect();
    order.sort_by(|&a, &b| settlements[a].name.cmp(&settlements[b].name));
    order
}

struct CardRow {
    background: OutlinedShape,
    icon_at: Pos2,
    icon_scale: f32,
    label: FieldSpec,
    label_text: String,
    value: FieldSpec,
    alert_at: Pos2,
    alert_scale: f32,
}

/// An icon of the list entry: where it sits in the entry and at what scale.
#[derive(Clone, Copy)]
struct EntryIcon {
    y: f32,
    scale: f32,
}

pub struct WorkshopsLayout {
    list: ListLayout,
    rows: Vec<CardRow>,
    alert: EntryIcon,
    alert_width: f32,
    raider: EntryIcon,
    icon_gap: f32,
    raider_gap: f32,
    text_x: f32,
    hint_label: String,
}

fn entry_icon(layout: &UiLayout, key: &PlacementKey) -> Result<EntryIcon, UiLayoutError> {
    let matrix = layout.get(key)?.local.matrix;
    Ok(EntryIcon {
        y: matrix.ty,
        scale: matrix.a,
    })
}

impl WorkshopsLayout {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let rows = KEYS
            .rows
            .iter()
            .map(|keys| {
                let icon = layout.get(&keys.icon)?.world.matrix;
                let alert = layout.get(&keys.alert)?.world.matrix;
                Ok(CardRow {
                    background: OutlinedShape::resolve(layout, &keys.background)?,
                    icon_at: Pos2::new(icon.tx, icon.ty),
                    icon_scale: icon.a,
                    label: FieldSpec::resolve(layout, &keys.label, TextAutoSize::Shrink)?,
                    label_text: layout.get(&keys.label_text)?,
                    value: FieldSpec::resolve(layout, &keys.value, TextAutoSize::None)?,
                    alert_at: Pos2::new(alert.tx, alert.ty),
                    alert_scale: alert.a,
                })
            })
            .collect::<Result<Vec<_>, UiLayoutError>>()?;
        let [left, _, right, _] = layout.get(&KEYS.alert_bounds)?;
        Ok(Self {
            list: ListLayout::resolve(layout, &LIST)?,
            rows,
            alert: entry_icon(layout, &KEYS.alert_icon)?,
            alert_width: right - left,
            raider: entry_icon(layout, &KEYS.raider_icon)?,
            icon_gap: layout.lit_number(&KEYS.icon_gap, 0)? as f32,
            raider_gap: layout.lit_number(&KEYS.raider_gap, 0)? as f32,
            hint_label: layout.get(&KEYS.hint_label)?,
            text_x: layout.get(&KEYS.text)?.placement.local.matrix.tx,
        })
    }

    /// Where the entry puts its alert and raider icons after a name whose text spans `run`
    /// (first glyph left to last advance end): the alert first when shown, then the raider icon.
    fn icon_x(&self, run: (f32, f32), alert: bool) -> (f32, f32) {
        let after_text = ((run.1 - run.0) + self.text_x + self.icon_gap).trunc();
        let raider = if alert {
            after_text + self.alert_width + self.raider_gap
        } else {
            after_text
        };
        (after_text, raider)
    }
}

pub struct WorkshopsPage {
    layout: WorkshopsLayout,
    list: ListState,
}

impl WorkshopsPage {
    pub fn new(layout: WorkshopsLayout) -> Self {
        Self {
            layout,
            list: ListState::default(),
        }
    }

    fn selected<'a>(
        &self,
        settlements: &'a [Settlement],
        order: &[usize],
    ) -> Option<&'a Settlement> {
        self.list
            .selected()
            .and_then(|position| order.get(position))
            .map(|&index| &settlements[index])
    }

    fn paint_entry(&self, cx: &mut TabContext<'_>, row: &Row, settlement: &Settlement) {
        let layout = &self.layout;
        let shown = layout.list.entry_text(cx.layout, &settlement.name);
        let painted = layout.list.paint_text(cx.painter, row, &shown);
        let colour = if row.selected { BLACK } else { row.text_colour };
        let (alert_x, raider_x) = layout.icon_x(painted.run, settlement.needs_attention);
        if settlement.needs_attention {
            let at = row.origin.to_pos2() + Vec2::new(alert_x, layout.alert.y);
            cx.draw_icon(Icon::Alert, at, layout.alert.scale, colour);
        }
        if settlement.raider_owned {
            let at = row.origin.to_pos2() + Vec2::new(raider_x, layout.raider.y);
            cx.draw_icon(Icon::RaiderSkullDiscovered, at, layout.raider.scale, colour);
        }
    }

    fn draw_card(&self, cx: &mut TabContext<'_>, settlement: &Settlement) {
        let layout = &self.layout;
        let tint = cx.tint;
        let white = cx.shade(1.0);
        for (row, resource) in layout.rows.iter().zip(Resource::ALL) {
            row.background.paint(cx.painter, tint);
            cx.draw_icon(resource.icon(), row.icon_at, row.icon_scale, white);
            row.label.paint(cx.painter, &row.label_text, tint);
            let value = resource.value(&settlement.stats).to_string();
            let value = layout.list.rules.apply(cx.layout, &value, false, false);
            row.value.paint(cx.painter, &value, tint);
            if SettlementRating::is_alert(resource, &settlement.stats) {
                cx.draw_icon(Icon::Alert, row.alert_at, row.alert_scale, white);
            }
        }
    }
}

impl Tab for WorkshopsPage {
    fn handle_input(&mut self, input: NavInput, cx: &mut TabContext<'_>) -> InputResult {
        let snapshot = *cx.snapshot;
        let settlements = &snapshot.model.settlements;
        let order = display_order(settlements);
        match input {
            NavInput::Hint(HintKey::R) => {
                if let Some(position) = self
                    .selected(settlements, &order)
                    .and_then(|settlement| settlement.location)
                {
                    cx.requests.push(ShellRequest::ShowOnMap(position));
                }
                InputResult::Consumed
            }
            NavInput::Up
            | NavInput::Down
            | NavInput::Scrolled(_)
            | NavInput::PointerMoved(_)
            | NavInput::Clicked(_) => {
                let view = ListView::new(&self.layout.list, order.len());
                self.list.handle(input, &view);
                InputResult::Consumed
            }
            _ => InputResult::Ignored,
        }
    }

    fn draw(&mut self, cx: &mut TabContext<'_>) {
        let snapshot = *cx.snapshot;
        let settlements = &snapshot.model.settlements;
        let order = display_order(settlements);
        let view = ListView::new(&self.layout.list, order.len());
        self.list.invalidate(&view);
        if self.list.selected().is_none() {
            self.list.select(&view, self.list.scroll());
        }
        let painter = cx.painter.clone();
        let tint = cx.tint;
        let this = &*self;
        let mut rows = Vec::new();
        this.list
            .paint(&painter, tint, &view, |_, row| rows.push(*row));
        for row in &rows {
            this.paint_entry(cx, row, &settlements[order[row.index]]);
        }
        this.list.paint_arrows(cx, &view);
        if let Some(settlement) = self.selected(settlements, &order) {
            self.draw_card(cx, settlement);
        }
    }

    fn hints(&self, snapshot: &PipBoySnapshot<'_>) -> Vec<ButtonHint> {
        let settlements = &snapshot.model.settlements;
        let order = display_order(settlements);
        let Some(settlement) = self.selected(settlements, &order) else {
            return Vec::new();
        };
        vec![
            ButtonHint::new(HintKey::R, self.layout.hint_label.clone())
                .enabled(settlement.location.is_some()),
        ]
    }

    fn bottom_bar(&self) -> BottomBar {
        BottomBar::DateTime {
            with_location: false,
        }
    }

    fn select_row(&mut self, row: usize) {
        let view = ListView::new(&self.layout.list, row + 1);
        self.list.select(&view, row);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glam::Vec3;

    use super::*;
    use crate::rendering::pipboy::pages::test_harness::{Harness, vitals};
    use crate::rendering::pipboy::test_layout::pipboy_layout;
    use crate::sim::pipboy::{PipBoyCatalog, PipBoyModel};

    fn stats() -> SettlementStats {
        SettlementStats {
            population: 10,
            food: 12,
            water: 9,
            power: 0,
            defense: 10,
            beds: 3,
            happiness: 49,
        }
    }

    fn settlement(name: &str, location: Option<Vec3>) -> Settlement {
        Settlement {
            name: name.to_owned(),
            needs_attention: false,
            raider_owned: false,
            stats: stats(),
            location,
        }
    }

    fn page() -> WorkshopsPage {
        WorkshopsPage::new(WorkshopsLayout::resolve(&pipboy_layout()).unwrap())
    }

    fn model(settlements: Vec<Settlement>) -> PipBoyModel {
        let mut model = PipBoyModel::new(Arc::new(PipBoyCatalog::default()));
        model.settlements = settlements;
        model
    }

    #[test]
    fn food_water_defense_and_beds_alert_when_below_the_population() {
        let stats = stats();
        let alerts: Vec<bool> = Resource::ALL
            .into_iter()
            .map(|resource| SettlementRating::is_alert(resource, &stats))
            .collect();
        assert_eq!(alerts, [false, false, true, false, false, true, true]);
    }

    #[test]
    fn settlements_list_by_name_comparing_characters_as_they_are() {
        let settlements = [
            settlement("beta", None),
            settlement("Zeta", None),
            settlement("alpha", None),
            settlement("Beta", None),
        ];
        assert_eq!(display_order(&settlements), vec![3, 1, 2, 0]);
    }

    #[test]
    fn the_card_values_are_plain_numbers_in_the_cards_order() {
        let stats = stats();
        let values: Vec<u32> = Resource::ALL
            .into_iter()
            .map(|resource| resource.value(&stats))
            .collect();
        assert_eq!(values, [10, 12, 9, 0, 10, 3, 49]);
    }

    #[test]
    fn the_icons_follow_the_text_and_the_raider_icon_follows_the_alert_when_shown() {
        let layout = WorkshopsLayout::resolve(&pipboy_layout()).unwrap();
        let (alert, raider_with_alert) = layout.icon_x((2.0, 52.7), true);
        let (_, raider_alone) = layout.icon_x((2.0, 52.7), false);
        assert!((alert - raider_alone).abs() < f32::EPSILON);
        assert!(raider_with_alert > alert);
        assert!(alert.fract().abs() < f32::EPSILON);
    }

    #[test]
    fn show_on_map_asks_for_the_selected_settlements_location() {
        let here = Vec3::new(1.0, 0.0, 2.0);
        let model = model(vec![settlement("B", None), settlement("A", Some(here))]);
        let mut harness = Harness::new();
        let mut page = page();
        harness.draw(&mut page, &model);

        harness.send(&mut page, &model, NavInput::Hint(HintKey::R));

        assert_eq!(harness.take_requests(), vec![ShellRequest::ShowOnMap(here)]);
    }

    #[test]
    fn the_hint_is_enabled_only_for_a_settlement_with_a_location() {
        let located = model(vec![settlement("A", Some(Vec3::ONE))]);
        let unlocated = model(vec![settlement("A", None)]);
        let mut harness = Harness::new();
        let mut page = page();
        let hint = |page: &WorkshopsPage, model: &PipBoyModel| {
            page.hints(&PipBoySnapshot {
                model,
                vitals: vitals(),
            })
        };
        harness.draw(&mut page, &located);
        assert!(hint(&page, &located)[0].enabled);
        assert!(!hint(&page, &unlocated)[0].enabled);
    }

    #[test]
    fn the_page_draws_with_and_without_settlements() {
        let mut harness = Harness::new();
        let mut page = page();
        harness.draw(&mut page, &model(Vec::new()));
        let mut busy = settlement("A", None);
        busy.needs_attention = true;
        busy.raider_owned = true;
        harness.draw(&mut page, &model(vec![busy]));
    }
}
