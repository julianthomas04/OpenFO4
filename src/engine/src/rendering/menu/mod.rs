//! The main menu, the spawn picker and the loading overlay, drawn with the HUD's quad pipeline,
//! glyphon text and the loading logo.

mod background_quad;
mod image_quad;
mod layout;
mod loading_icon;
mod loading_panel;
mod renderer;
#[cfg(test)]
mod renderer_tests;
mod safe_rect;
mod video_layer;
#[cfg(test)]
mod video_layer_tests;

pub use layout::{LAYOUT_KEYS, MenuLayout, RowRect};
pub use loading_panel::PanelContent;
pub use renderer::{Backdrop, MenuError, MenuRenderer, MenuScene, OverlayForeground};
