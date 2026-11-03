//! The three lights the game puts in the loading menu's 3D scene, the `fLoadingMenuSceneLight`
//! settings of `[Interface]`. They are placed lights like the world's, fixed relative to the
//! camera, so the model is lit the same way whatever it is turned to.
//!
//! The game's own INIs leave these settings out (the game's built-in values apply), so each one
//! falls back to the value publicly documented for the hidden defaults; a setting an INI does set
//! overrides it. Interpretation, not documented by the game: the position is in game units from
//! the camera along the camera's axes (X right, Y forward, Z up), the dimmer multiplies the
//! diffuse colour as a placed light's fade does, and the radius is the distance at which the
//! falloff reaches zero. The lights are spotlights, as the documented `SpotFOV` of 10 degrees
//! says: the full cone angle, shaded like a placed spotlight without a lamp shadow, aimed at the
//! model.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

use crate::assets::ini::{GameIni, IniError, IniKey};
use crate::bethesda_space::position_to_engine;
use crate::rendering::placed_light::{defaults, linear_colour, outer_cone_cos, radius_for};

pub const LIGHT_COUNT: usize = 3;

/// The documented hidden default of every light's `SpotFOV`, in degrees.
const VANILLA_SPOT_FOV: f32 = 10.0;

/// One light as its settings state it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneLight {
    /// The diffuse colour, gamma-encoded, 0 to 1 per channel.
    pub diffuse: Vec3,
    /// The intensity multiplier of the diffuse colour.
    pub dimmer: f32,
    /// In game units.
    pub radius: f32,
    /// In game units from the camera: X right, Y forward, Z up.
    pub position: Vec3,
    /// The full cone angle in degrees, above 0 and at most 180.
    pub spot_fov: f32,
}

/// The loading menu's lights, in the order of the settings: the key from the upper left behind
/// the model, a cool rim from the right behind it and a dim fill from the front.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneLights {
    pub lights: [SceneLight; LIGHT_COUNT],
}

struct LightKeys {
    diffuse: [IniKey; 3],
    dimmer: IniKey,
    radius: IniKey,
    position: [IniKey; 3],
    spot_fov: IniKey,
}

macro_rules! light_keys {
    ($prefix:literal) => {
        LightKeys {
            diffuse: [
                IniKey::new("Interface", concat!($prefix, "DiffuseColorR")),
                IniKey::new("Interface", concat!($prefix, "DiffuseColorG")),
                IniKey::new("Interface", concat!($prefix, "DiffuseColorB")),
            ],
            dimmer: IniKey::new("Interface", concat!($prefix, "DimmerValue")),
            radius: IniKey::new("Interface", concat!($prefix, "Radius")),
            position: [
                IniKey::new("Interface", concat!($prefix, "X")),
                IniKey::new("Interface", concat!($prefix, "Y")),
                IniKey::new("Interface", concat!($prefix, "Z")),
            ],
            spot_fov: IniKey::new("Interface", concat!($prefix, "SpotFOV")),
        }
    };
}

const KEYS: [LightKeys; LIGHT_COUNT] = [
    light_keys!("fLoadingMenuSceneLight"),
    light_keys!("fLoadingMenuSceneLight2"),
    light_keys!("fLoadingMenuSceneLight3"),
];

impl SceneLights {
    /// The hidden defaults of the game's built-in settings.
    pub const VANILLA: Self = Self {
        lights: [
            SceneLight {
                diffuse: Vec3::new(1.0, 1.0, 0.9),
                dimmer: 8.5,
                radius: 5000.0,
                position: Vec3::new(-1800.0, 1000.0, 1500.0),
                spot_fov: VANILLA_SPOT_FOV,
            },
            SceneLight {
                diffuse: Vec3::new(0.64, 0.78, 0.74),
                dimmer: 12.0,
                radius: 4000.0,
                position: Vec3::new(1450.0, 2400.0, 1700.0),
                spot_fov: VANILLA_SPOT_FOV,
            },
            SceneLight {
                diffuse: Vec3::new(0.8, 0.82, 0.74),
                dimmer: 1.2,
                radius: 5000.0,
                position: Vec3::new(400.0, -800.0, 220.0),
                spot_fov: VANILLA_SPOT_FOV,
            },
        ],
    };

