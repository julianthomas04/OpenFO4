//! Totals the bottom bar and the STATUS boxes show: damage of the equipped weapon and resistance
//! of the worn apparel, per damage type, in the order and numbering the game's widgets use.

use crate::assets::pipboy_art::Icon;
use crate::sim::pipboy::inventory::{ApparelStats, ItemDetails, WeaponStats};
use crate::sim::pipboy::{DamageType, Inventory};

pub fn damage_icon(kind: DamageType) -> Icon {
    match kind {
        DamageType::Physical => Icon::DamagePhysical,
        DamageType::Energy => Icon::DamageEnergy,
        DamageType::Radiation => Icon::DamageRadiation,
        DamageType::Poison => Icon::DamagePoison,
        DamageType::Cryo => Icon::DamageCryo,
        DamageType::Fire => Icon::DamageFire,
    }
}

pub fn damage_dealt_icon(kind: DamageType) -> Icon {
    match kind {
        DamageType::Physical => Icon::DamageDealtPhysical,
        DamageType::Energy => Icon::DamageDealtEnergy,
        DamageType::Radiation => Icon::DamageDealtRadiation,
        DamageType::Poison => Icon::DamageDealtPoison,
        DamageType::Cryo => Icon::DamageDealtCryo,
        DamageType::Fire => Icon::DamageDealtFire,
    }
}

/// The damage types in the game's numbering: type `n` is `TYPE_ORDER[n - 1]`, the order of the
/// damage icon clip's frames.
/// Stand-in: the healing the player's active effects will still give, as a fraction of the
/// maximum health (`CurrentHPGain`); the sim does not track healing over time yet.
pub const CURRENT_HP_GAIN: f32 = 0.0;

pub const TYPE_ORDER: [DamageType; 6] = [
    DamageType::Physical,
    DamageType::Poison,
    DamageType::Fire,
    DamageType::Energy,
    DamageType::Cryo,
    DamageType::Radiation,
];

/// The game's number of a damage type.
pub fn type_number(kind: DamageType) -> u32 {
    let index = TYPE_ORDER
        .iter()
        .position(|candidate| *candidate == kind)
        .expect("TYPE_ORDER lists every damage type");
    index as u32 + 1
}

/// The damage type the game numbers `number`.
pub fn numbered_type(number: u32) -> Option<DamageType> {
    let index = usize::try_from(number.checked_sub(1)?).ok()?;
    TYPE_ORDER.get(index).copied()
}

/// The art of frame `frame` of the damage icon clip: the resistance glyphs come first,
/// `types_per_set` of them, then the damage-dealt glyphs.
pub fn type_icon_frame(frame: u32, types_per_set: u32) -> Option<Icon> {
    if frame > types_per_set {
        numbered_type(frame - types_per_set).map(damage_dealt_icon)
    } else {
        numbered_type(frame).map(damage_icon)
    }
}

/// A damage or resistance total of one type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeTotal {
    pub kind: DamageType,
    pub amount: f32,
}

/// Sums `values` per type and lists the types with a positive total in the game's numbering.
/// Stand-in for the list the game hands its widgets, which it builds natively.
fn totals<'a>(
    values: impl Iterator<Item = &'a crate::sim::pipboy::inventory::DamageValue>,
) -> Vec<TypeTotal> {
    let mut sums = [0.0_f32; DamageType::ALL.len()];
    for value in values {
        sums[value.kind as usize] += value.amount;
    }
    TYPE_ORDER
        .into_iter()
        .filter(|kind| sums[*kind as usize] > 0.0)
        .map(|kind| TypeTotal {
            kind,
            amount: sums[kind as usize],
        })
        .collect()
}

fn equipped_weapons(inventory: &Inventory) -> impl Iterator<Item = &WeaponStats> {
    inventory
        .items
        .iter()
        .filter(|item| item.equipped)
        .filter_map(|item| match &item.details {
            ItemDetails::Weapon(stats) => Some(stats),
            _ => None,
        })
}

fn worn_apparel(inventory: &Inventory) -> impl Iterator<Item = &ApparelStats> {
    inventory
        .items
        .iter()
        .filter(|item| item.equipped)
        .filter_map(|item| match &item.details {
            ItemDetails::Apparel(stats) => Some(stats),
            _ => None,
        })
}

pub fn equipped_damage(inventory: &Inventory) -> Vec<TypeTotal> {
    totals(equipped_weapons(inventory).flat_map(|weapon| weapon.damage.iter()))
}

pub fn worn_resistance(inventory: &Inventory) -> Vec<TypeTotal> {
    totals(worn_apparel(inventory).flat_map(|apparel| apparel.resistances.iter()))
}

/// A value rounded to a whole number.
pub fn whole(value: f32) -> String {
    format!("{}", value.round() as i64)
}

