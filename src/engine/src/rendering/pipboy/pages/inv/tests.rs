use std::sync::Arc;

use super::*;
use crate::rendering::pipboy::pages::test_harness::Harness;
use crate::rendering::pipboy::test_layout::pipboy_layout;
use crate::sim::pipboy::inventory::{
    ApparelStats, ComponentAmount, DamageValue, FavouriteSlot, WeaponStats,
};
use crate::sim::pipboy::{DamageType, ItemId, PipBoyCatalog, PipBoyModel};

fn item(id: u32, category: InventoryCategory, count: u32, details: ItemDetails) -> InventoryItem {
    InventoryItem {
        id: ItemId(id),
        form_key: None,
        name: format!("item-{id}"),
        category,
        count,
        weight: 1.0,
        value: 1,
        equipped: false,
        favourite: None,
        legendary: false,
        details,
    }
}

fn weapon() -> ItemDetails {
    ItemDetails::Weapon(WeaponStats {
        damage: vec![DamageValue {
            kind: DamageType::Physical,
            amount: 5.0,
        }],
        fire_rate: 1.0,
        range: 1.0,
        accuracy: 1.0,
        ammo_name: None,
    })
}

fn apparel(mask: u64) -> ItemDetails {
    ItemDetails::Apparel(ApparelStats {
        resistances: Vec::new(),
        covers: Vec::new(),
        slot_mask: mask,
    })
}

fn model(items: Vec<InventoryItem>) -> PipBoyModel {
    let mut model = PipBoyModel::new(Arc::new(PipBoyCatalog::default()));
    model.inventory.items = items;
    model
}

fn page(category: InventoryCategory) -> InventoryPage {
    let layout = InventoryLayout::resolve(&pipboy_layout()).unwrap();
    InventoryPage::new(category, Arc::new(layout))
}

/// A page with its first row selected, as a frame leaves it.
fn drawn(category: InventoryCategory, harness: &mut Harness, model: &PipBoyModel) -> InventoryPage {
    let mut page = page(category);
    harness.draw(&mut page, model);
    page
}

fn hint_keys(page: &InventoryPage, model: &PipBoyModel) -> Vec<(HintKey, bool)> {
    let snapshot = PipBoySnapshot {
        model,
        vitals: crate::rendering::pipboy::pages::test_harness::vitals(),
    };
    page.hints(&snapshot)
        .into_iter()
        .map(|hint| (hint.key, hint.enabled))
        .collect()
}

#[test]
fn sorting_cycles_through_the_category_modes_and_wraps() {
    let weapons = InventoryCategory::Weapons;
    let modes = weapons.sort_modes();
    assert_eq!(next_sort_mode(weapons, modes[0]), modes[1]);
    assert_eq!(next_sort_mode(weapons, *modes.last().unwrap()), modes[0]);
}

#[test]
fn accepting_toggles_gear_uses_aid_and_ignores_the_rest() {
    let category = InventoryCategory::Weapons;
    let mut gear = item(1, category, 1, weapon());
    assert_eq!(accept_action(&gear), Some(PipBoyAction::Equip(ItemId(1))));
    gear.equipped = true;
    assert_eq!(accept_action(&gear), Some(PipBoyAction::Unequip(ItemId(1))));
    let aid = item(2, InventoryCategory::Aid, 1, ItemDetails::Aid(Vec::new()));
    assert_eq!(accept_action(&aid), Some(PipBoyAction::Use(ItemId(2))));
    let misc = item(3, InventoryCategory::Misc, 1, ItemDetails::Misc);
    assert_eq!(accept_action(&misc), None);
}

#[test]
fn the_counterpart_is_the_equipped_item_of_the_same_kind_and_slot() {
    let mut worn = item(3, InventoryCategory::Apparel, 1, apparel(0b01));
    worn.equipped = true;
    let mut other_slot = item(4, InventoryCategory::Apparel, 1, apparel(0b10));
    other_slot.equipped = true;
    let inventory = Inventory {
        items: vec![
            worn,
            other_slot,
            item(5, InventoryCategory::Apparel, 1, apparel(0b01)),
        ],
        ..Inventory::default()
    };
    assert_eq!(
        counterpart(&inventory, &inventory.items[2]).map(|i| i.id),
        Some(ItemId(3))
    );
    assert!(counterpart(&inventory, &inventory.items[0]).is_none());
}

