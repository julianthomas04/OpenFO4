//! Parameters of Fallout 4's bokeh depth of field effect: the image space's depth of field
//! block, which image space the effect takes it from, and the `cb2` registers every pass reads.

use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec4};
use serde::{Deserialize, Deserializer};

use super::dof_resolve::DofResolve;
use crate::assets::ini::{GameIni, IniError, IniKey};

const SCREEN_SPACE_BOKEH: IniKey = IniKey::new("Imagespace", "bScreenSpaceBokeh");

/// The depth of field block of an IMGS record as the installer exports it
/// (`DepthOfField` of `image_spaces.data`): the `DNAM` subrecord's strength,
/// distance, range, mode float and the optional vignette.
///
/// The mode is `& 3` 1 for the front and 2 for the back mode (3 is invalid and
/// disables the effect), bit 4 is no-sky, `>> 3` the blur radius index.
#[derive(Debug, Copy, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ImageSpaceDepthOfField {
    pub strength: f32,
    pub distance: f32,
    pub range: f32,
    /// A database that predates the mode export has none (0: no mode adjustments).
    #[serde(default, deserialize_with = "mode_from_float")]
    pub mode: u32,
    /// Percent of the half diagonal; absent in IMGS records with a 16 byte `DNAM`.
    #[serde(default)]
    pub vignette_radius: f32,
    #[serde(default)]
    pub vignette_strength: f32,
}

/// The mode is stored as a float and truncated to an integer.
fn mode_from_float<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    Ok(f32::deserialize(deserializer)? as u32)
}

#[derive(Deserialize)]
struct ImageSpaceRow {
    #[serde(rename = "DepthOfField")]
    depth_of_field: Option<ImageSpaceDepthOfField>,
}

/// The depth of field block of an `image_spaces.data` JSON, `None` when the database
/// predates the depth of field export.
pub fn parse_image_space(json: &str) -> Result<Option<ImageSpaceDepthOfField>, serde_json::Error> {
    Ok(serde_json::from_str::<ImageSpaceRow>(json)?.depth_of_field)
}

const MODE_FRONT: u32 = 1;
const MODE_BACK: u32 = 2;
const MODE_MASK: u32 = 3;
const MODE_INVALID: u32 = 3;
const MODE_NO_SKY: u32 = 4;
/// Modes below this have the default blur radius index, above it the index is `mode >> 3`.
const MODE_RADIUS_START: u32 = 8;
const MODE_RADIUS_SHIFT: u32 = 3;
const DEFAULT_RADIUS_INDEX: f32 = 3.0;
/// Register 2 `x` is the vignette radius in percent of the half diagonal.
const VIGNETTE_RADIUS_SCALE: f32 = 0.01;
/// The focus band is five percent of the range on each side of the distance.
const FOCUS_BAND: f32 = 0.05;
/// Ranges below one are raised to one (`R = max(range, 1)`).
const MIN_RANGE: f32 = 1.0;
/// The horizontal blur pass steps two texels; the diagonal pass steps
/// `(2 / width * 0.57357645, 1.63830411 / height)`.
const BLUR_STEP_TEXELS: f32 = 2.0;
const DIAGONAL_STEP_X_SCALE: f32 = 0.573_576_45;
const DIAGONAL_STEP_Y_TEXELS: f32 = 1.638_304_1;

impl ImageSpaceDepthOfField {
    /// The vignette this image space carries. The effect only ever uses the
    /// image-space manager's, i.e. the weather's.
    pub fn vignette(&self) -> Vignette {
        Vignette {
            radius: self.vignette_radius,
            strength: self.vignette_strength,
        }
    }

    /// The effect skips itself when the mode is invalid or the strength zero,
    /// unless the manager's vignette is on.
    pub fn is_active(&self, vignette: Vignette) -> bool {
        vignette.is_on() || (self.mode & MODE_MASK != MODE_INVALID && self.strength != 0.0)
    }

    fn blur_radius_index(&self) -> f32 {
        if self.mode < MODE_RADIUS_START {
            DEFAULT_RADIUS_INDEX
        } else {
            (self.mode >> MODE_RADIUS_SHIFT) as f32
        }
    }
}

/// The render camera's clip distances in game units.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct DofCamera {
    pub near: f32,
    pub far: f32,
}

/// The vignette of the image-space manager, which the depth of field effect always takes
/// from there: the weather's image space.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Vignette {
    pub radius: f32,
    pub strength: f32,
}

impl Vignette {
    pub const NONE: Self = Self {
        radius: 0.0,
        strength: 0.0,
    };

    fn is_on(self) -> bool {
        self.strength > 0.0
    }
}

