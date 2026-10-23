use std::str::FromStr;

use crate::rendering::water::WaterDebugOutput;
use crate::utils;

pub const DEBUG_VIEW_ENV: &str = "OPENFO4_DEBUG_VIEW";

/// Mip from which the tint is fully green.
const MIP_FULL_GREEN: u32 = 4;
const GREEN_HUE: f32 = 1.0 / 3.0;
const MIP_TINT_STRENGTH: f32 = 0.65;
/// Tint of slots still bound to a stand-in texture; outside the red-green ramp.
const PLACEHOLDER_TINT: [f32; 3] = [1.0, 0.35, 1.0];
/// Tint of slots without feedback data in the texture waste view; mirrors
/// `NO_FEEDBACK_TINT` in `shaders/mip_debug.wgsl`.
const NO_FEEDBACK_TINT: [f32; 3] = [0.35, 0.35, 1.0];

/// The water output of a table row: `@ Output` names the one the water pass draws, no mark
/// means the water pass takes no part.
macro_rules! water_output_of {
    () => {
        None
    };
    ($output:ident) => {
        Some(WaterDebugOutput::$output)
    };
}

macro_rules! debug_views {
    ($($name:ident => $label:literal $(@ $output:ident)?),+ $(,)?) => {
        /// The one scene-altering view the renderer shows; selecting a variant
        /// replaces whichever view was active. The discriminants (declaration
        /// order) are mirrored by the `DEBUG_VIEW_*` constants in
        /// `shaders/lighting.wgsl`, one `const DEBUG_VIEW_<LABEL> = <n>u;` line
        /// per view that `shade_surface` handles. Adding a view is one line in the table below:
        /// the selector and the `OPENFO4_DEBUG_VIEW` parser both read `ALL`, and a
        /// water view names its output there.
        #[derive(Debug, Copy, Clone, PartialEq, Eq)]
        #[repr(u32)]
        pub enum DebugView {
            $($name),+
        }

        impl DebugView {
            pub const ALL: &'static [DebugView] = &[$(DebugView::$name),+];

            pub fn label(self) -> &'static str {
                match self {
                    $(DebugView::$name => $label),+
                }
            }

            /// What the water pass draws in place of its shading while this view is
            /// active; `None` for every view the water pass does not take part in.
            pub fn water_output(self) -> Option<WaterDebugOutput> {
                match self {
                    $(DebugView::$name => water_output_of!($($output)?)),+
                }
            }
        }
    };
}

debug_views! {
    Off => "Off",
    Albedo => "Albedo",
    Normals => "Normals",
    LightingOnly => "Lighting only",
    SpecularOnly => "Specular only",
    ShadowCascades => "Shadow cascades",
    LightCountHeatmap => "Light count heatmap",
    LocalLightsOnly => "Local lights only",
    LampShadowSlots => "Lamp shadow slots",
    StreamedMip => "Streamed mip",
    SampledMip => "Sampled mip",
    MipDeficit => "Mip deficit",
    TextureWaste => "Texture waste",
    EnvReflection => "Env reflection",
    EffectClass => "Effect class",
    WaterCoverage => "Water coverage" @ Coverage,
    WaterSceneCopy => "Water scene copy" @ SceneCopy,
    WaterDepthFog => "Water depthfog" @ DepthFog,
    WaterDisplacement => "Water displacement" @ Displacement,
    WaterSsr => "Water SSR" @ Ssr,
}

impl DebugView {
    pub fn is_mip_view(self) -> bool {
        matches!(
            self,
            DebugView::StreamedMip
                | DebugView::SampledMip
                | DebugView::MipDeficit
                | DebugView::TextureWaste
        )
    }

    /// One line on what a water view shows, for the developer menu.
    fn water_description(self) -> Option<&'static str> {
        Some(match self.water_output()? {
            WaterDebugOutput::Coverage => "magenta wherever the water pass draws a surface",
            WaterDebugOutput::SceneCopy => {
                "the opaque scene copy the water refracts (already tinted by the depth fog), at the pixel and without the refraction offset"
            }
            WaterDebugOutput::DepthFog => {
                "colour of the surface's depth-fog pass times the alpha the pass blends it with (the pass composited over black, so brighter means more fog); surfaces without that pass are not drawn"
            }
            WaterDebugOutput::Displacement => {
                "wading displacement field the ripple lookup reads: red = surface above rest, blue = below, green = velocity; black where it is at rest or not bound"
            }
            WaterDebugOutput::Ssr => {
                "sharp screen-space reflection tap weighted by its coverage; black where the ray missed or the chain does not run"
            }
        })
    }

    /// One line on what an effect view shows, for the developer menu.
    fn effect_description(self) -> Option<&'static str> {
        match self {
            DebugView::EnvReflection => Some(
                "only the environment reflection term (cube lookup times fresnel, mask and exposure); black without an environment cube",
            ),
            DebugView::EffectClass => Some(
                "flat tint per material class: alpha-blend effect magenta, additive yellow, multiply cyan, opaque effect green, lit with environment map blue, everything else dark grey",
            ),
            _ => None,
        }
    }

    fn mip_description(self) -> &'static str {
        match self {
            DebugView::StreamedMip => {
                "first resident mip of the diffuse texture (0 = full resolution), as a tint"
            }
            DebugView::MipDeficit => {
                "per pixel: resident mip against the mip this pixel samples, as a tint"
            }
            DebugView::TextureWaste => {
                "per texture: resident mip against the finest mip any visible pixel samples of it, as a tint"
            }
            _ => {
                "mip level the GPU samples, in full-resolution terms, as a tint (approximate under anisotropic filtering)"
            }
        }
    }

    /// The view named by `OPENFO4_DEBUG_VIEW`, else `Off`.
    pub fn initial() -> Self {
        utils::env_opt(DEBUG_VIEW_ENV).unwrap_or(DebugView::Off)
    }
}