    /// A setting no INI sets takes its [`Self::VANILLA`] value, with one note in the log; one
    /// that is set must be a finite number, not negative for a colour or a dimmer and above zero
    /// for a radius.
    pub fn from_ini(ini: &GameIni) -> Result<Self, IniError> {
        let mut reader = Reader {
            ini,
            defaulted: Vec::new(),
        };
        let mut lights = Self::VANILLA.lights;
        for (light, keys) in lights.iter_mut().zip(&KEYS) {
            *light = SceneLight {
                diffuse: Vec3::from_array(reader.read_each(
                    keys.diffuse,
                    light.diffuse.to_array(),
                    NON_NEGATIVE,
                )?),
                dimmer: reader.read(keys.dimmer, light.dimmer, NON_NEGATIVE)?,
                radius: reader.read(keys.radius, light.radius, POSITIVE)?,
                position: Vec3::from_array(reader.read_each(
                    keys.position,
                    light.position.to_array(),
                    FINITE,
                )?),
                spot_fov: reader.read(keys.spot_fov, light.spot_fov, SPOT_FOV)?,
            };
        }
        reader.note_defaults();
        Ok(Self { lights })
    }

    pub(super) fn to_gpu(self) -> [GpuSceneLight; LIGHT_COUNT] {
        self.lights.map(SceneLight::to_gpu)
    }
}

#[derive(Clone, Copy)]
struct Rule {
    requirement: &'static str,
    holds: fn(&f32) -> bool,
}

const FINITE: Rule = Rule {
    requirement: "a finite number",
    holds: |value| value.is_finite(),
};

const NON_NEGATIVE: Rule = Rule {
    requirement: "a finite number, 0 or more",
    holds: |value| value.is_finite() && *value >= 0.0,
};

const SPOT_FOV: Rule = Rule {
    requirement: "a finite number above 0 and at most 180",
    holds: |value| value.is_finite() && *value > 0.0 && *value <= 180.0,
};

const POSITIVE: Rule = Rule {
    requirement: "a finite number above 0",
    holds: |value| value.is_finite() && *value > 0.0,
};

struct Reader<'a> {
    ini: &'a GameIni,
    defaulted: Vec<&'static str>,
}

impl Reader<'_> {
    fn read(&mut self, key: IniKey, default: f32, rule: Rule) -> Result<f32, IniError> {
        let value: Option<f32> = self.ini.optional_valid(key, rule.requirement, rule.holds)?;
        Ok(value.unwrap_or_else(|| {
            self.defaulted.push(key.name);
            default
        }))
    }

    fn read_each(
        &mut self,
        keys: [IniKey; 3],
        defaults: [f32; 3],
        rule: Rule,
    ) -> Result<[f32; 3], IniError> {
        let mut values = defaults;
        for ((value, key), default) in values.iter_mut().zip(keys).zip(defaults) {
            *value = self.read(key, default, rule)?;
        }
        Ok(values)
    }

    fn note_defaults(&self) {
        if let Some(first) = self.defaulted.first() {
            tracing::info!(
                "{} loading menu scene light settings (such as {first}) are not set in any game INI; the vanilla defaults apply",
                self.defaulted.len()
            );
        }
    }
}

/// Mirrors `SceneLight` in `loading_art.wgsl`: a placed light as the world's lighting consumes it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub(super) struct GpuSceneLight {
    /// The position in engine space and the radius in engine units.
    position_radius: [f32; 4],
    /// The linear colour with the dimmer applied.
    colour: [f32; 4],
    /// The falloff's constant, scalar and exponent: the modal values of the world's light records;
    /// and the cosine of the cone's half-angle.
    attenuation: [f32; 4],
}

