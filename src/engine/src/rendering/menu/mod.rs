//! The main menu, the spawn picker and the loading overlay, drawn with the HUD's quad pipeline,
//! glyphon text and the loading logo.

mod layout;
mod loading_icon;
mod renderer;
#[cfg(test)]
mod renderer_tests;
mod video_layer;
#[cfg(test)]
mod video_layer_tests;

pub use layout::{LAYOUT_KEYS, MenuLayout, RowRect};
pub use renderer::{MenuError, MenuRenderer, MenuScene};