#[test]
fn inspect_only_records_the_request() {
    let model = model(vec![item(1, InventoryCategory::Weapons, 1, weapon())]);
    let mut harness = Harness::new();
    let mut page = drawn(InventoryCategory::Weapons, &mut harness, &model);
    let key = Action::Inspect.key(InventoryCategory::Weapons).unwrap();
    harness.send(&mut page, &model, NavInput::Hint(key));
    assert_eq!(
        harness.take_actions(),
        [PipBoyAction::RequestInspect(ItemId(1))]
    );
    assert!(!page.is_modal());
}

#[test]
fn a_stack_up_to_the_threshold_drops_one_at_once_and_a_bigger_one_asks_how_many() {
    let mut harness = Harness::new();
    let model = model(vec![item(1, InventoryCategory::Weapons, 1, weapon())]);
    let mut page = drawn(InventoryCategory::Weapons, &mut harness, &model);
    let drop = Action::Drop.key(InventoryCategory::Weapons).unwrap();
    harness.send(&mut page, &model, NavInput::Hint(drop));
    assert_eq!(
        harness.take_actions(),
        [PipBoyAction::Drop {
            item: ItemId(1),
            count: 1
        }]
    );
    let stack = page.layout.drop_threshold + 1;
    let model = self::model(vec![item(2, InventoryCategory::Weapons, stack, weapon())]);
    let mut page = drawn(InventoryCategory::Weapons, &mut harness, &model);
    harness.send(&mut page, &model, NavInput::Hint(drop));
    assert!(page.is_modal());
    assert_eq!(harness.take_actions(), []);
    harness.send(&mut page, &model, NavInput::PrevSubTab);
    harness.send(&mut page, &model, NavInput::Hint(HintKey::Enter));
    assert_eq!(
        harness.take_actions(),
        [PipBoyAction::Drop {
            item: ItemId(2),
            count: stack - 1
        }]
    );
    assert!(!page.is_modal());
}

#[test]
fn cancelling_the_quantity_prompt_drops_nothing() {
    let mut harness = Harness::new();
    let stack = page(InventoryCategory::Weapons).layout.drop_threshold + 1;
    let model = model(vec![item(1, InventoryCategory::Weapons, stack, weapon())]);
    let mut page = drawn(InventoryCategory::Weapons, &mut harness, &model);
    let drop = Action::Drop.key(InventoryCategory::Weapons).unwrap();
    harness.send(&mut page, &model, NavInput::Hint(drop));
    harness.send(&mut page, &model, NavInput::Hint(HintKey::Tab));
    assert!(!page.is_modal());
    assert_eq!(harness.take_actions(), []);
}

#[test]
fn a_favourite_key_opens_the_menu_only_on_tabs_before_misc_and_for_items_that_can_be_one() {
    let mut harness = Harness::new();
    let key = Action::Favourite.key(InventoryCategory::Weapons).unwrap();
    let model = model(vec![item(1, InventoryCategory::Weapons, 1, weapon())]);
    let mut page = drawn(InventoryCategory::Weapons, &mut harness, &model);
    harness.send(&mut page, &model, NavInput::Hint(key));
    assert!(page.is_modal());

    let misc = self::model(vec![item(2, InventoryCategory::Misc, 1, ItemDetails::Misc)]);
    let mut page = drawn(InventoryCategory::Misc, &mut harness, &misc);
    harness.send(&mut page, &misc, NavInput::Hint(key));
    assert!(!page.is_modal());
}

#[test]
fn a_quick_key_assigns_the_item_and_the_menu_closes_after_its_delay() {
    let mut harness = Harness::new();
    let key = Action::Favourite.key(InventoryCategory::Weapons).unwrap();
    let model = model(vec![item(1, InventoryCategory::Weapons, 1, weapon())]);
    let mut page = drawn(InventoryCategory::Weapons, &mut harness, &model);
    harness.send(&mut page, &model, NavInput::Hint(key));
    harness.send(&mut page, &model, NavInput::Slot(3));
    assert_eq!(
        harness.take_actions(),
        [PipBoyAction::SetFavourite {
            item: ItemId(1),
            slot: Some(FavouriteSlot(3))
        }]
    );
    assert!(page.is_modal());
    harness.seconds_open += page.layout.favourites.close_delay_seconds + 0.01;
    harness.draw(&mut page, &model);
    assert!(!page.is_modal());
}

