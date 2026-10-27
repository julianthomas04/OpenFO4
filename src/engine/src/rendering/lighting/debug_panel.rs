use super::state::{ExposureSettings, FogSettings, LightingState};
use super::tonemap_settings::{TonemapOperator, TonemapSettings};
use super::weather_panel::weather_section;

const FOG_DISTANCE_MAX: f32 = 30000.0;
const SHADOW_BIAS_MAX_TEXELS: f32 = 8.0;

pub fn lighting_panel(ui: &mut egui::Ui, lighting: &mut LightingState) {
    weather_section(ui, lighting);

    ui.separator();
    ui.checkbox(&mut lighting.sun_override, "Override (manual sun/sky)");
    let manual = lighting.sun_override;
    ui.add_enabled_ui(manual, |ui| {
        ui.label("Sun");
        ui.add(
            egui::Slider::new(&mut lighting.sun.azimuth_deg, 0.0..=360.0)
                .text("azimuth (0 = north, clockwise)"),
        );
        ui.add(egui::Slider::new(&mut lighting.sun.elevation_deg, -10.0..=90.0).text("elevation"));
        color_row(ui, "color", &mut lighting.sun.color);
        color_row(ui, "disc color", &mut lighting.sun.disc_color);
        ui.add(egui::Slider::new(&mut lighting.sun.intensity, 0.0..=10.0).text("intensity"));

        ui.separator();
        ui.label("Sky");
        color_row(ui, "zenith", &mut lighting.sky.zenith);
        color_row(ui, "horizon", &mut lighting.sky.horizon);
        color_row(ui, "ground", &mut lighting.sky.ground);
        ui.add(egui::Slider::new(&mut lighting.sky.radiance_scale, 0.0..=10.0).text("sky scale"));

        ui.separator();
        fog_section(ui, &mut lighting.fog);
    });
    ui.add(egui::Slider::new(&mut lighting.sky.ambient_intensity, 0.0..=2.0).text("ambient"));

    ui.separator();
    shadow_section(ui, lighting);

    ui.separator();
    exposure_section(ui, &mut lighting.exposure);
    tonemap_section(ui, &mut lighting.tonemap);
}

fn exposure_section(ui: &mut egui::Ui, exposure: &mut ExposureSettings) {
    ui.checkbox(&mut exposure.auto, "auto exposure");
    let label = if exposure.auto {
        "exposure offset (EV)"
    } else {
        "exposure (EV)"
    };
    ui.add(egui::Slider::new(&mut exposure.ev, -6.0..=6.0).text(label));
    ui.label(format!(
        "adapted luminance {:.4}, exposure {:.3}",
        exposure.readout.adapted_luminance, exposure.readout.exposure
    ));
}

fn tonemap_section(ui: &mut egui::Ui, tonemap: &mut TonemapSettings) {
    egui::ComboBox::from_label("tonemap")
        .selected_text(tonemap.operator.label())
        .show_ui(ui, |ui| {
            for operator in TonemapOperator::ALL {
                ui.selectable_value(&mut tonemap.operator, operator, operator.label());
            }
        });
    ui.add_enabled(
        tonemap.operator == TonemapOperator::Reinhard,
        egui::Slider::new(&mut tonemap.white_point, 1.0..=16.0).text("white point"),
    );
    ui.add(egui::Slider::new(&mut tonemap.saturation, 0.0..=2.0).text("saturation"));
}

fn fog_section(ui: &mut egui::Ui, fog: &mut FogSettings) {
    egui::CollapsingHeader::new("Fog").show(ui, |ui| {
        let params = &mut fog.params;
        ui.add(egui::Slider::new(&mut params.near, 0.0..=FOG_DISTANCE_MAX).text("near"));
        ui.add(egui::Slider::new(&mut params.far, 0.0..=FOG_DISTANCE_MAX).text("far"));
        ui.add(egui::Slider::new(&mut params.power, 0.1..=4.0).text("power"));
        ui.add(egui::Slider::new(&mut params.max, 0.0..=1.0).text("max"));
        ui.checkbox(&mut fog.link_to_horizon, "link colours to sky horizon");
        if !fog.link_to_horizon {
            color_row(ui, "near color", &mut params.colors.near);
            color_row(ui, "far color", &mut params.colors.far);
            color_row(ui, "near high color", &mut params.colors.near_high);
            color_row(ui, "far high color", &mut params.colors.far_high);
        }
    });
}

fn shadow_section(ui: &mut egui::Ui, lighting: &mut LightingState) {
    egui::CollapsingHeader::new("Shadows").show(ui, |ui| {
        ui.label(format!("status: {}", lighting.shadow_status().label()));
        let config = lighting.shadows.config;
        ui.label(format!(
            "INI: {} cascades of {}², {:.0} units, blend {:.1}, {:?} filter, bias scale {}",
            config.cascade_count,
            config.resolution,
            config.distance,
            config.split_blend,
            config.filter,
            config.bias_scale,
        ));

        let settings = &mut lighting.shadows.settings;
        ui.checkbox(&mut settings.enabled, "enabled");
        let texels = 0.0..=SHADOW_BIAS_MAX_TEXELS;
        ui.add(
            egui::Slider::new(&mut settings.constant_bias, texels.clone())
                .text("constant bias (texels)"),
        );
        ui.add(
            egui::Slider::new(&mut settings.slope_bias, texels.clone()).text("slope bias (texels)"),
        );
        ui.add(
            egui::Slider::new(&mut settings.normal_offset, texels).text("normal offset (texels)"),
        );
        ui.checkbox(&mut settings.freeze_cascades, "freeze cascades");
    });
}

/// egui's `color_edit_button_rgb` stores linear values and only gamma-encodes
/// them for display, so the vector it edits is already linear.
fn color_row(ui: &mut egui::Ui, label: &str, color: &mut glam::Vec3) {
    ui.horizontal(|ui| {
        let mut rgb = color.to_array();
        if egui::color_picker::color_edit_button_rgb(ui, &mut rgb).changed() {
            *color = glam::Vec3::from_array(rgb);
        }
        ui.label(label);
    });
}
