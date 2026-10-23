//! The game's two interface colours, read once at boot: the HUD colour (`iHUDColor*` with
//! `fHUDOpacity`), which the HUD, the main menu, the loading screen, the perk chart and the
//! screen-space hint bar use, and the Pip-Boy colour (`fPipboyEffectColor*`) the Pip-Boy pages use.

use crate::assets::ini::{GameIni, IniError, IniKey};

const HUD_R: IniKey = IniKey::new("Interface", "iHUDColorR");
const HUD_G: IniKey = IniKey::new("Interface", "iHUDColorG");
const HUD_B: IniKey = IniKey::new("Interface", "iHUDColorB");
const HUD_OPACITY: IniKey = IniKey::new("MAIN", "fHUDOpacity");
const PIPBOY_R: IniKey = IniKey::new("Pipboy", "fPipboyEffectColorR");
const PIPBOY_G: IniKey = IniKey::new("Pipboy", "fPipboyEffectColorG");
const PIPBOY_B: IniKey = IniKey::new("Pipboy", "fPipboyEffectColorB");

/// The largest value of a HUD colour channel; the channel is `value / HUD_CHANNEL_MAX`. The game
/// applies only values from 2 to 255; every whole number from 0 to 255 is accepted here.
const HUD_CHANNEL_MAX: i32 = 255;

/// Only what the HUD draws has an opacity setting; every other surface is fully opaque.
const OPAQUE: f32 = 1.0;

/// Which interface colour a surface is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiColourSet {
    /// The HUD colour, opaque: the main menu, the loading overlay, the perk chart and the
    /// screen-space hint bar.
    Hud,
    /// The HUD colour with `fHUDOpacity`: only what the HUD itself draws (meters, crosshair).
    HudOverlay,
    /// The Pip-Boy colour, opaque: the Pip-Boy pages.
    Pipboy,
}

/// Both interface colours, validated at startup.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiColours {
    hud: UiTint,
    hud_overlay: UiTint,
    pipboy: UiTint,
}

impl UiColours {
    /// The sets from the HUD colour with its opacity and the Pip-Boy colour; the opaque HUD set
    /// is the HUD colour at full opacity.
    pub const fn new(hud_overlay: UiTint, pipboy: UiTint) -> Self {
        Self {
            hud: UiTint {
                opacity: OPAQUE,
                ..hud_overlay
            },
            hud_overlay,
            pipboy,
        }
    }

    pub fn from_ini(ini: &GameIni) -> Result<Self, IniError> {
        let hud_channel = |key| -> Result<f32, IniError> {
            let value = ini.require_in_range::<i32>(key, 0..=HUD_CHANNEL_MAX)?;
            Ok(value as f32 / HUD_CHANNEL_MAX as f32)
        };
        let hud_overlay = UiTint {
            color: [
                hud_channel(HUD_R)?,
                hud_channel(HUD_G)?,
                hud_channel(HUD_B)?,
            ],
            opacity: ini.require_in_range(HUD_OPACITY, 0.0..=1.0)?,
        };
        let pipboy = UiTint {
            color: [
                ini.require_non_negative(PIPBOY_R)?,
                ini.require_non_negative(PIPBOY_G)?,
                ini.require_non_negative(PIPBOY_B)?,
            ],
            opacity: OPAQUE,
        };
        Ok(Self::new(hud_overlay, pipboy))
    }

    /// The tint every surface of `set` is drawn with. Only the HUD overlay set carries `fHUDOpacity`.
    pub fn tint_for(&self, set: UiColourSet) -> UiTint {
        match set {
            UiColourSet::Hud => self.hud,
            UiColourSet::HudOverlay => self.hud_overlay,
            UiColourSet::Pipboy => self.pipboy,
        }
    }
}

/// A colour-multiply and opacity; the uniform the HUD shader reads.
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct UiTint {
    pub color: [f32; 3],
    pub opacity: f32,
}

/// Linear (0..1) -> 8-bit sRGB, matching the GPU's automatic linear->sRGB encode
/// when the quad shader writes `color * tint` to the sRGB swapchain. Keeps text drawn
/// with a CPU colour the same on-screen colour as the bars.
pub fn linear_to_srgb_u8(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round().clamp(0.0, 255.0) as u8
}

impl UiTint {
    /// The tint as the 8-bit sRGB colour full-white art shows on screen.
    pub fn srgb_bytes(&self) -> [u8; 3] {
        self.color.map(linear_to_srgb_u8)
    }

    /// Full-white art as the egui colour it is painted with.
    pub fn srgb_color32(&self) -> egui::Color32 {
        let [r, g, b] = self.srgb_bytes();
        egui::Color32::from_rgb(r, g, b)
    }

