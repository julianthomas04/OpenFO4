//! The JUNK component view: every component the carried junk holds, and which items hold it.
//! Both lists sort by their text, case-sensitively, as the game sorts them.

use crate::sim::pipboy::inventory::{InventoryItem, ItemDetails};
use crate::sim::pipboy::{Inventory, InventoryCategory};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentTotal {
    pub name: String,
    pub total: u32,
}

/// Components over all junk stacks (`count x amount`), sorted by name.
pub fn totals(inventory: &Inventory) -> Vec<ComponentTotal> {
    let mut totals: Vec<ComponentTotal> = Vec::new();
    for item in junk(inventory) {
        let ItemDetails::Junk(components) = &item.details else {
            continue;
        };
        for amount in components {
            let gained = amount.count * item.count;
            match totals.iter_mut().find(|t| t.name == amount.component) {
                Some(existing) => existing.total += gained,
                None => totals.push(ComponentTotal {
                    name: amount.component.clone(),
                    total: gained,
                }),
            }
        }
    }
    totals.sort_by(|a, b| a.name.cmp(&b.name));
    totals
}

fn junk(inventory: &Inventory) -> impl Iterator<Item = &InventoryItem> {
    inventory
        .items
        .iter()
        .filter(|item| item.category == InventoryCategory::Junk)
}

/// Junk stacks holding `component`, with the amount each stack yields.
pub fn owners<'a>(inventory: &'a Inventory, component: &str) -> Vec<(&'a InventoryItem, u32)> {
    let mut owners: Vec<(&InventoryItem, u32)> = junk(inventory)
        .filter_map(|item| match &item.details {
            ItemDetails::Junk(list) => list
                .iter()
                .find(|c| c.component == component)
                .map(|c| (item, c.count * item.count)),
            _ => None,
        })
        .collect();
    owners.sort_by(|(a, _), (b, _)| a.name.cmp(&b.name));
    owners
}

/// Whether `item` holds a component the player tagged for search.
pub fn has_tagged_component(inventory: &Inventory, item: &InventoryItem) -> bool {
    match &item.details {
        ItemDetails::Junk(list) => list
            .iter()
            .any(|c| inventory.tagged_components.contains(&c.component)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::pipboy::inventory::{ComponentAmount, ItemId};

    fn junk_item(id: u32, name: &str, count: u32, parts: &[(&str, u32)]) -> InventoryItem {
        InventoryItem {
            id: ItemId(id),
            form_key: None,
            name: name.into(),
            category: InventoryCategory::Junk,
            count,
            weight: 1.0,
            value: 1,
            equipped: false,
            favourite: None,
            legendary: false,
            details: ItemDetails::Junk(
                parts
                    .iter()
                    .map(|(c, n)| ComponentAmount {
                        component: (*c).into(),
                        count: *n,
                    })
                    .collect(),
            ),
        }
    }

    fn inventory() -> Inventory {
        Inventory {
            items: vec![
                junk_item(1, "item-b", 2, &[("part-b", 3)]),
                junk_item(2, "item-a", 4, &[("part-a", 1), ("part-b", 1)]),
            ],
            tagged_components: vec!["part-a".into()],
            ..Inventory::default()
        }
    }

    #[test]
    fn totals_multiply_by_stack_size_and_merge_across_stacks() {
        let totals = totals(&inventory());
        assert_eq!(
            totals,
            vec![
                ComponentTotal {
                    name: "part-a".into(),
                    total: 4
                },
                ComponentTotal {
                    name: "part-b".into(),
                    total: 10
                },
            ]
        );
    }

    #[test]
    fn owners_list_the_stacks_with_their_yield() {
        let inv = inventory();
        let owners = owners(&inv, "part-b");
        let named: Vec<_> = owners.iter().map(|(i, n)| (i.name.as_str(), *n)).collect();
        assert_eq!(named, [("item-a", 4), ("item-b", 6)]);
        assert_eq!(super::owners(&inv, "part-c"), []);
    }

    #[test]
    fn a_stack_is_tagged_when_any_component_is() {
        let inv = inventory();
        assert!(has_tagged_component(&inv, &inv.items[1]));
        assert!(!has_tagged_component(&inv, &inv.items[0]));
    }
}
