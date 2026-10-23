//! What an item card shows for an item: the entries the game's native code builds for the menu,
//! in the order it builds them.

use super::native::{
    self, ACCURACY, CHIP_PRECISION, FIRE_RATE, HEALTH, RANGE, StatSpec, VALUE, WEIGHT,
};
use crate::sim::pipboy::inventory::{
    AidEffect, ApparelStats, DamageValue, InventoryItem, ItemDetails, WeaponStats,
};
use crate::sim::pipboy::{DamageType, Inventory, InventoryCategory};

/// A number with the decimals it keeps, and how it compares with the equipped counterpart.
#[derive(Clone, Debug, PartialEq)]
pub struct StatLine {
    pub label: String,
    pub value: f32,
    pub precision: u8,
    pub percent: bool,
    pub comparison: Option<Comparison>,
}

/// The difference to the equipped counterpart and its rating `-3..=3`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Comparison {
    pub difference: f32,
    pub rating: i8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultiKind {
    Damage,
    Resist,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chip {
    pub kind: DamageType,
    pub value: f32,
    pub comparison: Option<Comparison>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ComponentLine {
    pub name: String,
    pub count: u32,
    pub tagged: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CardEntry {
    Standard(StatLine),
    Timed(StatLine),
    Ammo { name: String, carried: u32 },
    Multi { kind: MultiKind, chips: Vec<Chip> },
    Components(Vec<ComponentLine>),
    Description(String),
}

/// Whether a difference of `difference` shows at `precision` decimals.
pub fn difference_shows(difference: f32, precision: u8) -> bool {
    difference.abs() >= 10.0_f32.powi(-i32::from(precision))
}

/// A number as the card prints it: no rounding, only the decimals `precision` allows, and a
/// percent sign when it is one.
pub fn value_text(value: f32, precision: u8, percent: bool) -> String {
    let mut text = value.to_string();
    if let Some(dot) = text.find('.') {
        let keep = if precision > 0 {
            (dot + usize::from(precision) + 1).min(text.len())
        } else {
            dot
        };
        text.truncate(keep);
    }
    if percent {
        text.push('%');
    }
    text
}

fn comparison(own: f32, other: Option<f32>, precision: u8) -> Option<Comparison> {
    let other = other?;
    let difference = own - other;
    difference_shows(difference, precision).then(|| Comparison {
        difference,
        rating: native::diff_rating(own, other),
    })
}

fn stat(spec: StatSpec, value: f32, other: Option<f32>) -> CardEntry {
    CardEntry::Standard(StatLine {
        label: spec.label.to_owned(),
        value,
        precision: spec.precision,
        percent: false,
        comparison: comparison(value, other, spec.precision),
    })
}

fn amount_of(values: &[DamageValue], kind: DamageType) -> f32 {
    values
        .iter()
        .filter(|value| value.kind == kind)
        .map(|value| value.amount)
        .sum()
}

/// The chips of a multi entry: every type the item has a value of, and for resistances also
/// those whose difference shows.
fn chips(
    values: &[DamageValue],
    other: Option<&[DamageValue]>,
    keep_shown_differences: bool,
) -> Vec<Chip> {
    let mut chips = Vec::new();
    for kind in DamageType::ALL {
        let value = amount_of(values, kind);
        let comparison = comparison(value, other.map(|o| amount_of(o, kind)), CHIP_PRECISION);
        if value > 0.0 || (keep_shown_differences && comparison.is_some()) {
            chips.push(Chip {
                kind,
                value,
                comparison,
            });
        }
    }
    chips
}

fn multi(kind: MultiKind, chips: Vec<Chip>) -> Option<CardEntry> {
    (!chips.is_empty()).then_some(CardEntry::Multi { kind, chips })
}

pub fn carried(inventory: &Inventory, ammo: &str) -> u32 {
    inventory
        .items
        .iter()
        .filter(|item| item.category == InventoryCategory::Ammo && item.name == ammo)
        .map(|item| item.count)
        .sum()
}

fn weapon_entries(
    weapon: &WeaponStats,
    other: Option<&WeaponStats>,
    inventory: &Inventory,
) -> Vec<CardEntry> {
    let mut entries = Vec::new();
    entries.extend(multi(
        MultiKind::Damage,
        chips(&weapon.damage, other.map(|o| o.damage.as_slice()), false),
    ));
    if let Some(name) = &weapon.ammo_name {
        entries.push(CardEntry::Ammo {
            name: name.clone(),
            carried: carried(inventory, name),
        });
    }
    entries.push(stat(
        FIRE_RATE,
        weapon.fire_rate,
        other.map(|o| o.fire_rate),
    ));
    entries.push(stat(RANGE, weapon.range, other.map(|o| o.range)));
    entries.push(stat(ACCURACY, weapon.accuracy, other.map(|o| o.accuracy)));
    entries
}

fn apparel_entries(apparel: &ApparelStats, other: Option<&ApparelStats>) -> Vec<CardEntry> {
    multi(
        MultiKind::Resist,
        chips(
            &apparel.resistances,
            other.map(|o| o.resistances.as_slice()),
            true,
        ),
    )
    .into_iter()
    .collect()
}

fn aid_entries(effects: &[AidEffect]) -> Vec<CardEntry> {
    let mut entries = Vec::new();
    for effect in effects {
        if effect.restores_hp > 0.0 {
            let line = StatLine {
                label: HEALTH.label.to_owned(),
                value: effect.restores_hp,
                precision: HEALTH.precision,
                percent: false,
                comparison: None,
            };
            entries.push(if effect.duration_seconds.is_some_and(|d| d > 0.0) {
                CardEntry::Timed(line)
            } else {
                CardEntry::Standard(line)
            });
        } else {
            entries.push(CardEntry::Description(effect.description.clone()));
        }
    }
    entries
}

/// The entries of `item` in the order the game's menu receives them: the damage and resistance
/// entries and the description are placed by their own rules, the others stack upward from the
/// last.
pub fn entries(
    item: &InventoryItem,
    counterpart: Option<&InventoryItem>,
    inventory: &Inventory,
) -> Vec<CardEntry> {
    let mut entries = match (&item.details, counterpart.map(|c| &c.details)) {
        (ItemDetails::Weapon(weapon), other) => {
            let other = match other {
                Some(ItemDetails::Weapon(other)) => Some(other),
                _ => None,
            };
            weapon_entries(weapon, other, inventory)
        }
        (ItemDetails::Apparel(apparel), other) => {
            let other = match other {
                Some(ItemDetails::Apparel(other)) => Some(other),
                _ => None,
            };
            apparel_entries(apparel, other)
        }
        (ItemDetails::Aid(effects), _) => aid_entries(effects),
        (ItemDetails::Junk(components), _) => vec![CardEntry::Components(
            components
                .iter()
                .map(|component| ComponentLine {
                    name: component.component.clone(),
                    count: component.count,
                    tagged: inventory.tagged_components.contains(&component.component),
                })
                .collect(),
        )],
        (ItemDetails::Misc | ItemDetails::Mod | ItemDetails::Ammo, _) => Vec::new(),
    };
    entries.push(stat(WEIGHT, item.weight, None));
    entries.push(stat(VALUE, item.value as f32, None));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::pipboy::inventory::{ComponentAmount, ItemId};

    fn item(details: ItemDetails) -> InventoryItem {
        InventoryItem {
            id: ItemId(0),
            form_key: None,
            name: "thing".into(),
            category: InventoryCategory::Weapons,
            count: 1,
            weight: 2.25,
            value: 7,
            equipped: false,
            favourite: None,
            legendary: false,
            details,
        }
    }

    fn weapon(damage: f32, fire_rate: f32) -> ItemDetails {
        ItemDetails::Weapon(WeaponStats {
            damage: vec![DamageValue {
                kind: DamageType::Physical,
                amount: damage,
            }],
            fire_rate,
            range: 8.0,
            accuracy: 3.0,
            ammo_name: None,
        })
    }

    #[test]
    fn values_truncate_to_their_decimals_and_never_round() {
        assert_eq!(value_text(6.69, 1, false), "6.6");
        assert_eq!(value_text(6.69, 0, false), "6");
        assert_eq!(value_text(6.0, 2, false), "6");
        assert_eq!(value_text(0.5, 0, true), "0%");
        assert_eq!(value_text(12.0, 0, false), "12");
    }

    #[test]
    fn a_difference_shows_from_one_unit_of_the_last_decimal() {
        assert!(difference_shows(1.0, 0));
        assert!(!difference_shows(0.9, 0));
        assert!(difference_shows(-0.1, 1));
        assert!(!difference_shows(0.04, 1));
    }

    #[test]
    fn a_weapon_card_ends_in_weight_and_value_and_compares_against_the_equipped_one() {
        let inventory = Inventory::default();
        let own = item(weapon(20.0, 4.0));
        let other = item(weapon(10.0, 4.0));
        let entries = entries(&own, Some(&other), &inventory);
        let CardEntry::Multi { kind, chips } = &entries[0] else {
            panic!("the damage entry comes first");
        };
        assert_eq!(*kind, MultiKind::Damage);
        assert_eq!(chips[0].comparison.map(|c| c.rating), Some(3));
        let CardEntry::Standard(rate) = &entries[1] else {
            panic!("a plain row follows");
        };
        assert_eq!(rate.comparison, None);
        let CardEntry::Standard(last) = entries.last().unwrap() else {
            panic!("the value row is last");
        };
        assert_eq!(last.label, VALUE.label);
    }

    #[test]
    fn an_armour_chip_with_no_value_stays_when_its_difference_shows() {
        let own = ApparelStats {
            resistances: Vec::new(),
            covers: Vec::new(),
            slot_mask: 0,
        };
        let other = ApparelStats {
            resistances: vec![DamageValue {
                kind: DamageType::Energy,
                amount: 5.0,
            }],
            ..own.clone()
        };
        let entries = apparel_entries(&own, Some(&other));
        assert!(matches!(&entries[0], CardEntry::Multi { chips, .. } if chips.len() == 1));
        assert_eq!(apparel_entries(&own, None), []);
    }

    #[test]
    fn junk_lists_its_components_with_their_tag() {
        let mut inventory = Inventory::default();
        inventory.tagged_components.push("a".into());
        let junk = item(ItemDetails::Junk(vec![ComponentAmount {
            component: "a".into(),
            count: 2,
        }]));
        let entries = entries(&junk, None, &inventory);
        assert!(
            matches!(&entries[0], CardEntry::Components(lines) if lines[0].tagged && lines[0].count == 2)
        );
    }
}
