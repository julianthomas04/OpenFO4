//! Placed items of a cell and what the player changed about them while the cell is resident:
//! the item records the cell owns, the references taken out of the world, and how both reach
//! the collision registration and the instance rows.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use glam::Vec3;

use crate::rendering::door_prompt::non_blank;
use crate::rendering::row_pose::{PoseSource, RowPose};
use crate::sim::bake::BakedCellColliders;
use crate::sim::collision::CellColliderBatch;
use crate::sim::{CellGrid, EntityId, SimCommand};
use crate::utils::warn_once;

/// One pick-upable placed reference, as the cell recorded it at assembly.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlacedItem {
    pub entity: EntityId,
    /// Base item form key ("023736:Fallout4.esm"), the catalog key.
    pub base: String,
    /// Units the reference stands for: XCNT, else 1 (see `stack_count`).
    pub count: u32,
    /// Effective owner form key (NPC or faction), None = unowned.
    pub owner: Option<String>,
    /// Engine-space REFR origin; where a hidden row collapses to.
    pub position: Vec3,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(super) enum CountError {
    #[error("item count {0} is not positive")]
    NotPositive(i64),
    #[error("item count {0} does not fit a stack")]
    TooLarge(i64),
}

/// The units a placed reference stands for: its XCNT, one without.
pub(super) fn stack_count(item_count: Option<i64>) -> Result<u32, CountError> {
    let Some(count) = item_count else {
        return Ok(1);
    };
    if count <= 0 {
        return Err(CountError::NotPositive(count));
    }
    u32::try_from(count).map_err(|_| CountError::TooLarge(count))
}

/// The item record of a placed reference. A count the stack cannot hold is a data fault that
/// must not delete the item: the reference stands for one unit, logged once per reference.
pub(super) fn placed_item(
    entity: EntityId,
    base: &str,
    item_count: Option<i64>,
    owner: Option<&str>,
    position: Vec3,
) -> PlacedItem {
    let count = stack_count(item_count).unwrap_or_else(|error| {
        warn_once(&format!("placed-item-count:{entity:?}"), || {
            format!("placed item {entity:?} of {base}: {error}; it stands for one unit")
        });
        1
    });
    PlacedItem {
        entity,
        base: base.to_owned(),
        count,
        owner: owner.map(str::to_owned),
        position,
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum WorldItemError {
    #[error("entity {0:?} is not a placed item in a resident cell")]
    NotAnItem(EntityId),
}

/// How an item base enters the base-object lookup, from its marker flag and name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ItemBaseRole {
    /// A CK-only marker: never drawn, no body.
    Marker,
    /// The database cannot say whether it is a marker: treated as one, never a marker flood.
    UnknownMarker,
    /// A real but nameless item (test and chargen props): a plain loose body.
    Loose,
    /// A named item the player can take.
    Pickable,
}

pub(super) fn item_base_role(is_marker: Option<bool>, name: Option<&str>) -> ItemBaseRole {
    match is_marker {
        Some(true) => ItemBaseRole::Marker,
        None => ItemBaseRole::UnknownMarker,
        Some(false) if non_blank(name).is_some() => ItemBaseRole::Pickable,
        Some(false) => ItemBaseRole::Loose,
    }
}

/// The colliders a cell was assembled with: the shape of `CellResources.collider_payload`.
pub(super) type AuthoredColliders = (Arc<CellColliderBatch>, Option<BakedCellColliders>);

/// What the player changed in a resident cell. Belongs to the cell: unloading or demoting it
/// forgets the changes.
#[derive(Debug, Default)]
pub(super) struct CellEdits {
    /// Placed references taken out of the world; their bodies are never built again.
    pub removed: Arc<HashSet<EntityId>>,
}

/// The registration of a cell's colliders: as authored without edits, else without the
/// references taken out of the world.
pub(super) fn register_command(
    grid: CellGrid,
    authored: &AuthoredColliders,
    edits: &CellEdits,
) -> SimCommand {
    let (batch, baked) = authored;
    if edits.removed.is_empty() {
        SimCommand::AddCellColliders {
            grid,
            batch: Arc::clone(batch),
            baked: baked.clone(),
        }
    } else {
        SimCommand::AddCellCollidersSkipping {
            grid,
            batch: Arc::clone(batch),
            baked: baked.clone(),
            skip: Arc::clone(&edits.removed),
        }
    }
}

/// Takes the item behind `entity` out of the cell's records and remembers it as removed.
pub(super) fn take_item(
    items: &mut HashMap<EntityId, PlacedItem>,
    edits: &mut CellEdits,
    entity: EntityId,
) -> Option<PlacedItem> {
    let item = items.remove(&entity)?;
    Arc::make_mut(&mut edits.removed).insert(entity);
    Some(item)
}

/// A removed item whose instance rows still draw it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PendingHide {
    pub entity: EntityId,
    pub grid: CellGrid,
    /// The residency of the cell the item was taken from; a cell reloaded since is left alone.
    pub residency: u64,
    pub at: Vec3,
}

