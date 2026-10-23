//! The converted game UI layout: documents the installer extracts from the game's interface
//! movies, typed keys to read them, boot validation, and Scaleform text rules.

mod document;
mod error;
pub(crate) mod faces;
pub mod keys;
mod layout;
mod movie;
pub mod outline;
pub mod scaleform_text;
pub mod set_text;
#[cfg(test)]
pub mod test_support;
#[cfg(test)]
mod tests;
pub mod values;

pub use error::UiLayoutError;
pub(crate) use keys::key_check_fields;
pub use keys::{
    BoundsKey, Declaration, DeclaredKind, FlagKey, FontClassKey, InstanceClassKey, Key, KeyCheck,
    KeyGroup, LabelKey, LiteralKey, MainTimelineKey, ModuleKeys, NumberKey, PlacementKey,
    ScalarKind, ShapeKey, StageKey, StringKey, SymbolTimelineKey, TextKey, TimelineKey,
    TranslateKey,
};
pub use layout::{FONT_CONFIG_DOCUMENT, UiLayout};
pub use movie::Movie;
