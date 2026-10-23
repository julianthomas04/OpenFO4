use super::skeleton_overlay::{SkeletonOverlay, skeleton_overlay_section};
use crate::sim::RagdollSettings;

/// The "Ragdolls" subsection of the Physics debug section. Returns the edited
/// settings when a slider changed.
pub fn ragdoll_settings_section(
    ui: &mut egui::Ui,
    current: RagdollSettings,
    overlay: &mut SkeletonOverlay,
) -> Option<RagdollSettings> {
    let mut edited = current;
    egui::CollapsingHeader::new("Ragdolls")
        .default_open(false)
        .show(ui, |ui| {
            ui.add(
                egui::Slider::new(&mut edited.envelope, RagdollSettings::ENVELOPE_RANGE)
                    .logarithmic(true)
                    .text("Rest envelope (units)"),
            );
            ui.add(
                egui::Slider::new(
                    &mut edited.window_seconds,
                    RagdollSettings::WINDOW_SECONDS_RANGE,
                )
                .text("Rest window (s)"),
            );
            ui.add(
                egui::Slider::new(
                    &mut edited.settle_damping,
                    RagdollSettings::SETTLE_DAMPING_RANGE,
                )
                .text("Near-rest damping"),
            );
            if ui.button("Reset").clicked() {
                edited = RagdollSettings::default();
            }
            skeleton_overlay_section(ui, overlay);
        });
    (edited != current).then_some(edited)
}
