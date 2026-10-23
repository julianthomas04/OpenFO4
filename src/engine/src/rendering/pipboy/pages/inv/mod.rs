//! INV: the seven inventory lists with the item card, paper doll, component view and prompts.

mod card;
mod card_layout;
mod card_model;
mod components;
mod controls;
mod doll;
mod entry_text;
mod favourites;
mod layout;
mod modal;
mod native;
mod quantity;

use std::sync::Arc;

use egui::Pos2;

use self::controls::Action;
use self::doll::DollView;
use self::entry_text::{EntryFlags, list_entry_text};
use self::favourites::{Favourites, FavouritesOutcome};
use self::quantity::{Quantity, QuantityOutcome};
use crate::assets::pipboy_art::Icon;
use crate::rendering::pipboy::scrolling_list::{ListEvent, ListState, ListView};
use crate::rendering::pipboy::tab::{
    BottomBar, ButtonHint, HintKey, InputResult, NavInput, Tab, TabContext,
};
use crate::sim::pipboy::inventory::{InventoryItem, ItemDetails, SortMode};
use crate::sim::pipboy::{Inventory, InventoryCategory, PipBoyAction, PipBoySnapshot};

#[cfg(test)]
pub(crate) use layout::{COMPONENT_LIST, LIST, OWNER_LIST};
pub use layout::{InventoryLayout, LAYOUT_KEYS};

/// The sort mode after `current` in the category's cycle.
pub fn next_sort_mode(category: InventoryCategory, current: SortMode) -> SortMode {
    let modes = category.sort_modes();
    let index = modes
        .iter()
        .position(|mode| *mode == current)
        .map_or(0, |i| (i + 1) % modes.len());
    modes[index]
}

/// What accepting `item` does: equip or unequip gear, use aid, nothing for the rest.
pub fn accept_action(item: &InventoryItem) -> Option<PipBoyAction> {
    match item.details {
        ItemDetails::Weapon(_) | ItemDetails::Apparel(_) => Some(if item.equipped {
            PipBoyAction::Unequip(item.id)
        } else {
            PipBoyAction::Equip(item.id)
        }),
        ItemDetails::Aid(_) => Some(PipBoyAction::Use(item.id)),
        _ => None,
    }
}

/// The equipped item `item` would replace, for the comparison arrows.
pub fn counterpart<'a>(
    inventory: &'a Inventory,
    item: &InventoryItem,
) -> Option<&'a InventoryItem> {
    if item.equipped {
        return None;
    }
    inventory.items.iter().find(|other| {
        other.equipped
            && match (&item.details, &other.details) {
                (ItemDetails::Weapon(_), ItemDetails::Weapon(_)) => true,
                (ItemDetails::Apparel(a), ItemDetails::Apparel(b)) => {
                    a.slot_mask & b.slot_mask != 0
                }
                _ => false,
            }
    })
}