/// Depth-decoding constants of `cb2[3]`, chosen so the decoded depth equals `near / d` for the
/// engine's reverse-Z depth `d`.
const DEPTH_LINEARISATION_B: f32 = 0.5;
const DEPTH_LINEARISATION_C: f32 = -0.5;

/// `cb2[0..=3]` of every pass. `cb2[3]` is `(near, B, C, far)` with the constants above: the
/// engine's reverse-Z depth is linearised from the near distance, not from a standard
/// projection's `(2nf, f + n, f - n, f)`. `target_size` is the scene target in pixels.
pub fn registers(
    dof: &ImageSpaceDepthOfField,
    vignette: Vignette,
    camera: DofCamera,
    target_size: Vec2,
) -> [Vec4; 4] {
    let range = dof.range.max(MIN_RANGE);
    let mut focus = Vec4::new(
        dof.distance - FOCUS_BAND * range,
        dof.distance + FOCUS_BAND * range,
        range,
        range,
    );
    match dof.mode & MODE_MASK {
        MODE_FRONT => focus.y = camera.far,
        MODE_BACK => focus.x = 0.0,
        _ => {}
    }
    let horizontal_step = BLUR_STEP_TEXELS / target_size.x;
    [
        focus,
        Vec4::new(
            horizontal_step,
            horizontal_step * DIAGONAL_STEP_X_SCALE,
            DIAGONAL_STEP_Y_TEXELS / target_size.y,
            dof.blur_radius_index(),
        ),
        Vec4::new(
            vignette.radius * VIGNETTE_RADIUS_SCALE,
            vignette.strength,
            dof.strength,
            f32::from(u8::from(dof.mode & MODE_NO_SKY != 0)),
        ),
        Vec4::new(
            camera.near,
            DEPTH_LINEARISATION_B,
            DEPTH_LINEARISATION_C,
            camera.far,
        ),
    ]
}

/// The uniform block every pass of the effect reads: `cb2[0..=3]` from
/// [`registers`] and, for the fogged final pass, the fog colour and distances.
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Pod, Zeroable)]
pub struct DofRegisters {
    pub cb2: [Vec4; 6],
}

impl DofRegisters {
    /// `dof` are `cb2[0..=3]`, `fog` `cb2[4..=5]`.
    pub fn fogged(dof: [Vec4; 4], fog: [Vec4; 2]) -> Self {
        Self {
            cb2: [dof[0], dof[1], dof[2], dof[3], fog[0], fog[1]],
        }
    }

    /// The fog rows stay zero: the plain final pass does not read them.
    pub fn plain(dof: [Vec4; 4]) -> Self {
        Self::fogged(dof, [Vec4::ZERO; 2])
    }
}

/// What the camera being fully under water changes: the water's image space replaces the
/// weather's depth of field (strength, distance, range, mode) and the final pass
/// fogs.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct UnderwaterOverride {
    pub dof: ImageSpaceDepthOfField,
    /// `cb2[4..=5]`: the fog colour and distances.
    pub fog: [Vec4; 2],
}

/// One frame of the effect: the registers every pass reads and the final pass to run.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct DofFrame {
    pub registers: DofRegisters,
    pub resolve: DofResolve,
}

/// The effect's activation gate: the INI setting `bScreenSpaceBokeh:ImageSpace`, an applied
/// image space modifier with a depth of field, and no open menu that blurs the world (barter,
/// pause, workbench, book, container, lockpicking). The weather's depth of field is therefore
/// not drawn in normal gameplay; the game's legacy depth-of-field effect takes it over there.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct BokehGate {
    pub setting_enabled: bool,
    /// An image space modifier with a depth of field is applied.
    pub modifier_dof_applied: bool,
    /// The count of open menus that blur the world.
    pub blurring_menus: u32,
}

impl BokehGate {
    /// Normal gameplay: no image space modifier with a depth of field, no menu.
    pub const fn gameplay(setting_enabled: bool) -> Self {
        Self {
            setting_enabled,
            modifier_dof_applied: false,
            blurring_menus: 0,
        }
    }

    /// Normal gameplay with `bScreenSpaceBokeh` read from the INI cascade.
    pub fn gameplay_from_ini(ini: &GameIni) -> Result<Self, IniError> {
        ini.require_flag(SCREEN_SPACE_BOKEH).map(Self::gameplay)
    }

    pub fn is_open(self) -> bool {
        self.setting_enabled && self.modifier_dof_applied && self.blurring_menus == 0
    }
}

