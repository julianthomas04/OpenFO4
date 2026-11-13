//! Placed items of a cell and what the player changed about them while the cell is resident:
//! the item records the cell owns, the references taken out of the world, and how both reach
//! the collision registration and the instance rows.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use glam::Vec3;

use crate::rendering::game_text::non_blank;
use crate::rendering::row_pose::{PoseSource, RowPose};
use crate::sim::bake::BakedCellColliders;
use crate::sim::collision::CellColliderBatch;
use crate::sim::{AddedObjects, CellGrid, EntityId, RUNTIME_REF_PLUGIN, SimCommand};
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

/// A dropped item's colliders, kept so every later registration of its cell builds it again.
#[derive(Clone)]
pub(super) struct AddedObject {
    pub entity: EntityId,
    pub batch: Arc<CellColliderBatch>,
    pub baked: Option<BakedCellColliders>,
}

/// What the player changed in a resident cell. Belongs to the cell: unloading or demoting it
/// forgets the changes.
#[derive(Default)]
pub(super) struct CellEdits {
    /// References taken out of the world; their bodies are never built again.
    pub removed: Arc<HashSet<EntityId>>,
    /// Items dropped into the cell, built after its authored objects.
    pub added: Vec<AddedObject>,
}

impl CellEdits {
    /// A dropped item taken again is built by nothing: its added entry and its removed entry
    /// both go, so neither list grows with repeated drops and pickups.
    fn forget_retaken_drops(&mut self) {
        let removed = &self.removed;
        if !self
            .added
            .iter()
            .any(|added| removed.contains(&added.entity))
        {
            return;
        }
        let removed = Arc::make_mut(&mut self.removed);
        self.added.retain(|added| !removed.remove(&added.entity));
    }
}

/// The registration of a cell's colliders: as authored without edits, else without the
/// references taken out of the world and with the items dropped into it.
pub(super) fn register_command(
    grid: CellGrid,
    authored: &AuthoredColliders,
    edits: &mut CellEdits,
) -> SimCommand {
    edits.forget_retaken_drops();
    let (batch, baked) = authored;
    if edits.removed.is_empty() && edits.added.is_empty() {
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
            added: edits
                .added
                .iter()
                .map(|added| AddedObjects {
                    batch: Arc::clone(&added.batch),
                    baked: added.baked.clone(),
                })
                .collect(),
        }
    }
}

/// Whether a collision group becomes a moving body and whether a fixed tracked one: a static-only
/// mesh of an item reference is pinned, while a dropped object is always dynamic (a pinned body
/// never wakes, so it would hang where it was dropped).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct GroupBody {
    pub dynamic: bool,
    pub pinned: bool,
}