/// The hides `apply_pending_hides` has not written yet. Keeps its capacity across frames.
#[derive(Debug, Default)]
pub(super) struct PendingHides(Vec<PendingHide>);

impl PendingHides {
    pub(super) fn push(&mut self, hide: PendingHide) {
        self.0.push(hide);
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(super) fn as_slice(&self) -> &[PendingHide] {
        &self.0
    }

    pub(super) fn clear(&mut self) {
        self.0.clear();
    }
}

impl PoseSource for [PendingHide] {
    fn len(&self) -> usize {
        <[PendingHide]>::len(self)
    }

    fn entity(&self, index: usize) -> EntityId {
        self[index].entity
    }

    fn pose(&self, index: usize) -> Option<RowPose<'_>> {
        self.get(index).map(|hide| RowPose::Hidden { at: hide.at })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(entity: u64) -> PlacedItem {
        PlacedItem {
            entity: EntityId(entity),
            base: "00ABCD:Test.esm".to_owned(),
            count: 3,
            owner: None,
            position: Vec3::new(1.5, -2.0, 4.0),
        }
    }

    fn authored() -> AuthoredColliders {
        (Arc::new(CellColliderBatch::default()), None)
    }

    #[test]
    fn a_reference_without_a_count_stands_for_one_unit() {
        assert_eq!(stack_count(None), Ok(1));
    }

    #[test]
    fn a_positive_count_is_taken_as_it_is_up_to_the_largest_stack() {
        assert_eq!(stack_count(Some(1)), Ok(1));
        assert_eq!(stack_count(Some(37)), Ok(37));
        assert_eq!(stack_count(Some(i64::from(u32::MAX))), Ok(u32::MAX));
    }

    #[test]
    fn a_zero_or_negative_count_is_not_positive() {
        assert_eq!(stack_count(Some(0)), Err(CountError::NotPositive(0)));
        assert_eq!(stack_count(Some(-4)), Err(CountError::NotPositive(-4)));
    }

    #[test]
    fn a_count_beyond_a_stack_is_too_large() {
        let count = i64::from(u32::MAX) + 1;
        assert_eq!(stack_count(Some(count)), Err(CountError::TooLarge(count)));
    }

    #[test]
    fn a_placed_item_with_a_bad_count_stands_for_one_unit_and_keeps_its_owner() {
        let placed = placed_item(
            EntityId(9),
            "000ABC:Test.esm",
            Some(-2),
            Some("0001F2:Test.esm"),
            Vec3::X,
        );

        assert_eq!(
            placed,
            PlacedItem {
                entity: EntityId(9),
                base: "000ABC:Test.esm".to_owned(),
                count: 1,
                owner: Some("0001F2:Test.esm".to_owned()),
                position: Vec3::X,
            }
        );
    }

    #[test]
    fn a_placed_item_takes_its_reference_count() {
        assert_eq!(
            placed_item(EntityId(9), "B", Some(12), None, Vec3::ZERO).count,
            12
        );
    }

    #[test]
    fn only_a_base_known_not_to_be_a_marker_is_an_item_and_only_a_named_one_is_pickable() {
        assert_eq!(
            item_base_role(Some(true), Some("Wrench")),
            ItemBaseRole::Marker
        );
        assert_eq!(
            item_base_role(None, Some("Wrench")),
            ItemBaseRole::UnknownMarker
        );
        assert_eq!(
            item_base_role(Some(false), Some("Wrench")),
            ItemBaseRole::Pickable
        );
        assert_eq!(item_base_role(Some(false), Some("  ")), ItemBaseRole::Loose);
        assert_eq!(item_base_role(Some(false), None), ItemBaseRole::Loose);
    }