/// The effect of this frame, `None` when it skips itself. `manager` is the
/// weather's image space: its depth of field only applies while `gate` is open
/// (never in normal gameplay), its vignette only rides along with the water's
/// override. `underwater` replaces the depth of field while the camera's near
/// plane is fully under water and runs with the gate treated as open.
pub fn select_frame(
    manager: Option<&ImageSpaceDepthOfField>,
    underwater: Option<&UnderwaterOverride>,
    gate: BokehGate,
    camera: DofCamera,
    target_size: Vec2,
) -> Option<DofFrame> {
    let vignette = manager.map_or(Vignette::NONE, ImageSpaceDepthOfField::vignette);
    let (dof, fog) = match underwater {
        Some(overridden) => (&overridden.dof, Some(overridden.fog)),
        None if gate.is_open() => (manager?, None),
        None => return None,
    };
    if !dof.is_active(vignette) {
        return None;
    }
    let dof_registers = registers(dof, vignette, camera, target_size);
    Some(match fog {
        Some(fog) => DofFrame {
            registers: DofRegisters::fogged(dof_registers, fog),
            resolve: DofResolve::Fogged,
        },
        None => DofFrame {
            registers: DofRegisters::plain(dof_registers),
            resolve: DofResolve::Plain,
        },
    })
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "registers are exact products of fixture values"
)]
mod tests {
    use super::*;

    /// 023813 `UnderwaterDefaultIS` as the ESM holds it.
    const UNDERWATER_DEFAULT: ImageSpaceDepthOfField = ImageSpaceDepthOfField {
        strength: 1.0,
        distance: -2500.0,
        range: 3500.0,
        mode: 26,
        vignette_radius: 0.0,
        vignette_strength: 0.0,
    };
    const CAMERA: DofCamera = DofCamera {
        near: 15.0,
        far: 353_840.0,
    };
    const TARGET: Vec2 = Vec2::new(1920.0, 1080.0);
    const FOG: [Vec4; 2] = [
        Vec4::new(0.1, 0.2, 0.3, 1.0),
        Vec4::new(1700.0, -6400.0, 1.0, 0.0),
    ];

    fn dof_registers(dof: &ImageSpaceDepthOfField) -> [Vec4; 4] {
        registers(dof, Vignette::NONE, CAMERA, TARGET)
    }