impl SceneLight {
    fn to_gpu(self) -> GpuSceneLight {
        let [x, y, z] = self.position.to_array();
        let position = position_to_engine(x, y, z);
        let radius = radius_for(self.radius, None, 1.0);
        GpuSceneLight {
            position_radius: [position.x, position.y, position.z, radius],
            colour: linear_colour(self.diffuse, self.dimmer)
                .extend(0.0)
                .to_array(),
            attenuation: [
                defaults::CONSTANT,
                defaults::SCALAR,
                defaults::EXPONENT,
                outer_cone_cos(self.spot_fov).expect("a scene light's SpotFOV is within (0, 180]"),
            ],
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the expected values are exact literals parsed or copied, not computed"
)]
mod tests {
    use super::*;

    fn ini(text: &str) -> GameIni {
        GameIni::from_texts([text])
    }

    #[test]
    fn without_any_setting_the_documented_defaults_apply() {
        let lights = SceneLights::from_ini(&ini("")).unwrap();

        assert_eq!(lights, SceneLights::VANILLA);
    }

    #[test]
    fn the_defaults_are_the_documented_hidden_values() {
        let [key, rim, fill] = SceneLights::VANILLA.lights;

        assert_eq!(key.diffuse, Vec3::new(1.0, 1.0, 0.9));
        assert_eq!((key.dimmer, key.radius), (8.5, 5000.0));
        assert_eq!(key.position, Vec3::new(-1800.0, 1000.0, 1500.0));
        assert_eq!(rim.diffuse, Vec3::new(0.64, 0.78, 0.74));
        assert_eq!((rim.dimmer, rim.radius), (12.0, 4000.0));
        assert_eq!(rim.position, Vec3::new(1450.0, 2400.0, 1700.0));
        assert_eq!(fill.diffuse, Vec3::new(0.8, 0.82, 0.74));
        assert_eq!((fill.dimmer, fill.radius), (1.2, 5000.0));
        assert_eq!(fill.position, Vec3::new(400.0, -800.0, 220.0));
    }

    #[test]
    fn a_setting_in_an_ini_overrides_only_its_own_value() {
        let lights = SceneLights::from_ini(&ini(
            "[Interface]\nfLoadingMenuSceneLight2DimmerValue=3.5\nfLoadingMenuSceneLight3Z=-40\nfLoadingMenuSceneLightDiffuseColorG=0.25\n",
        ))
        .unwrap();

        let mut expected = SceneLights::VANILLA;
        expected.lights[1].dimmer = 3.5;
        expected.lights[2].position.z = -40.0;
        expected.lights[0].diffuse.y = 0.25;
        assert_eq!(lights, expected);
    }

    #[test]
    fn a_later_ini_wins_and_keys_match_regardless_of_case() {
        let lights = SceneLights::from_ini(&GameIni::from_texts([
            "fLoadingMenuSceneLightRadius=100\n",
            "FLOADINGMENUSCENELIGHTRADIUS=250\n",
        ]))
        .unwrap();

        assert_eq!(lights.lights[0].radius, 250.0);
    }

    #[test]
    fn a_setting_that_breaks_its_rule_is_an_error_naming_the_key() {
        for (text, key) in [
            (
                "fLoadingMenuSceneLightRadius=0\n",
                "fLoadingMenuSceneLightRadius",
            ),
            (
                "fLoadingMenuSceneLight2Radius=-5\n",
                "fLoadingMenuSceneLight2Radius",
            ),
            (
                "fLoadingMenuSceneLightDimmerValue=-1\n",
                "fLoadingMenuSceneLightDimmerValue",
            ),
            (
                "fLoadingMenuSceneLight3DiffuseColorB=-0.1\n",
                "fLoadingMenuSceneLight3DiffuseColorB",
            ),
            ("fLoadingMenuSceneLightX=inf\n", "fLoadingMenuSceneLightX"),
            ("fLoadingMenuSceneLightY=far\n", "fLoadingMenuSceneLightY"),
        ] {
            let error = SceneLights::from_ini(&ini(text)).unwrap_err();

            assert!(error.to_string().contains(key), "{text:?}: {error}");
        }
    }

    #[test]
    fn a_light_may_sit_on_either_side_of_the_camera_and_may_be_dimmed_to_nothing() {
        let lights = SceneLights::from_ini(&ini(
            "fLoadingMenuSceneLightX=-5\nfLoadingMenuSceneLightY=-5\nfLoadingMenuSceneLightZ=-5\nfLoadingMenuSceneLightDimmerValue=0\n",
        ))
        .unwrap();

        assert_eq!(lights.lights[0].position, Vec3::splat(-5.0));
        assert_eq!(lights.lights[0].dimmer, 0.0);
    }