pub(super) fn group_body(item_group: bool, mesh_dynamic: bool, dropped: bool) -> GroupBody {
    let dynamic = mesh_dynamic || dropped;
    GroupBody {
        dynamic,
        pinned: item_group && !dynamic,
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

/// An inventory stack the player drops, in engine space.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DroppedItemSpec {
    pub base: String,
    pub count: u32,
    pub position: Vec3,
    /// Engine camera yaw the item faces (see `bethesda_space::yaw_from_rotation_z`).
    pub yaw: f32,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub(crate) enum SpawnItemError {
    #[error("{0}")]
    RefKeys(#[from] RuntimeKeyError),
    #[error("base {0} is not a pickable item")]
    NotPickable(String),
    #[error("a drop of {0} has no units")]
    NoUnits(String),
    #[error("no cell with registered colliders can own a dropped item at {0:?}")]
    NoOwnerCell(Vec3),
    #[error("the cell worker channel is closed")]
    WorkerChannelClosed,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum RuntimeKeyError {
    #[error("the runtime reference keys are exhausted")]
    Exhausted,
}

/// The largest form id a reference key can carry: six hex digits.
const MAX_RUNTIME_REF_ID: u32 = 0x00FF_FFFF;

/// A drop the world can take: a pickable base and at least one unit.
pub(super) fn checked_drop(
    spec: DroppedItemSpec,
    pickable: bool,
) -> Result<DroppedItemSpec, SpawnItemError> {
    if !pickable {
        return Err(SpawnItemError::NotPickable(spec.base));
    }
    if spec.count == 0 {
        return Err(SpawnItemError::NoUnits(spec.base));
    }
    Ok(spec)
}

/// Reference keys of the objects that exist only at runtime: `{id:06X}:*dropped`
/// (`RUNTIME_REF_PLUGIN`), ids from 1, never wrapped and never reused in a session.
pub(super) struct RuntimeRefKeys {
    next: u32,
}

impl RuntimeRefKeys {
    pub(super) fn new() -> Self {
        Self { next: 1 }
    }

    pub(super) fn next_key(&mut self) -> Result<String, RuntimeKeyError> {
        if self.next > MAX_RUNTIME_REF_ID {
            return Err(RuntimeKeyError::Exhausted);
        }
        let key = format!("{:06X}:{RUNTIME_REF_PLUGIN}", self.next);
        self.next += 1;
        Ok(key)
    }
}

/// A drop the world did not take; the caller returns the units with `SimWorld::restore_units`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AbandonedDrop {
    pub base: String,
    pub count: u32,
}

impl From<DroppedItemSpec> for AbandonedDrop {
    fn from(spec: DroppedItemSpec) -> Self {
        Self {
            base: spec.base,
            count: spec.count,
        }
    }
}

/// Why a built drop payload cannot join its cell.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(super) enum DropRejection {
    #[error("the world changed before the item was built")]
    WorldChanged,
    #[error("its cell left before the item was built")]
    CellLeft,
    #[error("its payload holds no body")]
    NoBody,
    #[error("its model drew no rows")]
    NotDrawn,
}

/// The item record a built drop payload brings into its cell, if it may join it: built for the
/// current world, for the same residency of the owning cell, and with a body.
pub(super) fn drop_admission(
    same_world: bool,
    same_cell: bool,
    item: Option<PlacedItem>,
) -> Result<PlacedItem, DropRejection> {
    if !same_world {
        return Err(DropRejection::WorldChanged);
    }
    if !same_cell {
        return Err(DropRejection::CellLeft);
    }
    item.ok_or(DropRejection::NoBody)
}

/// The cell that owns a dropped item and its residency. Outdoors it is the cell of the drop
/// point, and only while its colliders are registered: over any other cell the body would fall
/// with no ground under it, so no cell owns the drop. In an interior the drop point's exterior
/// grid means nothing and the anchor cell owns it.
pub(super) fn drop_owner(
    drop_grid: CellGrid,
    player_cell: CellGrid,
    in_interior: bool,
    collision_registered: impl Fn(CellGrid) -> bool,
    residency: impl Fn(CellGrid) -> Option<u64>,
) -> Option<(CellGrid, u64)> {
    let owner = if in_interior {
        player_cell
    } else if collision_registered(drop_grid) {
        drop_grid
    } else {
        return None;
    };
    residency(owner).map(|residency| (owner, residency))
}

/// Records an admitted drop in its cell: the item, and its colliders as an object the cell's
/// edits add to every later registration. A drop whose model drew no rows is refused before
/// anything is recorded, as it would be a solid body nobody can see or aim at. Returns the
/// command that adds the body now when the cell's colliders are registered.
pub(super) fn join_drop(
    item: PlacedItem,
    colliders: Option<AuthoredColliders>,
    drawn: bool,
    registered_grid: Option<CellGrid>,
    items: &mut HashMap<EntityId, PlacedItem>,
    edits: &mut CellEdits,
) -> Result<Option<SimCommand>, DropRejection> {
    if !drawn {
        return Err(DropRejection::NotDrawn);
    }
    let entity = item.entity;
    items.insert(entity, item);
    let Some((batch, baked)) = colliders else {
        return Ok(None);
    };
    let command = registered_grid.map(|grid| SimCommand::AddWorldObjects {
        grid,
        batch: Arc::clone(&batch),
        baked: baked.clone(),
    });
    edits.added.push(AddedObject {
        entity,
        batch,
        baked,
    });
    Ok(command)
}

/// The Bethesda Z rotation an engine yaw faces: the inverse of
/// `bethesda_space::yaw_from_rotation_z`.
pub(super) fn rotation_z_from_yaw(yaw: f32) -> f32 {
    yaw + std::f32::consts::FRAC_PI_2
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
        let command = register_command((2, -3), &authored(), &mut CellEdits::default());

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

        let command = register_command((2, -3), &authored(), &mut edits);

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
            register_command((0, 0), &authored, &mut edits)
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
            register_command((0, 0), &authored(), &mut edits)
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

    fn dropped(entity: u64) -> AddedObject {
        AddedObject {
            entity: EntityId(entity),
            batch: Arc::new(CellColliderBatch::default()),
            baked: None,
        }
    }

    fn added_batches(command: &SimCommand) -> Vec<*const CellColliderBatch> {
        let SimCommand::AddCellCollidersSkipping { added, .. } = command else {
            panic!("an edited cell registers with its edits");
        };
        added
            .iter()
            .map(|object| Arc::as_ptr(&object.batch))
            .collect()
    }

    #[test]
    fn a_cell_with_a_dropped_item_registers_it_after_the_authored_objects() {
        let mut edits = CellEdits {
            added: vec![dropped(40), dropped(41)],
            ..CellEdits::default()
        };
        let expected: Vec<_> = edits.added.iter().map(|a| Arc::as_ptr(&a.batch)).collect();

        let command = register_command((1, 1), &authored(), &mut edits);

        assert_eq!(added_batches(&command), expected);
        let SimCommand::AddCellCollidersSkipping { skip, .. } = command else {
            unreachable!("checked above");
        };
        assert!(skip.is_empty());
    }

    #[test]
    fn a_dropped_item_taken_again_is_registered_by_neither_list() {
        let mut items = HashMap::from([(EntityId(5), item(5)), (EntityId(41), item(41))]);
        let mut edits = CellEdits {
            added: vec![dropped(40), dropped(41)],
            ..CellEdits::default()
        };
        take_item(&mut items, &mut edits, EntityId(41));
        take_item(&mut items, &mut edits, EntityId(5));
        let kept = Arc::as_ptr(&edits.added[0].batch);

        let command = register_command((1, 1), &authored(), &mut edits);

        assert_eq!(added_batches(&command), vec![kept]);
        let SimCommand::AddCellCollidersSkipping { skip, .. } = &command else {
            unreachable!("checked above");
        };
        assert_eq!(**skip, HashSet::from([EntityId(5)]));
        assert_eq!(*edits.removed, HashSet::from([EntityId(5)]));
        assert_eq!(edits.added.len(), 1);
        assert_eq!(edits.added[0].entity, EntityId(40));
    }

    #[test]
    fn a_dropped_item_taken_again_alone_leaves_a_cell_registered_as_authored() {
        let mut items = HashMap::from([(EntityId(41), item(41))]);
        let mut edits = CellEdits {
            added: vec![dropped(41)],
            ..CellEdits::default()
        };
        take_item(&mut items, &mut edits, EntityId(41));

        let command = register_command((1, 1), &authored(), &mut edits);

        assert!(matches!(command, SimCommand::AddCellColliders { .. }));
        assert!(edits.removed.is_empty() && edits.added.is_empty());
    }

    #[test]
    fn only_a_static_mesh_of_a_placed_item_is_pinned_and_a_dropped_object_always_moves() {
        let body = |dynamic, pinned| GroupBody { dynamic, pinned };
        assert_eq!(group_body(true, false, false), body(false, true));
        assert_eq!(group_body(true, true, false), body(true, false));
        assert_eq!(group_body(false, false, false), body(false, false));
        assert_eq!(group_body(false, true, false), body(true, false));
        assert_eq!(group_body(true, false, true), body(true, false));
        assert_eq!(group_body(false, false, true), body(true, false));
    }

    #[test]
    fn runtime_keys_count_up_from_one_in_six_hex_digits_of_the_runtime_plugin() {
        let mut keys = RuntimeRefKeys::new();

        assert_eq!(keys.next_key(), Ok("000001:*dropped".to_owned()));
        assert_eq!(keys.next_key(), Ok("000002:*dropped".to_owned()));
    }

    #[test]
    fn runtime_keys_end_at_the_largest_form_id_and_never_wrap() {
        let mut keys = RuntimeRefKeys { next: 0x00FF_FFFE };

        assert_eq!(keys.next_key(), Ok("FFFFFE:*dropped".to_owned()));
        assert_eq!(keys.next_key(), Ok("FFFFFF:*dropped".to_owned()));
        assert_eq!(keys.next_key(), Err(RuntimeKeyError::Exhausted));
        assert_eq!(keys.next_key(), Err(RuntimeKeyError::Exhausted));
    }

    fn residency_of(grid: CellGrid) -> Option<u64> {
        match grid {
            (3, 4) => Some(11),
            (0, 0) => Some(7),
            _ => None,
        }
    }

    #[test]
    fn a_drop_belongs_to_the_cell_of_the_drop_point_when_its_colliders_are_registered() {
        let owner = drop_owner((3, 4), (0, 0), false, |grid| grid == (3, 4), residency_of);

        assert_eq!(owner, Some(((3, 4), 11)));
    }

    #[test]
    fn a_drop_over_a_cell_without_registered_colliders_has_no_owner() {
        assert_eq!(
            drop_owner((3, 4), (0, 0), false, |_| false, residency_of),
            None
        );
        assert_eq!(
            drop_owner((0, 0), (0, 0), false, |grid| grid != (0, 0), residency_of),
            None
        );
    }

    #[test]
    fn a_drop_over_a_registered_cell_that_is_not_resident_has_no_owner() {
        assert_eq!(
            drop_owner((9, 9), (0, 0), false, |_| true, residency_of),
            None
        );
    }

    #[test]
    fn a_drop_in_an_interior_belongs_to_its_anchor_cell() {
        assert_eq!(
            drop_owner((3, 4), (0, 0), true, |_| true, residency_of),
            Some(((0, 0), 7))
        );
    }

    #[test]
    fn an_interior_drop_without_a_resident_anchor_has_no_owner() {
        assert_eq!(
            drop_owner((3, 4), (6, 6), true, |_| true, residency_of),
            None
        );
    }

    #[test]
    fn a_yaw_turned_into_a_rotation_turns_back_into_the_same_yaw() {
        for yaw in [-3.0_f32, -1.2, 0.0, 0.4, 2.9] {
            let back = crate::bethesda_space::yaw_from_rotation_z(rotation_z_from_yaw(yaw));
            assert!((back - yaw).abs() < 1e-6, "{yaw} -> {back}");
        }
        assert_eq!(rotation_z_from_yaw(-std::f32::consts::FRAC_PI_2), 0.0);
    }

    #[test]
    fn an_abandoned_drop_returns_the_dropped_base_and_units() {
        let spec = DroppedItemSpec {
            base: "00BEEF:Test.esm".to_owned(),
            count: 4,
            position: Vec3::ONE,
            yaw: 0.3,
        };

        assert_eq!(
            AbandonedDrop::from(spec),
            AbandonedDrop {
                base: "00BEEF:Test.esm".to_owned(),
                count: 4
            }
        );
    }

    #[test]
    fn a_payload_joins_its_cell_only_in_the_same_world_and_residency_and_with_a_body() {
        assert_eq!(drop_admission(true, true, Some(item(3))), Ok(item(3)));
        assert_eq!(
            drop_admission(false, true, Some(item(3))),
            Err(DropRejection::WorldChanged)
        );
        assert_eq!(
            drop_admission(true, false, Some(item(3))),
            Err(DropRejection::CellLeft)
        );
        assert_eq!(drop_admission(true, true, None), Err(DropRejection::NoBody));
    }

    fn drop_of(count: u32) -> DroppedItemSpec {
        DroppedItemSpec {
            base: "00BEEF:Test.esm".to_owned(),
            count,
            position: Vec3::ONE,
            yaw: 0.3,
        }
    }

    #[test]
    fn a_drop_of_a_pickable_base_with_units_is_taken_as_it_is() {
        assert_eq!(checked_drop(drop_of(2), true), Ok(drop_of(2)));
    }

    #[test]
    fn a_drop_without_units_is_refused_rather_than_made_one_unit() {
        assert_eq!(
            checked_drop(drop_of(0), true),
            Err(SpawnItemError::NoUnits("00BEEF:Test.esm".to_owned()))
        );
    }

    #[test]
    fn a_drop_of_a_base_that_is_not_pickable_is_refused() {
        assert_eq!(
            checked_drop(drop_of(2), false),
            Err(SpawnItemError::NotPickable("00BEEF:Test.esm".to_owned()))
        );
    }

    #[test]
    fn a_drop_that_drew_no_rows_records_neither_item_nor_body() {
        let mut items = HashMap::new();
        let mut edits = CellEdits::default();

        let joined = join_drop(
            item(44),
            Some(authored()),
            false,
            Some((2, 2)),
            &mut items,
            &mut edits,
        );

        assert_eq!(joined.err(), Some(DropRejection::NotDrawn));
        assert!(items.is_empty() && edits.added.is_empty());
    }

    #[test]
    fn a_drawn_drop_in_a_registered_cell_adds_its_body_now_and_to_later_registrations() {
        let mut items = HashMap::new();
        let mut edits = CellEdits::default();
        let colliders = authored();
        let batch = Arc::as_ptr(&colliders.0);

        let joined = join_drop(
            item(44),
            Some(colliders),
            true,
            Some((2, 2)),
            &mut items,
            &mut edits,
        );

        let Ok(Some(SimCommand::AddWorldObjects {
            grid, batch: sent, ..
        })) = joined
        else {
            panic!("a registered cell receives the body at once");
        };
        assert_eq!(grid, (2, 2));
        assert_eq!(Arc::as_ptr(&sent), batch);
        assert_eq!(items[&EntityId(44)], item(44));
        assert_eq!(edits.added.len(), 1);
        assert_eq!(edits.added[0].entity, EntityId(44));
        assert_eq!(Arc::as_ptr(&edits.added[0].batch), batch);
    }

    #[test]
    fn a_drawn_drop_in_an_unregistered_cell_waits_for_the_next_registration() {
        let mut items = HashMap::new();
        let mut edits = CellEdits::default();

        let joined = join_drop(
            item(44),
            Some(authored()),
            true,
            None,
            &mut items,
            &mut edits,
        );

        assert!(matches!(joined, Ok(None)));
        assert!(items.contains_key(&EntityId(44)));
        assert_eq!(edits.added.len(), 1);
    }
}