    #[test]
    fn the_exported_block_parses_and_an_old_database_has_none() {
        let json = r#"{"Hdr":{},"DepthOfField":{"Strength":1.0,"Distance":-2500.0,"Range":3500.0,"Mode":26.0,"VignetteRadius":0.0,"VignetteStrength":0.0}}"#;
        assert_eq!(parse_image_space(json).unwrap(), Some(UNDERWATER_DEFAULT));
        assert_eq!(parse_image_space(r#"{"Hdr":{}}"#).unwrap(), None);
        assert!(parse_image_space("not json").is_err());
    }

    #[test]
    fn a_block_without_mode_and_vignette_defaults_to_none_of_them() {
        let json = r#"{"DepthOfField":{"Strength":1.0,"Distance":-2500.0,"Range":3500.0}}"#;
        let dof = parse_image_space(json).unwrap().unwrap();
        assert_eq!(dof.mode, 0);
        assert_eq!(dof.vignette(), Vignette::NONE);
    }

    #[test]
    fn the_mode_float_is_truncated_and_the_vignette_read() {
        let json = r#"{"DepthOfField":{"Strength":0.5,"Distance":1.0,"Range":2.0,"Mode":62.0,"VignetteRadius":50.0,"VignetteStrength":0.25}}"#;
        let dof = parse_image_space(json).unwrap().unwrap();
        assert_eq!(dof.mode, 62);
        assert_eq!(
            dof.vignette(),
            Vignette {
                radius: 50.0,
                strength: 0.25
            }
        );
    }

    #[test]
    fn the_back_mode_clears_the_near_blur_start() {
        assert_eq!(
            dof_registers(&UNDERWATER_DEFAULT)[0],
            Vec4::new(0.0, -2500.0 + 0.05 * 3500.0, 3500.0, 3500.0)
        );
    }

    #[test]
    fn the_front_mode_moves_the_far_blur_start_to_the_far_plane() {
        let dof = ImageSpaceDepthOfField {
            mode: 1,
            ..UNDERWATER_DEFAULT
        };
        let focus = dof_registers(&dof)[0];
        assert_eq!(focus.x, -2500.0 - 0.05 * 3500.0);
        assert_eq!(focus.y, CAMERA.far);
    }

    #[test]
    fn the_other_modes_keep_the_symmetric_focus_band() {
        let dof = ImageSpaceDepthOfField {
            mode: 0,
            ..UNDERWATER_DEFAULT
        };
        let focus = dof_registers(&dof)[0];
        assert_eq!(focus.x, -2500.0 - 175.0);
        assert_eq!(focus.y, -2500.0 + 175.0);
    }

    #[test]
    fn a_range_below_one_is_raised_to_one() {
        let dof = ImageSpaceDepthOfField {
            range: 0.25,
            mode: 0,
            ..UNDERWATER_DEFAULT
        };
        let focus = dof_registers(&dof)[0];
        assert_eq!((focus.z, focus.w), (1.0, 1.0));
    }

    #[test]
    fn register_one_holds_the_blur_steps_and_the_radius_index() {
        let texel = 2.0 / 1920.0;
        assert_eq!(
            dof_registers(&UNDERWATER_DEFAULT)[1],
            Vec4::new(texel, texel * 0.573_576_45, 1.638_304_1 / 1080.0, 3.0)
        );
    }

    #[test]
    fn the_radius_index_is_the_mode_over_eight_from_mode_eight_up() {
        let index = |mode| {
            dof_registers(&ImageSpaceDepthOfField {
                mode,
                ..UNDERWATER_DEFAULT
            })[1]
                .w
        };
        assert_eq!(index(0), 3.0);
        assert_eq!(index(7), 3.0);
        assert_eq!(index(8), 1.0);
        assert_eq!(index(18), 2.0);
        assert_eq!(index(34), 4.0);
        assert_eq!(index(62), 7.0);
    }

    #[test]
    fn register_two_carries_the_vignette_strength_and_the_no_sky_flag() {
        let dof = ImageSpaceDepthOfField {
            mode: 2 | 4,
            ..UNDERWATER_DEFAULT
        };
        let vignette = Vignette {
            radius: 50.0,
            strength: 0.5,
        };
        assert_eq!(
            registers(&dof, vignette, CAMERA, TARGET)[2],
            Vec4::new(0.5, 0.5, 1.0, 1.0)
        );
        assert_eq!(
            dof_registers(&UNDERWATER_DEFAULT)[2],
            Vec4::new(0.0, 0.0, 1.0, 0.0),
            "mode 26 has no no-sky bit"
        );
    }

    #[test]
    fn register_three_holds_the_clip_distances_and_the_linearisation_constants() {
        assert_eq!(
            dof_registers(&UNDERWATER_DEFAULT)[3],
            Vec4::new(15.0, 0.5, -0.5, 353_840.0)
        );
    }

    #[test]
    fn register_three_linearises_reverse_z_depth_to_near_over_depth() {
        let cb2_3 = dof_registers(&UNDERWATER_DEFAULT)[3];
        for depth in [1.0_f32, 0.5, 0.01] {
            let linear = cb2_3.x / (cb2_3.y - (2.0 * depth - 1.0) * cb2_3.z);
            let expected = CAMERA.near / depth;
            assert!(
                (linear - expected).abs() <= expected * 1e-5,
                "depth {depth}: {linear} vs {expected}"
            );
        }
    }

    #[test]
    fn a_nonzero_strength_with_a_valid_mode_activates_the_effect() {
        assert!(UNDERWATER_DEFAULT.is_active(Vignette::NONE));
        let off = ImageSpaceDepthOfField {
            strength: 0.0,
            ..UNDERWATER_DEFAULT
        };
        assert!(!off.is_active(Vignette::NONE));
    }

    #[test]
    fn mode_three_disables_the_effect_unless_the_vignette_is_on() {
        let invalid = ImageSpaceDepthOfField {
            mode: 3,
            ..UNDERWATER_DEFAULT
        };
        assert!(!invalid.is_active(Vignette::NONE));
        let vignette = Vignette {
            radius: 10.0,
            strength: 0.1,
        };
        assert!(invalid.is_active(vignette));
    }

    #[test]
    fn a_vignette_alone_activates_a_zero_strength_effect() {
        let off = ImageSpaceDepthOfField {
            strength: 0.0,
            ..UNDERWATER_DEFAULT
        };
        let vignette = Vignette {
            radius: 10.0,
            strength: 0.1,
        };
        assert!(off.is_active(vignette));
    }

    fn weather_dof() -> ImageSpaceDepthOfField {
        ImageSpaceDepthOfField {
            strength: 0.65,
            distance: 2000.0,
            range: 5048.0,
            mode: 2,
            vignette_radius: 40.0,
            vignette_strength: 0.3,
        }
    }

    const GAMEPLAY: BokehGate = BokehGate::gameplay(true);
    const OPEN_GATE: BokehGate = BokehGate {
        modifier_dof_applied: true,
        ..GAMEPLAY
    };

    fn select(
        manager: Option<&ImageSpaceDepthOfField>,
        underwater: Option<&UnderwaterOverride>,
    ) -> Option<DofFrame> {
        select_frame(manager, underwater, OPEN_GATE, CAMERA, TARGET)
    }

    #[test]
    fn normal_gameplay_leaves_the_gate_shut() {
        assert!(!GAMEPLAY.is_open());
        assert_eq!(
            select_frame(Some(&weather_dof()), None, GAMEPLAY, CAMERA, TARGET),
            None
        );
    }

    #[test]
    fn the_bokeh_setting_comes_from_the_ini() {
        for (text, enabled) in [
            ("bScreenSpaceBokeh=1\n", true),
            ("bScreenSpaceBokeh=0\n", false),
        ] {
            let gate = BokehGate::gameplay_from_ini(&GameIni::from_texts([text])).unwrap();
            assert_eq!(gate, BokehGate::gameplay(enabled), "{text}");
        }
    }

    #[test]
    fn a_missing_or_invalid_bokeh_setting_is_an_error_naming_it() {
        for text in ["", "bScreenSpaceBokeh=2\n", "bScreenSpaceBokeh=on\n"] {
            let error = BokehGate::gameplay_from_ini(&GameIni::from_texts([text])).unwrap_err();
            assert!(
                error.to_string().contains("bScreenSpaceBokeh"),
                "{text}: {error}"
            );
        }
    }

    #[test]
    fn every_gate_condition_is_required() {
        assert!(OPEN_GATE.is_open());
        let shut = [
            BokehGate {
                setting_enabled: false,
                ..OPEN_GATE
            },
            BokehGate {
                modifier_dof_applied: false,
                ..OPEN_GATE
            },
            BokehGate {
                blurring_menus: 1,
                ..OPEN_GATE
            },
        ];
        assert!(shut.iter().all(|gate| !gate.is_open()));
    }

    #[test]
    fn the_underwater_override_runs_with_a_shut_gate() {
        let overridden = UnderwaterOverride {
            dof: UNDERWATER_DEFAULT,
            fog: FOG,
        };
        let frame = select_frame(
            Some(&weather_dof()),
            Some(&overridden),
            GAMEPLAY,
            CAMERA,
            TARGET,
        )
        .unwrap();
        assert_eq!(frame.resolve, DofResolve::Fogged);
    }

    #[test]
    fn the_weather_drives_a_plain_pass_once_the_gate_opens() {
        let weather = weather_dof();
        let frame = select(Some(&weather), None).unwrap();
        assert_eq!(frame.resolve, DofResolve::Plain);
        assert_eq!(
            frame.registers,
            DofRegisters::plain(registers(&weather, weather.vignette(), CAMERA, TARGET))
        );
    }

    #[test]
    fn the_override_replaces_the_weather_dof_but_keeps_the_weather_vignette() {
        let weather = weather_dof();
        let overridden = UnderwaterOverride {
            dof: UNDERWATER_DEFAULT,
            fog: FOG,
        };
        let frame = select(Some(&weather), Some(&overridden)).unwrap();
        assert_eq!(frame.resolve, DofResolve::Fogged);
        assert_eq!(
            frame.registers,
            DofRegisters::fogged(
                registers(&UNDERWATER_DEFAULT, weather.vignette(), CAMERA, TARGET),
                FOG
            )
        );
    }

    #[test]
    fn an_override_works_without_a_weather() {
        let overridden = UnderwaterOverride {
            dof: UNDERWATER_DEFAULT,
            fog: FOG,
        };
        let frame = select(None, Some(&overridden)).unwrap();
        assert_eq!(frame.registers.cb2[2], Vec4::new(0.0, 0.0, 1.0, 0.0));
    }

    #[test]
    fn an_inactive_override_silences_the_weather_underwater() {
        let weather = ImageSpaceDepthOfField {
            vignette_strength: 0.0,
            ..weather_dof()
        };
        let default_image_space = UnderwaterOverride {
            dof: ImageSpaceDepthOfField {
                strength: 0.0,
                ..UNDERWATER_DEFAULT
            },
            fog: FOG,
        };
        assert_eq!(select(Some(&weather), Some(&default_image_space)), None);
    }

    #[test]
    fn no_source_means_no_effect() {
        assert_eq!(select(None, None), None);
        let off = ImageSpaceDepthOfField {
            strength: 0.0,
            vignette_strength: 0.0,
            ..weather_dof()
        };
        assert_eq!(select(Some(&off), None), None);
    }
}