    #[test]
    fn a_cell_without_edits_registers_as_authored() {
        let command = register_command((2, -3), &authored(), &CellEdits::default());

        assert!(matches!(
            command,
            SimCommand::AddCellColliders {
                grid: (2, -3),
                baked: None,
                ..
            }
        ));
    }

    #[test]
    fn a_cell_with_removed_items_registers_without_them() {
        let mut items = HashMap::from([(EntityId(5), item(5)), (EntityId(6), item(6))]);
        let mut edits = CellEdits::default();
        take_item(&mut items, &mut edits, EntityId(6));

        let command = register_command((2, -3), &authored(), &edits);

        let SimCommand::AddCellCollidersSkipping { grid, skip, .. } = command else {
            panic!("an edited cell registers with its skip set");
        };
        assert_eq!(grid, (2, -3));
        assert_eq!(*skip, HashSet::from([EntityId(6)]));
    }

    #[test]
    fn the_registration_carries_the_authored_batch_itself() {
        let authored = authored();
        let mut edits = CellEdits::default();
        Arc::make_mut(&mut edits.removed).insert(EntityId(1));

        let SimCommand::AddCellCollidersSkipping { batch, .. } =
            register_command((0, 0), &authored, &edits)
        else {
            panic!("an edited cell registers with its skip set");
        };
        assert!(Arc::ptr_eq(&batch, &authored.0));
    }

    #[test]
    fn taking_an_item_moves_it_from_the_records_to_the_removed_set() {
        let mut items = HashMap::from([(EntityId(5), item(5)), (EntityId(6), item(6))]);
        let mut edits = CellEdits::default();

        let taken = take_item(&mut items, &mut edits, EntityId(5));

        assert_eq!(taken, Some(item(5)));
        assert_eq!(items.keys().copied().collect::<Vec<_>>(), vec![EntityId(6)]);
        assert_eq!(*edits.removed, HashSet::from([EntityId(5)]));
    }

    #[test]
    fn an_entity_that_is_no_item_of_the_cell_changes_nothing() {
        let mut items = HashMap::from([(EntityId(5), item(5))]);
        let mut edits = CellEdits::default();

        assert_eq!(take_item(&mut items, &mut edits, EntityId(8)), None);
        assert_eq!(
            take_item(&mut items, &mut edits, EntityId(5)),
            Some(item(5))
        );
        assert_eq!(take_item(&mut items, &mut edits, EntityId(5)), None);

        assert!(items.is_empty());
        assert_eq!(*edits.removed, HashSet::from([EntityId(5)]));
    }

    #[test]
    fn a_registration_already_sent_keeps_its_skip_set_when_the_cell_edits_again() {
        let mut items = HashMap::from([(EntityId(5), item(5)), (EntityId(6), item(6))]);
        let mut edits = CellEdits::default();
        take_item(&mut items, &mut edits, EntityId(5));
        let SimCommand::AddCellCollidersSkipping { skip: sent, .. } =
            register_command((0, 0), &authored(), &edits)
        else {
            panic!("an edited cell registers with its skip set");
        };

        take_item(&mut items, &mut edits, EntityId(6));

        assert_eq!(*sent, HashSet::from([EntityId(5)]));
        assert_eq!(*edits.removed, HashSet::from([EntityId(5), EntityId(6)]));
    }

    #[test]
    fn a_pending_hide_collapses_the_rows_of_its_entity_to_the_item_position() {
        let hides = [PendingHide {
            entity: EntityId(4),
            grid: (1, 1),
            residency: 2,
            at: Vec3::new(7.0, 8.0, 9.0),
        }];

        assert_eq!(PoseSource::len(&hides[..]), 1);
        assert_eq!(hides[..].entity(0), EntityId(4));
        assert!(matches!(
            hides[..].pose(0),
            Some(RowPose::Hidden { at }) if at == Vec3::new(7.0, 8.0, 9.0)
        ));
        assert!(hides[..].pose(1).is_none());
    }
}
