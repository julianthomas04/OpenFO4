//! What happened to the player's inventory through the world, for the sound and the HUD message.

use super::pipboy::InventoryError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InventoryEvent {
    Added { form_key: String, count: u32 },
    Dropped { form_key: String, count: u32 },
}

/// Why units could not join the inventory.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ItemGrantError {
    #[error("item {form_key} has no catalog entry")]
    UnknownItem { form_key: String },
    #[error(transparent)]
    Inventory(#[from] InventoryError),
}
