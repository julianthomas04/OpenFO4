//! What taking a placed item asks of the sim, decided before anything changes. Pure.

use super::world_streamer::PlacedItem;
use crate::sim::EntityId;
use crate::sim::pipboy::PipBoyCatalog;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PickupPlan {
    pub entity: EntityId,
    pub base: String,
    pub count: u32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(super) enum PickupError {
    #[error("item base {base} has no catalog entry")]
    MissingCatalogEntry { base: String },
    #[error("placed item {entity:?} stands for zero units")]
    ZeroUnits { entity: EntityId },
}

/// The units and base of `item`, once the catalog knows the base and the item stands for at
/// least one unit.
pub(super) fn plan_pickup(
    item: &PlacedItem,
    catalog: &PipBoyCatalog,
) -> Result<PickupPlan, PickupError> {
    if catalog.item(&item.base).is_none() {
        return Err(PickupError::MissingCatalogEntry {
            base: item.base.clone(),
        });
    }
    if item.count == 0 {
        return Err(PickupError::ZeroUnits {
            entity: item.entity,
        });
    }
    Ok(PickupPlan {
        entity: item.entity,
        base: item.base.clone(),
        count: item.count,
    })
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::sim::pipboy::inventory::ItemDetails;
    use crate::sim::pipboy::{InventoryCategory, ItemDefinition, ItemKind};

    const WIDGET_KEY: &str = "0F3B01:Test.esm";

    fn item(base: &str, count: u32) -> PlacedItem {
        PlacedItem {
            entity: EntityId(77),
            base: base.to_owned(),
            count,
            owner: None,
            position: Vec3::ZERO,
        }
    }

    fn catalog() -> PipBoyCatalog {
        PipBoyCatalog {
            items: vec![ItemDefinition {
                form_key: WIDGET_KEY.to_owned(),
                kind: ItemKind::Misc,
                editor_id: None,
                name: "Brass Widget".to_owned(),
                category: InventoryCategory::Misc,
                weight: 1.0,
                value: 7,
                details: ItemDetails::Misc,
            }],
            ..PipBoyCatalog::default()
        }
    }

    #[test]
    fn a_known_item_plans_its_entity_base_and_whole_count() {
        assert_eq!(
            plan_pickup(&item(WIDGET_KEY, 12), &catalog()),
            Ok(PickupPlan {
                entity: EntityId(77),
                base: WIDGET_KEY.to_owned(),
                count: 12,
            })
        );
    }

    #[test]
    fn an_item_whose_base_the_catalog_lacks_is_refused() {
        let unknown = "0F3BFF:Test.esm";
        assert_eq!(
            plan_pickup(&item(unknown, 1), &catalog()),
            Err(PickupError::MissingCatalogEntry {
                base: unknown.to_owned()
            })
        );
    }

    #[test]
    fn an_item_standing_for_zero_units_is_refused() {
        assert_eq!(
            plan_pickup(&item(WIDGET_KEY, 0), &catalog()),
            Err(PickupError::ZeroUnits {
                entity: EntityId(77)
            })
        );
    }

    #[test]
    fn a_missing_catalog_entry_is_reported_before_the_zero_count() {
        assert!(matches!(
            plan_pickup(&item("0F3BFF:Test.esm", 0), &catalog()),
            Err(PickupError::MissingCatalogEntry { .. })
        ));
    }
}
