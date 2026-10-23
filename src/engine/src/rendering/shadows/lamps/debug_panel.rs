use super::cache::LampShadowStats;
use super::settings::MAX_SHADOWED_LAMPS;
use super::state::LampShadowState;

const BIAS_MAX_TEXELS: f32 = 8.0;
const ATLAS_PREVIEW_POINTS: f32 = 320.0;

pub fn lamp_shadows_panel(
    ui: &mut egui::Ui,
    state: &mut LampShadowState,
    stats: &LampShadowStats,
    atlas_preview: egui::TextureId,
) {
    egui::CollapsingHeader::new("Lamp shadows").show(ui, |ui| {
        let config = state.config;
        ui.label(format!(
            "INI: up to {} lamps, {}² atlas, {:?} filter, tiles halve every {:.0} units, gone at {:.0} units",
            config.max_lamps, config.atlas_resolution, config.filter, config.halve_distance, config.fade_end,
        ));

        let settings = &mut state.settings;
        ui.checkbox(&mut settings.enabled, "enabled");
        ui.add(egui::Slider::new(&mut settings.max_lamps, 0..=MAX_SHADOWED_LAMPS).text("max lamps"));
        let texels = 0.0..=BIAS_MAX_TEXELS;
        ui.add(egui::Slider::new(&mut settings.constant_bias, texels.clone()).text("constant bias (texels)"));
        ui.add(egui::Slider::new(&mut settings.slope_bias, texels.clone()).text("slope bias (texels)"));
        ui.add(egui::Slider::new(&mut settings.normal_offset, texels).text("normal offset (texels)"));
        ui.checkbox(&mut settings.freeze_selection, "freeze selection");
        if ui.button("re-render all tiles").clicked() {
            settings.rerender_requested = true;
        }

        ui.label(format!(
            "lamps: {} resident, {} eligible, {} selected, {} sampled",
            stats.resident, stats.eligible, stats.selected, stats.sampled,
        ));
        ui.label(format!(
            "views rendered: {}, faces pending: {}",
            stats.views_rendered, stats.faces_dirty,
        ));
        ui.label(format!(
            "atlas: {:.1}% used, {} halvings to fit, {} repacks",
            stats.atlas_used_percent, stats.fit_halvings, stats.atlas_repacks,
        ));

        ui.checkbox(&mut settings.show_atlas, "show atlas");
        if settings.show_atlas {
            ui.image((atlas_preview, egui::vec2(ATLAS_PREVIEW_POINTS, ATLAS_PREVIEW_POINTS)));
        }
    });
}