    /// The tint as the colour of native text: 8-bit sRGB, with the HUD opacity as alpha. The
    /// bars are coloured by the shader in linear light and encoded by the sRGB surface; text
    /// goes through glyphon unencoded, so it is encoded here to look the same.
    pub fn text_rgba(&self) -> [u8; 4] {
        let [r, g, b] = self.srgb_bytes();
        [r, g, b, alpha_byte(self.opacity)]
    }

    /// `rgb` multiplied channel by channel with the tint, both as encoded 8-bit values: the colour
    /// transform Scaleform applies to a movie, so white becomes the tint and black stays black.
    pub fn apply_to(&self, rgb: [u8; 3]) -> [u8; 3] {
        let [tint_r, tint_g, tint_b, _] = self.text_rgba();
        let [r, g, b] = rgb;
        [
            multiply_encoded(r, tint_r),
            multiply_encoded(g, tint_g),
            multiply_encoded(b, tint_b),
        ]
    }

    /// Alpha of a black shadow that is `strength` (0 to 1) as strong as the text itself.
    pub fn shadow_alpha(&self, strength: f32) -> u8 {
        alpha_byte(self.opacity * strength)
    }
}

/// `channel` scaled by `tint`, both encoded 8-bit values: the one channel multiply of every
/// tinted colour.
pub fn multiply_encoded(channel: u8, tint: u8) -> u8 {
    (f32::from(channel) * f32::from(tint) / 255.0).round() as u8
}

fn alpha_byte(alpha: f32) -> u8 {
    (255.0 * alpha).round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are the exact literals parsed from the INI text"
)]
mod tests {
    use super::*;
    use crate::rendering::test_support::{FULL_INI, ini_with};

    #[test]
    fn text_colour_encodes_linear_channels_to_srgb_and_carries_the_opacity_as_alpha() {
        let tint = UiTint {
            color: [0.0, 1.0, 0.215_86],
            opacity: 0.5,
        };
        assert_eq!(tint.text_rgba(), [0, 255, 128, 128]);
    }

    #[test]
    fn text_colour_clamps_a_bright_tint_to_the_displayable_range() {
        let tint = UiTint {
            color: [1.5, 0.0, 0.0],
            opacity: 1.0,
        };
        assert_eq!(tint.text_rgba(), [255, 0, 0, 255]);
    }

    #[test]
    fn white_maps_to_the_tint_and_black_stays_black() {
        let green = UiTint {
            color: [0.0, 1.0, 0.0],
            opacity: 1.0,
        };
        assert_eq!(green.apply_to([255, 255, 255]), [0, 255, 0]);
        assert_eq!(green.apply_to([0, 0, 0]), [0, 0, 0]);
    }

    #[test]
    fn a_grey_is_multiplied_with_the_tint_channel_by_channel() {
        let tint = UiTint {
            color: [1.0, 0.5, 0.0],
            opacity: 1.0,
        };
        let [r, g, b] = tint.apply_to([128, 128, 128]);
        assert_eq!(r, 128);
        assert_eq!(
            g,
            (f32::from(128u8) * f32::from(tint.text_rgba()[1]) / 255.0).round() as u8
        );
        assert_eq!(b, 0);
    }

    #[test]
    fn the_shadow_alpha_scales_the_opacity_by_the_strength() {
        let tint = UiTint {
            color: [1.0; 3],
            opacity: 0.8,
        };
        assert_eq!(tint.shadow_alpha(0.5), 102);
        assert_eq!(tint.shadow_alpha(0.0), 0);
        assert_eq!(tint.shadow_alpha(1.0), 204);
    }

    fn hud_ini(extra: &str) -> GameIni {
        ini_with(&format!(
            "iHUDColorR=51
iHUDColorG=102
iHUDColorB=255
fHUDOpacity=0.4
{extra}"
        ))
    }

    #[test]
    fn each_set_reads_its_own_keys() {
        let colours = UiColours::from_ini(&ini_with(
            "iHUDColorR=51
iHUDColorG=102
iHUDColorB=255
fHUDOpacity=0.4
             fPipboyEffectColorR=0.1
fPipboyEffectColorG=0.2
fPipboyEffectColorB=0.3
",
        ))
        .unwrap();
        assert_eq!(
            colours.tint_for(UiColourSet::HudOverlay),
            UiTint {
                color: [0.2, 0.4, 1.0],
                opacity: 0.4
            }
        );
        assert_eq!(
            colours.tint_for(UiColourSet::Pipboy),
            UiTint {
                color: [0.1, 0.2, 0.3],
                opacity: 1.0
            }
        );
    }

