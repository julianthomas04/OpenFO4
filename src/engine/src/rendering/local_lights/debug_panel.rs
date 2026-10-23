use super::cluster_grid::{MAX_LIGHTS_PER_CLUSTER, MAX_VISIBLE_LIGHTS};
use super::settings::LocalLightSettings;
use super::stats::LocalLightCounters;

const INTENSITY_MAX: f32 = 8.0;

pub fn local_lights_panel(
    ui: &mut egui::Ui,
    settings: &mut LocalLightSettings,
    counters: &LocalLightCounters,
    loaded: u32,
) {
    egui::CollapsingHeader::new("Placed lights").show(ui, |ui| {
        ui.checkbox(&mut settings.enabled, "enabled");
        ui.add(
            egui::Slider::new(&mut settings.intensity, 0.0..=INTENSITY_MAX)
                .text("intensity multiplier"),
        );
        ui.checkbox(&mut settings.show_spheres, "show light spheres");
        ui.label(format!(
            "fade: {:.0} to {:.0} units",
            settings.fade.start,
            settings.fade.end()
        ));
        ui.label(format!("loaded: {loaded}"));
        if !settings.enabled {
            ui.label("visible: - (disabled)");
            return;
        }
        ui.label(format!(
            "visible: {} / {MAX_VISIBLE_LIGHTS} ({} over capacity)",
            counters.visible, counters.visible_overflow
        ));
        ui.label(format!(
            "clusters over {MAX_LIGHTS_PER_CLUSTER} lights: {}",
            counters.overflowing_clusters
        ));
    });
}
