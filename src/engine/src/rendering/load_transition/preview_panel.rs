//! The debug menu's "Loading screen" section: pick a screen, or a random one, to show over the
//! world.

use super::LoadScreens;
use super::preview::{ScreenRequest, matching_entries};

const LIST_MAX_HEIGHT: f32 = 220.0;

/// Draws the section; answers the screen the user asked to open, if they did.
pub(in crate::rendering) fn loading_screen_section(
    ui: &mut egui::Ui,
    screens: &mut LoadScreens,
) -> Option<ScreenRequest> {
    if screens.preview.entries.is_none() {
        match screens.host.screen_entries() {
            Ok(entries) => screens.preview.entries = Some(entries),
            Err(error) => {
                ui.colored_label(ui.visuals().error_fg_color, error.to_string());
                return None;
            }
        }
    }
    let preview = &mut screens.preview;
    let mut request = None;
    ui.label("Opens over the world; Esc closes it.");
    ui.checkbox(
        &mut preview.pin_camera,
        "Pin camera (no spin, start view, still logo)",
    );
    if ui.button("Random").clicked() {
        request = Some(ScreenRequest::Random);
    }
    ui.horizontal(|ui| {
        ui.label("Filter:");
        ui.text_edit_singleline(&mut preview.filter);
    });
    let entries = preview.entries.as_deref().unwrap_or_default();
    let matching = matching_entries(entries, &preview.filter);
    let row_height = ui.text_style_height(&egui::TextStyle::Button);
    egui::ScrollArea::vertical()
        .max_height(LIST_MAX_HEIGHT)
        .show_rows(ui, row_height, matching.len(), |ui, rows| {
            for entry in &matching[rows] {
                if ui.selectable_label(false, &entry.text).clicked() {
                    request = Some(ScreenRequest::Named(entry.form_key.clone()));
                }
            }
        });
    if let Some(error) = &preview.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    request
}