    #[test]
    fn the_hud_channels_are_whole_numbers_over_255() {
        let colours = UiColours::from_ini(&hud_ini(
            "iHUDColorG=0
",
        ))
        .unwrap();
        assert_eq!(
            colours.tint_for(UiColourSet::HudOverlay).color,
            [0.2, 0.0, 1.0]
        );
    }

    #[test]
    fn a_missing_key_is_an_error_naming_it() {
        for key in [
            "iHUDColorR",
            "iHUDColorG",
            "iHUDColorB",
            "fHUDOpacity",
            "fPipboyEffectColorR",
            "fPipboyEffectColorG",
            "fPipboyEffectColorB",
        ] {
            let kept: Vec<_> = FULL_INI
                .lines()
                .filter(|line| line.split_once('=').is_none_or(|(name, _)| name != key))
                .collect();
            let error = UiColours::from_ini(&GameIni::from_texts([kept
                .join(
                    "
",
                )
                .as_str()]))
            .unwrap_err();
            assert!(
                matches!(&error, IniError::MissingKey { key: missing, .. } if *missing == key),
                "{key}: {error}"
            );
        }
    }

    #[test]
    fn invalid_values_are_errors_naming_the_key() {
        for (text, key) in [
            (
                "iHUDColorR=256
",
                "iHUDColorR",
            ),
            (
                "iHUDColorG=-1
",
                "iHUDColorG",
            ),
            (
                "iHUDColorB=0.5
",
                "iHUDColorB",
            ),
            (
                "iHUDColorB=blue
",
                "iHUDColorB",
            ),
            (
                "fPipboyEffectColorR=NaN
",
                "fPipboyEffectColorR",
            ),
            (
                "fPipboyEffectColorG=inf
",
                "fPipboyEffectColorG",
            ),
            (
                "fPipboyEffectColorB=green
",
                "fPipboyEffectColorB",
            ),
            (
                "fPipboyEffectColorR=-0.1
",
                "fPipboyEffectColorR",
            ),
            (
                "fHUDOpacity=opaque
",
                "fHUDOpacity",
            ),
            (
                "fHUDOpacity=NaN
",
                "fHUDOpacity",
            ),
            (
                "fHUDOpacity=-0.5
",
                "fHUDOpacity",
            ),
            (
                "fHUDOpacity=1.5
",
                "fHUDOpacity",
            ),
        ] {
            let error = UiColours::from_ini(&hud_ini(text)).unwrap_err();
            assert!(error.to_string().contains(key), "{text}: {error}");
        }
    }

    #[test]
    fn the_bounds_and_a_bright_pipboy_colour_are_accepted() {
        for (channel, opacity) in [(0, 0.0), (255, 1.0)] {
            let ini = hud_ini(&format!(
                "iHUDColorR={channel}
fHUDOpacity={opacity}
"
            ));
            let hud = UiColours::from_ini(&ini)
                .unwrap()
                .tint_for(UiColourSet::HudOverlay);
            assert_eq!(
                (hud.color[0], hud.opacity),
                (channel as f32 / 255.0, opacity)
            );
        }
        let pipboy = UiColours::from_ini(&hud_ini(
            "fPipboyEffectColorG=1.5
",
        ))
        .unwrap()
        .tint_for(UiColourSet::Pipboy);
        assert_eq!(pipboy.color[1], 1.5);
    }

    #[test]
    fn only_the_hud_overlay_set_carries_the_hud_opacity() {
        let colours = UiColours::from_ini(&hud_ini("")).unwrap();
        let overlay = colours.tint_for(UiColourSet::HudOverlay);
        let menus = colours.tint_for(UiColourSet::Hud);
        assert_eq!(overlay.opacity, 0.4);
        assert_eq!(menus.color, overlay.color);
        assert_eq!(menus.opacity, 1.0);
        assert_eq!(menus.text_rgba()[3], 255);
        assert_eq!(menus.shadow_alpha(1.0), 255);
        assert_eq!(colours.tint_for(UiColourSet::Pipboy).opacity, 1.0);
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the installed INI cascade"]
    fn the_installed_ini_cascade_gives_both_colours() {
        let install = std::env::var(crate::assets::INSTALL_DIR_VAR).expect("OPENFO4_INSTALL_DIR");
        let data = std::path::Path::new(&install)
            .join(crate::assets::layout::ASSETS_DIR)
            .join(crate::assets::layout::DATA_DIR);
        let colours = UiColours::from_ini(&GameIni::load(&data).unwrap()).unwrap();
        println!(
            "hud {:?} pipboy {:?}",
            colours.tint_for(UiColourSet::Hud),
            colours.tint_for(UiColourSet::Pipboy)
        );
    }
}
