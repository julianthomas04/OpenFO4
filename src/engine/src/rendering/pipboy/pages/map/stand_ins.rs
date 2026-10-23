//! What the map page shows that the map movie and the translation table do not define: our own
//! stand-ins, each a named value kept here.

use egui::{Pos2, Vec2};

/// Shown in the map window when the catalog has no world map.
pub const NO_MAP_DATA_TEXT: &str = "NO MAP DATA";
/// Shown in the map window when the world map's image was never registered.
pub const NO_MAP_IMAGE_TEXT: &str = "NO MAP IMAGE";
/// The message-line notice for a "show on map" request the catalog has no map for.
pub const NO_MAP_LOCATION_NOTICE: &str = "NO MAP DATA TO SHOW THAT LOCATION ON";

pub const NOTICE_TEXT_SIZE: f32 = 28.0;
const NOTICE_FIELD_SIZE: Vec2 = Vec2::new(400.0, 28.0);

/// The message-line notice for a fast-travel request: the game's fast-travel label, then the
/// destination.
pub fn fast_travel_notice(label: &str, destination: &str) -> String {
    format!("{label}: {destination}")
}

/// The top-left corner and width of the window notice's text field, centred in the map window.
pub fn notice_field(window_centre: Pos2) -> (Pos2, f32) {
    (window_centre - NOTICE_FIELD_SIZE / 2.0, NOTICE_FIELD_SIZE.x)
}

/// Our own keyboard choice: the map pan one arrow key press moves, in stage pixels.
pub const KEY_PAN_STEP: f32 = 40.0;
/// Our own keyboard choice: the wheel delta one zoom key press stands for.
pub const KEY_ZOOM_DELTA: f32 = 1.0;
/// Our own keyboard choice: the favourites-slot key `-`, which zooms the map out.
pub const ZOOM_OUT_SLOT: u8 = 10;
/// Our own keyboard choice: the favourites-slot key `=`, which zooms the map in.
pub const ZOOM_IN_SLOT: u8 = 11;

/// The ring that marks the place a "show on map" request centred on.
pub const FOCUS_RING_RADIUS: f32 = 11.0;
pub const FOCUS_RING_STROKE: f32 = 2.0;
pub const FOCUS_RING_SECONDS: f64 = 3.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fast_travel_notice_names_the_destination_after_the_label() {
        assert_eq!(fast_travel_notice("GO", "Town"), "GO: Town");
    }

    #[test]
    fn the_notice_field_is_centred_on_the_window() {
        let (origin, width) = notice_field(Pos2::new(500.0, 300.0));
        assert!((origin - Pos2::new(300.0, 286.0)).length() < 1e-4);
        assert!((width - 400.0).abs() < 1e-4);
    }
}
