use glam::Vec3;

use super::ambient::AmbientCube;
use super::cell_lighting::{CellLighting, DirectionalLight, ResolvedLighting, TemplateStatus};
use super::environment::{Environment, InteriorLighting, NeutralReason};
use super::exterior_weather::ExteriorWeather;
use super::fog::Fog;
use super::state::LightingState;

const SWATCH_SIZE: egui::Vec2 = egui::vec2(28.0, 14.0);
const AMBIENT_FACE_LABELS: [&str; 6] = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"];

pub fn environment_section(ui: &mut egui::Ui, lighting: &mut LightingState) {
    egui::CollapsingHeader::new("Environment")
        .default_open(true)
        .show(ui, |ui| {
            let lighting_weather = lighting.exterior_weather().cloned();
            match lighting.environment() {
                Environment::Exterior => {
                    ui.label("exterior");
                    weather_status(ui, lighting_weather);
                }
                Environment::Interior { cell, lighting } => {
                    ui.label(format!("interior {cell}"));
                    interior_status(ui, lighting);
                }
            }
            ui.checkbox(&mut lighting.use_cell_lighting, "Use cell lighting (XCLL)");
            ui.checkbox(
                &mut lighting.use_weather_ambient,
                "Use weather ambient (else sky ambient)",
            );
        });
}

fn weather_status(ui: &mut egui::Ui, weather: Option<ExteriorWeather>) {
    match weather {
        Some(weather) => {
            ui.label(format!(
                "weather {} ({})",
                weather.editor_id.as_deref().unwrap_or("<no editor id>"),
                weather.form_key
            ));
            match weather.fog {
                Some(fog) => {
                    ui.label("fog source: weather");
                    fog_rows(ui, &fog);
                }
                None => {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "fog source: default (weather has no fog data)",
                    );
                }
            }
        }
        None => {
            ui.colored_label(ui.visuals().warn_fg_color, "no weather: default lighting");
        }
    }
}

fn interior_status(ui: &mut egui::Ui, lighting: &InteriorLighting) {
    match lighting {
        InteriorLighting::Cell(resolved) => {
            ui.label(format!("status: {}", resolved.template.label()));
            if let TemplateStatus::Unavailable { form_key, .. } = &resolved.template {
                ui.label(format!("template {form_key}"));
            }
            resolved_values(ui, resolved);
        }
        InteriorLighting::Neutral(reason) => {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("neutral fallback: {}", reason.label()),
            );
            if let NeutralReason::Unresolvable { detail, .. } = reason {
                ui.label(detail);
            }
        }
        InteriorLighting::CatalogUnavailable => {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "no lighting catalog: exterior lighting",
            );
        }
    }
}

fn resolved_values(ui: &mut egui::Ui, resolved: &ResolvedLighting) {
    let CellLighting {
        ambient_color,
        ambient_cube,
        directional,
        fog,
        clip_distance,
    } = &resolved.lighting;

    linear_row(ui, "ambient color", *ambient_color);
    ambient_cube_rows(ui, ambient_cube);
    directional_rows(ui, directional.as_ref());
    fog_rows(ui, fog);
    match clip_distance {
        Some(distance) => ui.label(format!("clip distance {distance:.1}")),
        None => ui.label("clip distance none"),
    };
}

fn ambient_cube_rows(ui: &mut egui::Ui, cube: &AmbientCube) {
    for (label, face) in AMBIENT_FACE_LABELS.iter().zip(cube.0) {
        linear_row(ui, &format!("ambient {label}"), face);
    }
}

fn directional_rows(ui: &mut egui::Ui, directional: Option<&DirectionalLight>) {
    match directional {
        Some(light) => {
            linear_row(ui, "directional color", light.color);
            ui.label(format!("directional fade {:.3}", light.fade));
            ui.label(format!(
                "directional to light ({:.3}, {:.3}, {:.3})",
                light.to_light.x, light.to_light.y, light.to_light.z
            ));
        }
        None => {
            ui.label("directional none");
        }
    }
}

fn fog_rows(ui: &mut egui::Ui, fog: &Fog) {
    match fog {
        Fog::Disabled => {
            ui.label("fog disabled");
        }
        Fog::Enabled(params) => {
            ui.label(format!(
                "fog near {:.1} far {:.1} power {:.2} max {:.2}",
                params.near, params.far, params.power, params.max
            ));
            linear_row(ui, "fog near color", params.near_color);
            linear_row(ui, "fog far color", params.far_color);
        }
    }
}

fn linear_row(ui: &mut egui::Ui, label: &str, color: Vec3) {
    swatch_row(
        ui,
        label,
        egui::Rgba::from_rgb(color.x, color.y, color.z).into(),
        color,
    );
}

fn swatch_row(ui: &mut egui::Ui, label: &str, swatch: egui::Color32, values: Vec3) {
    ui.horizontal(|ui| {
        egui::color_picker::show_color(ui, swatch, SWATCH_SIZE);
        ui.label(format!(
            "{label} ({:.3}, {:.3}, {:.3})",
            values.x, values.y, values.z
        ));
    });
}