fn normalised_name(name: &str) -> String {
    name.trim()
        .chars()
        .map(|c| {
            if matches!(c, '_' | '-') {
                ' '
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect()
}

impl FromStr for DebugView {
    type Err = String;

    /// Matches the label case-insensitively, `_` and `-` standing for spaces.
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        let wanted = normalised_name(name);
        DebugView::ALL
            .iter()
            .copied()
            .find(|view| normalised_name(view.label()) == wanted)
            .ok_or_else(|| {
                let valid: Vec<_> = DebugView::ALL.iter().map(|view| view.label()).collect();
                format!("unknown debug view; expected one of {}", valid.join(", "))
            })
    }
}

/// The selector and, for the mip views, their colour legend.
pub fn debug_view_section(ui: &mut egui::Ui, view: &mut DebugView, texture_feedback_enabled: bool) {
    egui::ComboBox::from_label("view")
        .selected_text(view.label())
        .show_ui(ui, |ui| {
            for &candidate in DebugView::ALL {
                ui.selectable_value(view, candidate, candidate.label());
            }
        });
    if view.is_mip_view() {
        ui.label(view.mip_description());
        match view {
            DebugView::MipDeficit => deficit_legend(ui),
            DebugView::TextureWaste => waste_legend(ui, texture_feedback_enabled),
            _ => mip_legend(ui),
        }
    } else if let Some(description) = view.water_description().or(view.effect_description()) {
        ui.label(description);
    }
}

fn mip_legend(ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        for mip in 0..=MIP_FULL_GREEN {
            let label = if mip == MIP_FULL_GREEN {
                format!("{mip}+")
            } else {
                mip.to_string()
            };
            swatch(ui, linear_swatch(mip_tint(mip as f32)), &label);
        }
        swatch(ui, linear_swatch(PLACEHOLDER_TINT), "placeholder");
    });
}

fn slack_swatches(ui: &mut egui::Ui) {
    swatch(ui, linear_swatch(mip_tint(0.0)), "too coarse");
    swatch(
        ui,
        linear_swatch(mip_tint(MIP_FULL_GREEN as f32)),
        "tight (0-1 finer)",
    );
    swatch(
        ui,
        linear_swatch(mip_tint(MIP_FULL_GREEN as f32 * 0.5)),
        "2+ finer",
    );
    swatch(ui, linear_swatch(PLACEHOLDER_TINT), "placeholder");
}

fn deficit_legend(ui: &mut egui::Ui) {
    ui.horizontal_wrapped(slack_swatches);
}

fn waste_legend(ui: &mut egui::Ui, texture_feedback_enabled: bool) {
    if !texture_feedback_enabled {
        ui.label(
            "texture feedback is disabled (OPENFO4_TEX_FEEDBACK=0): every texture shows as no data",
        );
    }
    ui.horizontal_wrapped(|ui| {
        slack_swatches(ui);
        swatch(ui, linear_swatch(NO_FEEDBACK_TINT), "no feedback data");
    });
}

pub(super) fn swatch(ui: &mut egui::Ui, color: egui::Color32, label: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 2.0, color);
        ui.label(label);
    });
}

fn linear_swatch(rgb: [f32; 3]) -> egui::Color32 {
    egui::Color32::from(egui::Rgba::from_rgb(rgb[0], rgb[1], rgb[2]))
}