/// `Math.floor(value).toString()`, as the game's widgets show their numbers.
pub fn floored(value: f32) -> String {
    format!("{}", value.floor() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::pipboy::inventory::{
        DamageValue, InventoryCategory, InventoryItem, ItemId, WeaponStats,
    };

    fn item(id: u32, equipped: bool, details: ItemDetails) -> InventoryItem {
        InventoryItem {
            id: ItemId(id),
            form_key: None,
            name: format!("item {id}"),
            category: InventoryCategory::Weapons,
            count: 1,
            weight: 1.0,
            value: 1,
            equipped,
            favourite: None,
            legendary: false,
            details,
        }
    }

    fn weapon(damage: &[(DamageType, f32)]) -> ItemDetails {
        ItemDetails::Weapon(WeaponStats {
            damage: damage
                .iter()
                .map(|(kind, amount)| DamageValue {
                    kind: *kind,
                    amount: *amount,
                })
                .collect(),
            fire_rate: 1.0,
            range: 1.0,
            accuracy: 1.0,
            ammo_name: None,
        })
    }

    fn apparel(resist: &[(DamageType, f32)]) -> ItemDetails {
        ItemDetails::Apparel(ApparelStats {
            resistances: resist
                .iter()
                .map(|(kind, amount)| DamageValue {
                    kind: *kind,
                    amount: *amount,
                })
                .collect(),
            covers: Vec::new(),
            slot_mask: 1,
        })
    }

    fn inventory(items: Vec<InventoryItem>) -> Inventory {
        Inventory {
            items,
            ..Inventory::default()
        }
    }

    #[test]
    fn only_equipped_weapons_count_and_types_sum_in_canonical_order() {
        let inv = inventory(vec![
            item(
                1,
                true,
                weapon(&[(DamageType::Energy, 12.0), (DamageType::Physical, 30.0)]),
            ),
            item(2, false, weapon(&[(DamageType::Fire, 99.0)])),
            item(3, true, apparel(&[(DamageType::Physical, 5.0)])),
        ]);

        let totals = equipped_damage(&inv);

        assert_eq!(
            totals,
            vec![
                TypeTotal {
                    kind: DamageType::Physical,
                    amount: 30.0
                },
                TypeTotal {
                    kind: DamageType::Energy,
                    amount: 12.0
                },
            ]
        );
    }

    #[test]
    fn nothing_equipped_lists_nothing() {
        assert_eq!(equipped_damage(&inventory(Vec::new())), []);
    }

    #[test]
    fn totals_follow_the_game_numbering() {
        let inv = inventory(vec![item(
            1,
            true,
            weapon(&[(DamageType::Radiation, 1.0), (DamageType::Poison, 2.0)]),
        )]);
        let kinds: Vec<_> = equipped_damage(&inv).iter().map(|t| t.kind).collect();
        assert_eq!(kinds, [DamageType::Poison, DamageType::Radiation]);
    }

    #[test]
    fn type_numbers_round_trip_and_start_at_one() {
        for kind in DamageType::ALL {
            assert_eq!(numbered_type(type_number(kind)), Some(kind));
        }
        assert_eq!(type_number(DamageType::Physical), 1);
        assert_eq!(numbered_type(0), None);
        assert_eq!(numbered_type(7), None);
    }

    #[test]
    fn icon_frames_past_one_set_are_the_damage_dealt_glyphs() {
        assert_eq!(type_icon_frame(1, 6), Some(Icon::DamagePhysical));
        assert_eq!(type_icon_frame(7, 6), Some(Icon::DamageDealtPhysical));
        assert_eq!(type_icon_frame(9, 6), Some(Icon::DamageDealtFire));
        assert_eq!(type_icon_frame(13, 6), None);
    }

    #[test]
    fn worn_pieces_add_their_resistances_per_type() {
        let inv = inventory(vec![
            item(
                1,
                true,
                apparel(&[(DamageType::Physical, 10.0), (DamageType::Radiation, 4.0)]),
            ),
            item(2, true, apparel(&[(DamageType::Physical, 7.0)])),
            item(3, false, apparel(&[(DamageType::Cryo, 50.0)])),
        ]);

        let totals = worn_resistance(&inv);

        assert_eq!(
            totals,
            vec![
                TypeTotal {
                    kind: DamageType::Physical,
                    amount: 17.0
                },
                TypeTotal {
                    kind: DamageType::Radiation,
                    amount: 4.0
                },
            ]
        );
    }

    #[test]
    fn values_round_to_whole_numbers() {
        assert_eq!(whole(41.5), "42");
        assert_eq!(whole(0.4), "0");
    }

    #[test]
    fn floored_values_drop_the_fraction() {
        assert_eq!(floored(41.9), "41");
        assert_eq!(floored(-0.5), "-1");
    }

    #[test]
    fn every_damage_type_has_its_own_icon() {
        let icons: std::collections::HashSet<_> =
            DamageType::ALL.into_iter().map(damage_icon).collect();
        assert_eq!(icons.len(), DamageType::ALL.len());
    }
}
