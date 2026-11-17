//! Which descriptor an item's sound plays. Pure: the catalog is the only input.

use super::ItemSoundKind;
use crate::assets::db::item_sounds::ItemSoundCatalog;

/// The descriptor key for `kind`: the item's own, else its slot's default for that side. `None` when
/// the catalog does not know the item, or when the slot has no default (a `from_connection`
/// invariant, still checked).
pub fn descriptor_for<'a>(
    catalog: &'a ItemSoundCatalog,
    item: &str,
    kind: ItemSoundKind,
) -> Option<&'a str> {
    let entry = catalog.items.get(item)?;
    let (own, slot) = match kind {
        ItemSoundKind::PickUp => (&entry.pickup, entry.slot.pickup_slot()),
        ItemSoundKind::PutDown => (&entry.put_down, entry.slot.put_down_slot()),
    };
    own.as_deref()
        .or_else(|| catalog.defaults.get(slot).map(String::as_str))
}