/// Mirrors `mip_tint` in `shaders/mip_debug.wgsl`: red at mip 0 through yellow
/// to green from `MIP_FULL_GREEN`, softened towards white like the cascade tint.
fn mip_tint(mip: f32) -> [f32; 3] {
    let hue = (mip / MIP_FULL_GREEN as f32).clamp(0.0, 1.0) * GREEN_HUE;
    let rgb = egui::ecolor::Hsva::new(hue, 1.0, 1.0, 1.0).to_rgb();
    rgb.map(|channel| 1.0 + (channel - 1.0) * MIP_TINT_STRENGTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIGHTING_SHADER: &str = include_str!("shaders/lighting.wgsl");

    #[test]
    fn discriminants_follow_the_order_of_all() {
        for (index, view) in DebugView::ALL.iter().enumerate() {
            assert_eq!(*view as usize, index, "{view:?}");
        }
    }

    #[test]
    fn every_listed_view_has_a_distinct_label_that_parses_back_to_it() {
        for &view in DebugView::ALL {
            assert_eq!(view.label().parse::<DebugView>(), Ok(view), "{view:?}");
            assert_eq!(
                view.label().replace(' ', "_").parse::<DebugView>(),
                Ok(view),
                "{view:?} by its environment spelling"
            );
        }
    }

    #[test]
    fn env_names_resolve_to_the_views() {
        let expected = [
            ("Albedo", DebugView::Albedo),
            ("lamp_shadow_slots", DebugView::LampShadowSlots),
            ("mip-deficit", DebugView::MipDeficit),
            ("water_coverage", DebugView::WaterCoverage),
            ("water_scene_copy", DebugView::WaterSceneCopy),
            ("water_depthfog", DebugView::WaterDepthFog),
            ("water_displacement", DebugView::WaterDisplacement),
            ("WATER_SSR", DebugView::WaterSsr),
            ("env_reflection", DebugView::EnvReflection),
            ("Effect-Class", DebugView::EffectClass),
        ];
        for (name, view) in expected {
            assert_eq!(name.parse::<DebugView>(), Ok(view), "{name}");
        }
    }

    #[test]
    fn an_unknown_name_lists_the_valid_ones() {
        let error = "nonsense".parse::<DebugView>().unwrap_err();
        for &view in DebugView::ALL {
            assert!(error.contains(view.label()), "{error}");
        }
    }

    #[test]
    fn every_water_output_is_selected_by_exactly_one_view() {
        for output in WaterDebugOutput::ALL {
            let views = DebugView::ALL
                .iter()
                .filter(|view| view.water_output() == Some(output))
                .count();
            assert_eq!(views, 1, "{output:?}");
        }
    }

    #[test]
    fn every_water_view_describes_itself_and_no_other_view_does() {
        for &view in DebugView::ALL {
            assert_eq!(
                view.water_description().is_some(),
                view.water_output().is_some(),
                "{view:?}"
            );
        }
    }

    const CONSTANT_CONTRACT: &str = "shaders/lighting.wgsl must declare one `const DEBUG_VIEW_<LABEL> = <discriminant>u;` line per view `shade_surface` handles";

    fn parse_constant(line: &str) -> Option<(&str, u32)> {
        let declaration = line.trim().strip_prefix("const DEBUG_VIEW_")?;
        let (head, value) = declaration.split_once('=')?;
        let name = head.split(':').next()?.trim();
        let value = value.trim().strip_suffix(';')?.trim_end();
        Some((name, value.strip_suffix('u').unwrap_or(value).parse().ok()?))
    }

    fn lighting_shader_constants() -> Vec<(&'static str, u32)> {
        LIGHTING_SHADER.lines().filter_map(parse_constant).collect()
    }

    #[test]
    fn a_constant_declaration_parses_whatever_its_spacing_and_type_annotation() {
        for line in [
            "const DEBUG_VIEW_ALBEDO = 1u;",
            "  const DEBUG_VIEW_ALBEDO=1u ;",
            "const DEBUG_VIEW_ALBEDO: u32 = 1u;",
            "const DEBUG_VIEW_ALBEDO = 1;",
        ] {
            assert_eq!(parse_constant(line), Some(("ALBEDO", 1)), "{line}");
        }
        for line in [
            "const DEBUG_VIEW_ALBEDO = one;",
            "const DEBUG_VIEW_ALBEDO = 1u",
            "const OTHER_ALBEDO = 1u;",
            "// const DEBUG_VIEW_ALBEDO = 1u;",
        ] {
            assert_eq!(parse_constant(line), None, "{line}");
        }
    }

    #[test]
    fn the_lighting_shader_mirrors_the_discriminants_of_the_views_it_shades() {
        let constants = lighting_shader_constants();
        for &view in DebugView::ALL {
            let name = view.label().to_ascii_uppercase().replace(' ', "_");
            let mirrored = constants.iter().find(|(constant, _)| *constant == name);
            if view == DebugView::Off || view.water_output().is_some() {
                assert_eq!(mirrored, None, "{view:?} is not shaded by shade_surface");
            } else {
                assert_eq!(
                    mirrored.map(|(_, value)| *value),
                    Some(view as u32),
                    "DEBUG_VIEW_{name}: {CONSTANT_CONTRACT}"
                );
            }
        }
        assert_eq!(
            constants.len(),
            DebugView::ALL
                .iter()
                .filter(|view| **view != DebugView::Off && view.water_output().is_none())
                .count(),
            "a DEBUG_VIEW_* constant without a view: {constants:?}; {CONSTANT_CONTRACT}"
        );
    }
}