/// Whether the favourite key does anything on `category`'s item list: the tabs before MISC.
fn favourites_on(category: InventoryCategory) -> bool {
    (category as usize) < InventoryCategory::Misc as usize
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Modal {
    None,
    Quantity(Quantity),
    Favourites(Favourites),
}

pub struct InventoryPage {
    category: InventoryCategory,
    layout: Arc<InventoryLayout>,
    list: ListState,
    components: ListState,
    owners: ListState,
    component_view: bool,
    modal: Modal,
    /// Which of the doll's damage types the badges show.
    cycle: usize,
    pointer: Option<Pos2>,
    shown_item: Option<crate::sim::pipboy::ItemId>,
    selected_at: f64,
    pending_select: Option<usize>,
}

impl InventoryPage {
    pub fn new(category: InventoryCategory, layout: Arc<InventoryLayout>) -> Self {
        Self {
            category,
            layout,
            list: ListState::default(),
            components: ListState::default(),
            owners: ListState::default(),
            component_view: false,
            modal: Modal::None,
            cycle: 0,
            pointer: None,
            shown_item: None,
            selected_at: 0.0,
            pending_select: None,
        }
    }

    fn selected<'a>(&self, inventory: &'a Inventory) -> Option<&'a InventoryItem> {
        let listed = inventory.listed(self.category);
        listed.get(self.list.selected()?).copied()
    }

    /// Keeps the selection inside the list: the requested row, else the first shown one.
    fn settle_selection(&mut self, view: &ListView<'_>) {
        self.list.invalidate(view);
        if let Some(row) = self.pending_select.take() {
            self.list.select(view, row);
        } else if self.list.selected().is_none() {
            self.list.select(view, self.list.scroll());
        }
    }

    fn handle_modal(&mut self, input: NavInput, cx: &mut TabContext<'_>) {
        match &mut self.modal {
            Modal::None => {}
            Modal::Quantity(quantity) => match self.layout.quantity.handle(quantity, input) {
                QuantityOutcome::Open => {}
                QuantityOutcome::Drop { item, count } => {
                    cx.actions.push(PipBoyAction::Drop { item, count });
                    self.modal = Modal::None;
                }
                QuantityOutcome::Cancel => self.modal = Modal::None,
            },
            Modal::Favourites(menu) => {
                let key = Action::Favourite.key(self.category);
                match self.layout.favourites.handle(menu, input, key) {
                    FavouritesOutcome::Open => {}
                    FavouritesOutcome::Assign { item, slot } => {
                        cx.actions.push(PipBoyAction::SetFavourite {
                            item,
                            slot: Some(slot),
                        });
                        if menu.closing_at.is_none() {
                            menu.closing_at =
                                Some(cx.seconds_open + self.layout.favourites.close_delay_seconds);
                        }
                    }
                    FavouritesOutcome::Close => self.modal = Modal::None,
                }
            }
        }
    }

    fn handle_components(&mut self, input: NavInput, cx: &mut TabContext<'_>) -> InputResult {
        let inventory = &cx.snapshot.model.inventory;
        let totals = components::totals(inventory);
        let layout = Arc::clone(&self.layout);
        let view = ListView::new(&layout.components, totals.len());
        let selected = self
            .components
            .selected()
            .and_then(|index| totals.get(index));
        match input {
            NavInput::Hint(key) => match Action::of_key(self.category, key) {
                Some(Action::ComponentView) => self.component_view = false,
                Some(Action::Favourite) => {
                    if let Some(component) = selected {
                        cx.actions.push(PipBoyAction::ToggleComponentSearchTag(
                            component.name.clone(),
                        ));
                    }
                }
                _ => return InputResult::Ignored,
            },
            NavInput::Scrolled(_) if self.over_owners() => {
                let owners = components::owners(inventory, selected.map_or("", |c| &c.name));
                let owner_view = ListView::new(&layout.owners, owners.len());
                self.owners.handle(input, &owner_view);
            }
            NavInput::Up | NavInput::Down | NavInput::Scrolled(_) => {
                if self.components.handle(input, &view) == ListEvent::Moved {
                    self.owners = ListState::default();
                }
            }
            NavInput::PointerMoved(at) | NavInput::Clicked(at) => {
                self.pointer = Some(at);
                if self.components.handle(input, &view) == ListEvent::Moved {
                    self.owners = ListState::default();
                }
            }
            _ => return InputResult::Ignored,
        }
        InputResult::Consumed
    }

    fn over_owners(&self) -> bool {
        self.pointer
            .is_some_and(|at| self.layout.owners_area.contains(at))
    }

    fn start_drop(&mut self, item: &InventoryItem, cx: &mut TabContext<'_>) {
        if item.count > self.layout.drop_threshold {
            self.modal = Modal::Quantity(self.layout.quantity.new_quantity(item.id, item.count));
        } else {
            cx.actions.push(PipBoyAction::Drop {
                item: item.id,
                count: 1,
            });
        }
    }

    fn act(&mut self, action: Action, item: Option<&InventoryItem>, cx: &mut TabContext<'_>) {
        let inventory = &cx.snapshot.model.inventory;
        match (action, item) {
            (Action::Inspect, Some(item)) => {
                cx.actions.push(PipBoyAction::RequestInspect(item.id));
            }
            (Action::Drop, Some(item)) => self.start_drop(item, cx),
            (Action::Favourite, Some(item))
                if favourites_on(self.category) && native::can_favourite(item) =>
            {
                self.modal = Modal::Favourites(Favourites::new(item.id));
            }
            (Action::ComponentView, Some(_)) => {
                self.component_view = true;
                self.components = ListState::default();
                self.owners = ListState::default();
            }
            (Action::Sort, _) => cx.actions.push(PipBoyAction::SetSortMode {
                category: self.category,
                mode: next_sort_mode(self.category, inventory.sort[self.category as usize]),
            }),
            (Action::CycleDamage, _) => {
                self.cycle = (self.cycle + 1) % self.layout.doll.cycle_len();
            }
            _ => {}
        }
    }

    fn handle_items(&mut self, input: NavInput, cx: &mut TabContext<'_>) -> InputResult {
        let inventory = &cx.snapshot.model.inventory;
        let listed = inventory.listed(self.category);
        let layout = Arc::clone(&self.layout);
        let view = ListView::new(&layout.items, listed.len());
        self.settle_selection(&view);
        let selected = self
            .list
            .selected()
            .and_then(|index| listed.get(index))
            .copied();
        match input {
            NavInput::Hint(HintKey::Enter | HintKey::E) => {
                cx.actions.extend(selected.and_then(accept_action));
            }
            NavInput::Hint(key) => {
                let Some(action) = Action::of_key(self.category, key) else {
                    return InputResult::Ignored;
                };
                self.act(action, selected, cx);
            }
            NavInput::PointerMoved(at) => {
                self.pointer = Some(at);
                self.list.handle(input, &view);
            }
            NavInput::Clicked(at) => {
                self.pointer = Some(at);
                if let ListEvent::Activated(index) = self.list.handle(input, &view) {
                    cx.actions.extend(accept_action(listed[index]));
                }
            }
            NavInput::Up | NavInput::Down | NavInput::Scrolled(_) => {
                self.list.handle(input, &view);
            }
            _ => return InputResult::Ignored,
        }
        InputResult::Consumed
    }

    fn draw_items(&mut self, cx: &mut TabContext<'_>) {
        let snapshot = *cx.snapshot;
        let inventory = &snapshot.model.inventory;
        let layout = Arc::clone(&self.layout);
        let listed = inventory.listed(self.category);
        let view = ListView::new(&layout.items, listed.len());
        self.settle_selection(&view);
        let mut icons: Vec<(Icon, Pos2, egui::Color32)> = Vec::new();
        let ui_layout = cx.layout;
        self.list.paint(cx.painter, cx.tint, &view, |painter, row| {
            let item = listed[row.index];
            let shown = list_entry_text(
                &layout.items.rules,
                ui_layout,
                &layout.suffix,
                &item.name,
                item.count,
            );
            let flags = EntryFlags {
                legendary: item.legendary,
                favourite: item.favourite.is_some(),
                search: components::has_tagged_component(inventory, item),
                equipped: item.equipped,
            };
            icons.extend(
                layout
                    .item_icons
                    .paint_row(painter, &layout.items, row, &shown, flags),
            );
        });
        self.list.paint_arrows(cx, &view);
        for (icon, at, colour) in icons {
            cx.draw_icon(icon, at, 1.0, colour);
        }
        let Some(item) = self
            .list
            .selected()
            .and_then(|index| listed.get(index))
            .copied()
        else {
            return;
        };
        if self.shown_item != Some(item.id) {
            self.shown_item = Some(item.id);
            self.selected_at = cx.seconds_open;
        }
        if self.category == InventoryCategory::Apparel {
            layout.doll.draw(
                cx,
                &DollView {
                    inventory,
                    selected: item,
                    underwear: native::UNDERWEAR_TYPE,
                    cycle: self.cycle,
                    pulse_seconds: cx.seconds_open - self.selected_at,
                },
            );
        }
        let entries = card_model::entries(item, counterpart(inventory, item), inventory);
        let rules = &layout.items.rules;
        let placed = card::plan(cx.painter, cx.layout, &layout.card, rules, &entries);
        card::draw(cx, &layout.card, rules, &entries, &placed);
    }

    fn draw_components(&mut self, cx: &mut TabContext<'_>) {
        let snapshot = *cx.snapshot;
        let inventory = &snapshot.model.inventory;
        let layout = Arc::clone(&self.layout);
        let totals = components::totals(inventory);
        let view = ListView::new(&layout.components, totals.len());
        self.components.invalidate(&view);
        if self.components.selected().is_none() && !totals.is_empty() {
            self.components.select(&view, 0);
        }
        let mut icons: Vec<(Icon, Pos2, egui::Color32)> = Vec::new();
        let ui_layout = cx.layout;
        self.components
            .paint(cx.painter, cx.tint, &view, |painter, row| {
                let component = &totals[row.index];
                let shown = list_entry_text(
                    &layout.components.rules,
                    ui_layout,
                    &layout.suffix,
                    &component.name,
                    component.total,
                );
                let flags = EntryFlags {
                    search: inventory.tagged_components.contains(&component.name),
                    ..EntryFlags::default()
                };
                icons.extend(layout.component_icons.paint_row(
                    painter,
                    &layout.components,
                    row,
                    &shown,
                    flags,
                ));
            });
        self.components.paint_arrows(cx, &view);
        for (icon, at, colour) in icons {
            cx.draw_icon(icon, at, 1.0, colour);
        }
        let Some(selected) = self
            .components
            .selected()
            .and_then(|index| totals.get(index))
        else {
            return;
        };
        let owners = components::owners(inventory, &selected.name);
        let owner_view = ListView::new(&layout.owners, owners.len());
        self.owners.invalidate(&owner_view);
        self.owners
            .paint(cx.painter, cx.tint, &owner_view, |painter, row| {
                let (item, amount) = owners[row.index];
                let shown = layout
                    .owners
                    .entry_text(ui_layout, &native::owner_text(&item.name, amount));
                layout.owners.paint_text(painter, row, &shown);
            });
        self.owners.paint_arrows(cx, &owner_view);
    }

    fn close_expired_prompt(&mut self, seconds_open: f64) {
        if let Modal::Favourites(menu) = self.modal
            && menu.closing_at.is_some_and(|at| seconds_open >= at)
        {
            self.modal = Modal::None;
        }
    }

    fn hints_for_items(&self, inventory: &Inventory) -> Vec<ButtonHint> {
        let labels = &self.layout.hints;
        let selected = self.selected(inventory);
        let category = self.category;
        let key = |action: Action| action.key(category);
        let mut hints = Vec::new();
        let mut push = |action: Action, label: &str, enabled: bool| {
            if let Some(key) = key(action) {
                hints.push(ButtonHint::new(key, label).enabled(enabled));
            }
        };
        push(Action::Inspect, &labels.inspect, selected.is_some());
        push(Action::Drop, &labels.drop, selected.is_some());
        if category == InventoryCategory::Apparel {
            push(Action::CycleDamage, &labels.cycle_damage, true);
        }
        if category == InventoryCategory::Junk && selected.is_some() {
            push(Action::ComponentView, &labels.component_view, true);
        }
        if favourites_on(category) {
            push(
                Action::Favourite,
                &labels.favourite,
                selected.is_some_and(native::can_favourite),
            );
        }
        push(
            Action::Sort,
            labels.sort(inventory.sort[category as usize]),
            true,
        );
        hints
    }

    fn hints_for_components(&self, inventory: &Inventory) -> Vec<ButtonHint> {
        let labels = &self.layout.hints;
        let has_component = !components::totals(inventory).is_empty();
        let mut hints = Vec::new();
        if let Some(key) = Action::Favourite.key(self.category) {
            hints.push(ButtonHint::new(key, &labels.tag_for_search).enabled(has_component));
        }
        if let Some(key) = Action::ComponentView.key(self.category) {
            hints.push(ButtonHint::new(key, &labels.item_view));
        }
        hints
    }
}

