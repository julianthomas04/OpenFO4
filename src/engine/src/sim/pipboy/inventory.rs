//! The player's inventory as the INV page shows it.

use super::catalog::ItemDefinition;
use super::character::Limb;

/// Session-stable handle of one inventory stack, allocated by the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InventoryCategory {
    Weapons,
    Apparel,
    Aid,
    Misc,
    Junk,
    Mods,
    Ammo,
}

impl InventoryCategory {
    pub const ALL: [InventoryCategory; 7] = [
        InventoryCategory::Weapons,
        InventoryCategory::Apparel,
        InventoryCategory::Aid,
        InventoryCategory::Misc,
        InventoryCategory::Junk,
        InventoryCategory::Mods,
        InventoryCategory::Ammo,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DamageType {
    Physical,
    Energy,
    Radiation,
    Poison,
    Cryo,
    Fire,
}

impl DamageType {
    pub const ALL: [DamageType; 6] = [
        DamageType::Physical,
        DamageType::Energy,
        DamageType::Radiation,
        DamageType::Poison,
        DamageType::Cryo,
        DamageType::Fire,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DamageValue {
    pub kind: DamageType,
    pub amount: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WeaponStats {
    pub damage: Vec<DamageValue>,
    pub fire_rate: f32,
    pub range: f32,
    pub accuracy: f32,
    /// Display name of the ammunition the weapon fires; `None` for melee and unarmed.
    pub ammo_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ApparelStats {
    pub resistances: Vec<DamageValue>,
    /// Limbs the piece covers; drives the paper doll.
    pub covers: Vec<Limb>,
    /// Biped slot bit mask; two equipped pieces never share a bit.
    pub slot_mask: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AidEffect {
    pub description: String,
    /// Health restored on use, in hit points; `0` for effects that do not heal.
    pub restores_hp: f32,
    pub duration_seconds: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ComponentAmount {
    pub component: String,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ItemDetails {
    Weapon(WeaponStats),
    Apparel(ApparelStats),
    Aid(Vec<AidEffect>),
    Misc,
    Junk(Vec<ComponentAmount>),
    Mod,
    Ammo,
}

/// Number of favourites-wheel slots.
pub const FAVOURITE_SLOT_COUNT: u8 = 12;

/// One of the twelve favourites-wheel slots, `0..12`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FavouriteSlot(pub u8);

#[derive(Clone, Debug, PartialEq)]
pub struct InventoryItem {
    pub id: ItemId,
    /// Source record (`"00000A:Fallout4.esm"`); `None` for items with no record.
    pub form_key: Option<String>,
    pub name: String,
    pub category: InventoryCategory,
    pub count: u32,
    /// Weight of one unit.
    pub weight: f32,
    /// Value in caps of one unit.
    pub value: u32,
    pub equipped: bool,
    pub favourite: Option<FavouriteSlot>,
    pub legendary: bool,
    pub details: ItemDetails,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SortMode {
    #[default]
    Alphabetical,
    Damage,
    RateOfFire,
    Range,
    Accuracy,
    Value,
    Weight,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inventory {
    pub items: Vec<InventoryItem>,
    pub caps: u32,
    pub carry_capacity: f32,
    /// Sort mode per category, indexed by `InventoryCategory as usize`.
    pub sort: [SortMode; 7],
    /// Components tagged for search (JUNK component view).
    pub tagged_components: Vec<String>,
    /// The next id [`Inventory::insert`] hands out; ids are never reused after a stack goes away.
    pub(crate) next_item_id: u32,
}

impl InventoryCategory {
    /// The sort modes the category's list offers, in cycling order; the first is the default.
    pub fn sort_modes(self) -> &'static [SortMode] {
        const WEAPON_MODES: [SortMode; 7] = [
            SortMode::Alphabetical,
            SortMode::Damage,
            SortMode::RateOfFire,
            SortMode::Range,
            SortMode::Accuracy,
            SortMode::Value,
            SortMode::Weight,
        ];
        const COMMON_MODES: [SortMode; 3] =
            [SortMode::Alphabetical, SortMode::Value, SortMode::Weight];
        match self {
            InventoryCategory::Weapons => &WEAPON_MODES,
            _ => &COMMON_MODES,
        }
    }
}

impl SortMode {
    pub fn is_valid_for(self, category: InventoryCategory) -> bool {
        category.sort_modes().contains(&self)
    }
}

impl InventoryItem {
    fn weapon_stats(&self) -> Option<&WeaponStats> {
        match &self.details {
            ItemDetails::Weapon(stats) => Some(stats),
            _ => None,
        }
    }

    /// Sum of every damage component; `0` for anything that is not a weapon.
    pub fn total_damage(&self) -> f32 {
        self.weapon_stats()
            .map_or(0.0, |stats| stats.damage.iter().map(|d| d.amount).sum())
    }

    fn sort_metric(&self, mode: SortMode) -> f32 {
        match mode {
            SortMode::Alphabetical => 0.0,
            SortMode::Damage => self.total_damage(),
            SortMode::RateOfFire => self.weapon_stats().map_or(0.0, |s| s.fire_rate),
            SortMode::Range => self.weapon_stats().map_or(0.0, |s| s.range),
            SortMode::Accuracy => self.weapon_stats().map_or(0.0, |s| s.accuracy),
            SortMode::Value => self.value as f32,
            SortMode::Weight => self.weight,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InventoryError {
    #[error("cannot add zero units of {form_key}")]
    ZeroUnits { form_key: String },
    #[error("the stack of {form_key} would exceed {max} units")]
    CountOverflow { form_key: String, max: u32 },
}

impl Inventory {
    /// The items of `category` in the order the list shows them (the category's sort mode).
    /// Alphabetical ascending; every other mode descending by its metric, ties by name.
    pub fn listed(&self, category: InventoryCategory) -> Vec<&InventoryItem> {
        let mode = self.sort[category as usize];
        let mut listed: Vec<&InventoryItem> = self
            .items
            .iter()
            .filter(|item| item.category == category)
            .collect();
        listed.sort_by_cached_key(|item| (item.name.to_lowercase(), item.id));
        if mode != SortMode::Alphabetical {
            listed.sort_by(|a, b| b.sort_metric(mode).total_cmp(&a.sort_metric(mode)));
        }
        listed
    }

    /// Total weight carried, every stack counted.
    pub fn carried_weight(&self) -> f32 {
        self.items
            .iter()
            .map(|item| item.weight * item.count as f32)
            .sum()
    }

    /// Adds `item` as a new stack under a fresh id (`item.id` is ignored) and returns that id.
    pub fn insert(&mut self, mut item: InventoryItem) -> ItemId {
        let first_unused = self.items.iter().map(|i| i.id.0 + 1).max().unwrap_or(0);
        let id = ItemId(self.next_item_id.max(first_unused));
        self.next_item_id = id.0 + 1;
        item.id = id;
        self.items.push(item);
        id
    }

    /// Adds `count` units of `definition`: into the first stack of its `form_key` that is not
    /// `legendary` (leaving that stack's `equipped`, `favourite` and `legendary` as they are),
    /// else as a new stack. Returns the stack's id; the inventory is unchanged on `Err`.
    pub fn add_units(
        &mut self,
        definition: &ItemDefinition,
        count: u32,
    ) -> Result<ItemId, InventoryError> {
        if count == 0 {
            return Err(InventoryError::ZeroUnits {
                form_key: definition.form_key.clone(),
            });
        }
        let stack = self.items.iter_mut().find(|item| {
            item.form_key.as_deref() == Some(definition.form_key.as_str()) && !item.legendary
        });
        let Some(stack) = stack else {
            return Ok(self.insert(InventoryItem {
                id: ItemId(0),
                form_key: Some(definition.form_key.clone()),
                name: definition.name.clone(),
                category: definition.category,
                count,
                weight: definition.weight,
                value: definition.value,
                equipped: false,
                favourite: None,
                legendary: false,
                details: definition.details.clone(),
            }));
        };
        stack.count =
            stack
                .count
                .checked_add(count)
                .ok_or_else(|| InventoryError::CountOverflow {
                    form_key: definition.form_key.clone(),
                    max: u32::MAX,
                })?;
        Ok(stack.id)
    }

    pub fn item(&self, id: ItemId) -> Option<&InventoryItem> {
        self.items.iter().find(|item| item.id == id)
    }

    pub(super) fn item_mut(&mut self, id: ItemId) -> Option<&mut InventoryItem> {
        self.items.iter_mut().find(|item| item.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{aid, definition, weapon};
    use super::*;

    fn inventory_of(items: Vec<InventoryItem>) -> Inventory {
        let mut inventory = Inventory::default();
        for item in items {
            inventory.insert(item);
        }
        inventory
    }

    fn names(listed: &[&InventoryItem]) -> Vec<String> {
        listed.iter().map(|item| item.name.clone()).collect()
    }

    fn arsenal() -> Inventory {
        let mut heavy = weapon("Zeta Cannon", 90.0, 2.0);
        heavy.weight = 20.0;
        heavy.value = 500;
        let mut fast = weapon("alpha smg", 10.0, 90.0);
        fast.weight = 5.0;
        fast.value = 80;
        let mut balanced = weapon("Mid Rifle", 30.0, 20.0);
        balanced.weight = 8.0;
        balanced.value = 200;
        inventory_of(vec![heavy, fast, balanced, aid("Stimpak", 40.0, None)])
    }

    #[test]
    fn listing_is_alphabetical_ignoring_case_by_default_and_per_category() {
        let inventory = arsenal();
        assert_eq!(
            names(&inventory.listed(InventoryCategory::Weapons)),
            ["alpha smg", "Mid Rifle", "Zeta Cannon"]
        );
        assert_eq!(
            names(&inventory.listed(InventoryCategory::Aid)),
            ["Stimpak"]
        );
        assert_eq!(
            inventory.listed(InventoryCategory::Ammo),
            [] as [&InventoryItem; 0]
        );
    }

    #[test]
    fn metric_modes_sort_descending() {
        let mut inventory = arsenal();
        let expectations = [
            (SortMode::Damage, ["Zeta Cannon", "Mid Rifle", "alpha smg"]),
            (
                SortMode::RateOfFire,
                ["alpha smg", "Mid Rifle", "Zeta Cannon"],
            ),
            (SortMode::Weight, ["Zeta Cannon", "Mid Rifle", "alpha smg"]),
            (SortMode::Value, ["Zeta Cannon", "Mid Rifle", "alpha smg"]),
        ];
        for (mode, expected) in expectations {
            inventory.sort[InventoryCategory::Weapons as usize] = mode;
            assert_eq!(
                names(&inventory.listed(InventoryCategory::Weapons)),
                expected,
                "{mode:?}"
            );
        }
    }

    #[test]
    fn range_and_accuracy_modes_order_by_their_stat() {
        let mut short = weapon("Short", 1.0, 1.0);
        let mut long = weapon("Long", 1.0, 1.0);
        if let ItemDetails::Weapon(stats) = &mut short.details {
            stats.range = 10.0;
            stats.accuracy = 90.0;
        }
        if let ItemDetails::Weapon(stats) = &mut long.details {
            stats.range = 300.0;
            stats.accuracy = 20.0;
        }
        let mut inventory = inventory_of(vec![short, long]);
        inventory.sort[InventoryCategory::Weapons as usize] = SortMode::Range;
        assert_eq!(
            names(&inventory.listed(InventoryCategory::Weapons)),
            ["Long", "Short"]
        );
        inventory.sort[InventoryCategory::Weapons as usize] = SortMode::Accuracy;
        assert_eq!(
            names(&inventory.listed(InventoryCategory::Weapons)),
            ["Short", "Long"]
        );
    }

    #[test]
    fn equal_metrics_fall_back_to_alphabetical() {
        let mut inventory = inventory_of(vec![weapon("B", 5.0, 1.0), weapon("A", 5.0, 1.0)]);
        inventory.sort[InventoryCategory::Weapons as usize] = SortMode::Damage;
        assert_eq!(
            names(&inventory.listed(InventoryCategory::Weapons)),
            ["A", "B"]
        );
    }

    #[test]
    fn carried_weight_counts_every_unit_of_every_stack() {
        let mut stimpaks = aid("Stimpak", 40.0, None);
        stimpaks.count = 3;
        stimpaks.weight = 0.5;
        let mut rifle = weapon("Rifle", 1.0, 1.0);
        rifle.weight = 6.0;
        let inventory = inventory_of(vec![stimpaks, rifle]);
        assert!((inventory.carried_weight() - 7.5).abs() < f32::EPSILON);
        assert!(Inventory::default().carried_weight().abs() < f32::EPSILON);
    }

    #[test]
    fn insert_allocates_distinct_ids() {
        let mut inventory = Inventory::default();
        let first = inventory.insert(weapon("A", 1.0, 1.0));
        let second = inventory.insert(weapon("B", 1.0, 1.0));
        assert_ne!(first, second);
        assert_eq!(inventory.item(second).unwrap().name, "B");
    }

    #[test]
    fn an_id_is_not_reused_after_the_newest_stack_is_removed() {
        let mut inventory = Inventory::default();
        inventory.insert(weapon("A", 1.0, 1.0));
        let removed = inventory.insert(weapon("B", 1.0, 1.0));
        inventory.items.retain(|item| item.id != removed);
        let replacement = inventory.insert(weapon("C", 1.0, 1.0));
        assert_ne!(replacement, removed);
        assert!(inventory.item(removed).is_none());
    }

    #[test]
    fn insert_skips_ids_already_taken_by_stacks_placed_directly() {
        let mut inventory = Inventory::default();
        let mut placed = weapon("Placed", 1.0, 1.0);
        placed.id = ItemId(7);
        inventory.items.push(placed);
        assert_eq!(inventory.insert(weapon("New", 1.0, 1.0)), ItemId(8));
    }

    #[test]
    fn only_weapons_offer_the_weapon_stat_modes() {
        for category in InventoryCategory::ALL {
            let modes = category.sort_modes();
            assert_eq!(modes[0], SortMode::Alphabetical);
            assert!(modes.contains(&SortMode::Value) && modes.contains(&SortMode::Weight));
            assert_eq!(
                modes.contains(&SortMode::Damage),
                category == InventoryCategory::Weapons,
                "{category:?}"
            );
        }
        assert!(SortMode::Range.is_valid_for(InventoryCategory::Weapons));
        assert!(!SortMode::Range.is_valid_for(InventoryCategory::Aid));
    }

    const WRENCH: &str = "0AA001:Test.esm";

    fn stacks_of(inventory: &Inventory, form_key: &str) -> Vec<(u32, bool)> {
        inventory
            .items
            .iter()
            .filter(|item| item.form_key.as_deref() == Some(form_key))
            .map(|item| (item.count, item.legendary))
            .collect()
    }

    #[test]
    fn adding_a_new_item_creates_a_fresh_unequipped_stack_from_the_definition() {
        let mut inventory = Inventory::default();
        let mut wrench = definition(WRENCH, "Pipe Wrench");
        wrench.category = InventoryCategory::Junk;
        wrench.details = ItemDetails::Junk(vec![ComponentAmount {
            component: "Steel".to_owned(),
            count: 3,
        }]);

        let id = inventory.add_units(&wrench, 4).unwrap();

        let stack = inventory.item(id).unwrap();
        assert_eq!(stack.form_key.as_deref(), Some(WRENCH));
        assert_eq!(stack.name, "Pipe Wrench");
        assert_eq!(stack.category, InventoryCategory::Junk);
        assert_eq!(stack.count, 4);
        assert!((stack.weight - 2.0).abs() < f32::EPSILON && stack.value == 7);
        assert!(!stack.equipped && stack.favourite.is_none() && !stack.legendary);
        assert_eq!(stack.details, wrench.details);
        assert_eq!(inventory.items.len(), 1);
    }

    #[test]
    fn adding_units_of_a_held_item_grows_its_stack_and_keeps_equipped_and_favourite() {
        let mut inventory = Inventory::default();
        let wrench = definition(WRENCH, "Pipe Wrench");
        let id = inventory.add_units(&wrench, 2).unwrap();
        let stack = inventory.item_mut(id).unwrap();
        stack.equipped = true;
        stack.favourite = Some(FavouriteSlot(5));

        let merged = inventory.add_units(&wrench, 3).unwrap();

        assert_eq!(merged, id);
        let stack = inventory.item(id).unwrap();
        assert_eq!(stack.count, 5);
        assert!(stack.equipped);
        assert_eq!(stack.favourite, Some(FavouriteSlot(5)));
        assert_eq!(inventory.items.len(), 1);
    }

    #[test]
    fn a_legendary_stack_never_absorbs_units_but_a_plain_one_beside_it_does() {
        let mut inventory = Inventory::default();
        let wrench = definition(WRENCH, "Pipe Wrench");
        let legendary = inventory.add_units(&wrench, 1).unwrap();
        inventory.item_mut(legendary).unwrap().legendary = true;

        let plain = inventory.add_units(&wrench, 2).unwrap();
        assert_ne!(plain, legendary);
        let again = inventory.add_units(&wrench, 5).unwrap();

        assert_eq!(again, plain);
        assert_eq!(stacks_of(&inventory, WRENCH), [(1, true), (7, false)]);
    }

    #[test]
    fn merging_picks_the_first_plain_stack_of_the_same_record_only() {
        let mut inventory = Inventory::default();
        let other = inventory
            .add_units(&definition("0AA002:Test.esm", "Spanner"), 9)
            .unwrap();
        let first = inventory
            .add_units(&definition(WRENCH, "Pipe Wrench"), 1)
            .unwrap();
        let mut duplicate = weapon("Pipe Wrench", 1.0, 1.0);
        duplicate.form_key = Some(WRENCH.to_owned());
        inventory.insert(duplicate);

        let merged = inventory
            .add_units(&definition(WRENCH, "Pipe Wrench"), 1)
            .unwrap();

        assert_eq!(merged, first);
        assert_eq!(inventory.item(other).unwrap().count, 9);
        assert_eq!(stacks_of(&inventory, WRENCH), [(2, false), (1, false)]);
    }

    #[test]
    fn adding_zero_units_is_rejected_and_changes_nothing() {
        let mut inventory = Inventory::default();
        let wrench = definition(WRENCH, "Pipe Wrench");
        inventory.add_units(&wrench, 2).unwrap();
        let before = inventory.clone();

        let error = inventory.add_units(&wrench, 0).unwrap_err();

        assert_eq!(
            error,
            InventoryError::ZeroUnits {
                form_key: WRENCH.to_owned()
            }
        );
        assert_eq!(inventory, before);
        assert_eq!(Inventory::default().add_units(&wrench, 0), Err(error));
    }

    #[test]
    fn a_stack_that_would_overflow_is_rejected_and_unchanged() {
        let mut inventory = Inventory::default();
        let wrench = definition(WRENCH, "Pipe Wrench");
        inventory.add_units(&wrench, u32::MAX - 1).unwrap();
        let before = inventory.clone();

        let error = inventory.add_units(&wrench, 2).unwrap_err();

        assert_eq!(
            error,
            InventoryError::CountOverflow {
                form_key: WRENCH.to_owned(),
                max: u32::MAX
            }
        );
        assert_eq!(inventory, before);
        inventory.add_units(&wrench, 1).unwrap();
        assert_eq!(stacks_of(&inventory, WRENCH), [(u32::MAX, false)]);
    }
}
