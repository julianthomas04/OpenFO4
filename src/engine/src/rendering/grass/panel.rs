//! The debug panel's Grass section.

use super::fade::{FadeLimits, FadeOverride, RANGE_RANGE_GAME_UNITS};
use super::{GrassControls, GrassResidency, GrassStats};
use crate::utils::format_bytes;

/// Read-only figures the section shows next to the controls.
pub struct GrassPanelState {
    pub available: bool,
    pub stats: GrassStats,
    pub residency: GrassResidency,
    pub fade_limits: FadeLimits,
}

pub fn grass_panel(ui: &mut egui::Ui, state: &GrassPanelState, controls: &mut GrassControls) {
    if state.available {
        ui.checkbox(&mut controls.draw, "Draw grass");
    } else {
        ui.label("Grass is disabled by OPENFO4_GRASS=0");
    }
    fade_controls(ui, &mut controls.fade, &state.fade_limits);
    stats_grid(ui, state);
}

fn fade_controls(ui: &mut egui::Ui, fade: &mut FadeOverride, limits: &FadeLimits) {
    ui.add(
        egui::Slider::new(
            &mut fade.start_game_units,
            limits.start_game_units.0..=limits.start_game_units.1,
        )
        .text("Fade start (BU)"),
    );
    ui.add(
        egui::Slider::new(
            &mut fade.range_game_units,
            RANGE_RANGE_GAME_UNITS.0..=RANGE_RANGE_GAME_UNITS.1,
        )
        .text("Fade range (BU)"),
    );
    if ui.button("Reset").clicked() {
        *fade = FadeOverride::configured(limits);
    }
    let resolved = fade.resolve(limits);
    ui.label(format!(
        "Draw distance: {:.0} BU",
        resolved.draw_distance_game_units
    ));
    if resolved.clamped {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            format!(
                "start + range exceeds the loaded cell block; clamped to {:.0} BU",
                resolved.cap_game_units
            ),
        );
    }
}

fn stats_grid(ui: &mut egui::Ui, state: &GrassPanelState) {
    let GrassPanelState {
        stats, residency, ..
    } = state;
    ui.label(format!(
        "Visible: {} draws, {} instances",
        stats.draws, stats.instances
    ));
    ui.label(format!(
        "Resident: {} cells, {} grass types, {} models, {} blades",
        residency.cells, residency.types, residency.models, residency.blades
    ));
    ui.label(format!(
        "Instance buffers: {}",
        format_bytes(residency.instance_bytes)
    ));
}