impl Tab for InventoryPage {
    fn handle_input(&mut self, input: NavInput, cx: &mut TabContext<'_>) -> InputResult {
        if self.modal != Modal::None {
            self.handle_modal(input, cx);
            return InputResult::Consumed;
        }
        if self.component_view {
            self.handle_components(input, cx)
        } else {
            self.handle_items(input, cx)
        }
    }

    fn draw(&mut self, cx: &mut TabContext<'_>) {
        self.close_expired_prompt(cx.seconds_open);
        if self.component_view {
            self.draw_components(cx);
        } else {
            self.draw_items(cx);
        }
    }

    fn draw_overlay(&mut self, cx: &mut TabContext<'_>) {
        let layout = Arc::clone(&self.layout);
        let snapshot = *cx.snapshot;
        match &self.modal {
            Modal::None => {}
            Modal::Quantity(quantity) => {
                layout.dim.paint(cx.painter, cx.tint);
                layout.quantity.draw(cx, quantity);
            }
            Modal::Favourites(menu) => {
                layout.dim.paint(cx.painter, cx.tint);
                layout
                    .favourites
                    .draw(cx, menu, &snapshot.model.inventory, &layout.items.rules);
            }
        }
    }

    fn hints(&self, snapshot: &PipBoySnapshot<'_>) -> Vec<ButtonHint> {
        let inventory = &snapshot.model.inventory;
        if self.modal != Modal::None {
            let labels = &self.layout.hints;
            return vec![
                ButtonHint::new(HintKey::Enter, &labels.accept),
                ButtonHint::new(HintKey::Tab, &labels.cancel),
            ];
        }
        if self.component_view {
            self.hints_for_components(inventory)
        } else {
            self.hints_for_items(inventory)
        }
    }

    fn bottom_bar(&self) -> BottomBar {
        BottomBar::Inventory(self.category)
    }

    fn on_open(&mut self) {
        self.modal = Modal::None;
        self.component_view = false;
    }

    fn is_modal(&self) -> bool {
        self.modal != Modal::None
    }

    fn select_row(&mut self, row: usize) {
        self.pending_select = Some(row);
    }
}

#[cfg(test)]
mod tests;