    #[test]
    fn game_axes_become_engine_axes_with_forward_along_minus_z() {
        let light = SceneLight {
            position: Vec3::new(100.0, 200.0, 300.0),
            ..SceneLights::VANILLA.lights[0]
        };

        let [x, y, z, _] = light.to_gpu().position_radius;

        assert_eq!([x, y, z], [10.0, 30.0, -20.0]);
    }

    #[test]
    fn the_vanilla_layout_puts_the_key_up_left_behind_the_model_and_the_fill_in_front() {
        let [key, rim, fill] = SceneLights::VANILLA
            .to_gpu()
            .map(|light| light.position_radius);

        assert!(key[0] < 0.0 && key[1] > 0.0 && key[2] < 0.0, "{key:?}");
        assert!(rim[0] > 0.0 && rim[1] > 0.0 && rim[2] < 0.0, "{rim:?}");
        assert!(fill[0] > 0.0 && fill[2] > 0.0, "{fill:?}");
    }

    #[test]
    fn the_radius_is_in_engine_units() {
        let [key, ..] = SceneLights::VANILLA.to_gpu();

        assert!((key.position_radius[3] - 500.0).abs() < 1e-3);
    }

    #[test]
    fn the_colour_is_the_encoded_diffuse_made_linear_like_a_placed_light_times_the_dimmer() {
        let light = SceneLight {
            diffuse: Vec3::new(1.0, 0.5, 0.0),
            dimmer: 4.0,
            radius: 1000.0,
            position: Vec3::ZERO,
            spot_fov: 10.0,
        };

        let [red, green, blue, _] = light.to_gpu().colour;

        assert!((red - 4.0).abs() < 1e-5);
        assert!((green - 4.0 * 0.5_f32.powf(2.2)).abs() < 1e-5, "{green}");
        assert_eq!(blue, 0.0);
    }

    #[test]
    fn the_falloff_is_the_worlds_modal_record_attenuation() {
        let [light, ..] = SceneLights::VANILLA.to_gpu();

        assert_eq!(light.attenuation[..3], [0.0, 1.0, 2.0]);
    }

    #[test]
    fn every_vanilla_light_is_a_spotlight_of_10_degrees() {
        for light in SceneLights::VANILLA.lights {
            assert_eq!(light.spot_fov, 10.0);
        }
    }

    #[test]
    fn the_cone_is_stored_as_the_cosine_of_the_half_angle_of_the_full_angle() {
        let light = SceneLight {
            spot_fov: 60.0,
            ..SceneLights::VANILLA.lights[0]
        };

        let cosine = light.to_gpu().attenuation[3];

        assert!(
            (cosine - 30.0_f32.to_radians().cos()).abs() < 1e-6,
            "{cosine}"
        );
    }

    #[test]
    fn a_spot_fov_setting_overrides_only_its_own_light() {
        let lights = SceneLights::from_ini(&ini(
            "[Interface]\nfLoadingMenuSceneLight2SpotFOV=45\nfLoadingMenuSceneLight3SpotFOV=180\n",
        ))
        .unwrap();

        let fovs = lights.lights.map(|light| light.spot_fov);
        assert_eq!(fovs, [10.0, 45.0, 180.0]);
    }

    #[test]
    fn a_spot_fov_outside_the_cone_range_is_an_error_naming_the_key() {
        for text in [
            "fLoadingMenuSceneLightSpotFOV=0\n",
            "fLoadingMenuSceneLightSpotFOV=-10\n",
            "fLoadingMenuSceneLightSpotFOV=180.5\n",
            "fLoadingMenuSceneLightSpotFOV=inf\n",
            "fLoadingMenuSceneLightSpotFOV=wide\n",
        ] {
            let error = SceneLights::from_ini(&ini(text)).unwrap_err();

            assert!(
                error.to_string().contains("fLoadingMenuSceneLightSpotFOV"),
                "{text:?}: {error}"
            );
        }
    }
}