#[test]
fn the_favourite_key_closes_the_menu_it_opened() {
    let mut harness = Harness::new();
    let key = Action::Favourite.key(InventoryCategory::Weapons).unwrap();
    let model = model(vec![item(1, InventoryCategory::Weapons, 1, weapon())]);
    let mut page = drawn(InventoryCategory::Weapons, &mut harness, &model);
    harness.send(&mut page, &model, NavInput::Hint(key));
    harness.send(&mut page, &model, NavInput::Hint(key));
    assert!(!page.is_modal());
    assert_eq!(harness.take_actions(), []);
}

fn junk(id: u32) -> InventoryItem {
    item(
        id,
        InventoryCategory::Junk,
        2,
        ItemDetails::Junk(vec![ComponentAmount {
            component: "part".into(),
            count: 1,
        }]),
    )
}

#[test]
fn the_component_view_tags_components_and_leaves_with_the_same_key() {
    let mut harness = Harness::new();
    let model = model(vec![junk(1)]);
    let mut page = drawn(InventoryCategory::Junk, &mut harness, &model);
    let toggle = Action::ComponentView.key(InventoryCategory::Junk).unwrap();
    harness.send(&mut page, &model, NavInput::Hint(toggle));
    assert!(page.component_view);
    harness.draw(&mut page, &model);
    let tag = Action::Favourite.key(InventoryCategory::Junk).unwrap();
    harness.send(&mut page, &model, NavInput::Hint(tag));
    assert_eq!(
        harness.take_actions(),
        [PipBoyAction::ToggleComponentSearchTag("part".into())]
    );
    harness.send(&mut page, &model, NavInput::Hint(toggle));
    assert!(!page.component_view);
}

#[test]
fn the_apparel_hints_offer_cycle_damage_and_no_equip_hint_exists() {
    let mut harness = Harness::new();
    let model = model(vec![item(1, InventoryCategory::Apparel, 1, apparel(1))]);
    let page = drawn(InventoryCategory::Apparel, &mut harness, &model);
    let keys: Vec<HintKey> = hint_keys(&page, &model)
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    let cycle = Action::CycleDamage.key(InventoryCategory::Apparel).unwrap();
    assert!(keys.contains(&cycle));
    assert!(!keys.contains(&HintKey::E));
    assert!(!keys.contains(&HintKey::Enter));
}

#[test]
fn the_hints_follow_the_selection_and_the_prompt() {
    let mut harness = Harness::new();
    let empty = model(Vec::new());
    let idle = drawn(InventoryCategory::Weapons, &mut harness, &empty);
    assert!(hint_keys(&idle, &empty).iter().any(|(_, enabled)| !enabled));

    let stacked = model(vec![item(1, InventoryCategory::Weapons, 1, weapon())]);
    let mut page = drawn(InventoryCategory::Weapons, &mut harness, &stacked);
    assert!(
        hint_keys(&page, &stacked)
            .iter()
            .all(|(_, enabled)| *enabled)
    );
    let fav = Action::Favourite.key(InventoryCategory::Weapons).unwrap();
    harness.send(&mut page, &stacked, NavInput::Hint(fav));
    assert_eq!(
        hint_keys(&page, &stacked),
        [(HintKey::Enter, true), (HintKey::Tab, true)]
    );
}

#[test]
fn the_item_card_and_doll_draw_for_the_selection_without_panicking() {
    let mut harness = Harness::new();
    let mut worn = item(1, InventoryCategory::Apparel, 1, apparel(1));
    worn.equipped = true;
    let model = model(vec![
        worn,
        item(2, InventoryCategory::Apparel, 1, apparel(1)),
    ]);
    let mut page = drawn(InventoryCategory::Apparel, &mut harness, &model);
    harness.send(&mut page, &model, NavInput::Down);
    harness.draw(&mut page, &model);
    assert_eq!(page.shown_item, Some(ItemId(2)));
}

#[test]
#[ignore = "needs OPENFO4_INSTALL_DIR with the installer's layout documents"]
fn the_installed_layout_resolves_every_inventory_key() {
    let installed = crate::ui_layout::test_support::installed_layout();
    installed.validate(&[&LAYOUT_KEYS]).unwrap();
    let layout = InventoryLayout::resolve(&installed).unwrap();
    assert_eq!(layout.drop_threshold, 5);
    assert_eq!(layout.doll.cycle_len(), 3);
    assert!((layout.favourites.close_delay_seconds - 0.45).abs() < 1e-6);
    assert!(layout.owners_area.height() > 0.0);
    assert!(layout.hints.drop.chars().all(|c| c != '$'));
    assert_ne!(layout.hints.sort(SortMode::Weight), "");
    assert!(layout.card.spacing.is_finite());
}
