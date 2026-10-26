//! The sun's light scattered into the exterior fog towards the eye, from a weather's god rays
//! (`GDRY`). The game draws its god rays as shafts behind occluders; without shadows in the fog
//! this is an analytic approximation: a forward and a backward Henyey-Greenstein lobe around the
//! sun and an isotropic air term, added to the fog colour so the fog factor weights it.

use std::f32::consts::PI;

use glam::{Vec3, Vec4};
use serde::Deserialize;

use super::cell_lighting::srgb8_to_linear;
use super::lerp::lerp_fields;

/// The largest Henyey-Greenstein asymmetry a phase maps to, short of the singular lobe at 1.
pub const MAX_ASYMMETRY: f32 = 0.95;

/// The `god_rayses.data` JSON: the `GDRY` `DATA` block, colours as stored bytes.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct GodRaysData {
    pub back_color: [u8; 3],
    pub forward_color: [u8; 3],
    pub air_color: [u8; 3],
    pub back_color_scale: f32,
    pub forward_color_scale: f32,
    pub air_color_scale: f32,
    pub intensity: f32,
    pub back_phase: f32,
    pub forward_phase: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
#[error("negative {field} ({value})")]
pub struct NegativeGodRaysValue {
    pub field: &'static str,
    pub value: f32,
}

/// The in-scatter of one weather at one moment, per unit of sun radiance: each colour linear,
/// with its scale and the record's intensity folded in. `forward_asymmetry` is positive (the lobe
/// leans towards the sun), `back_asymmetry` negative (away from it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SunInScatter {
    pub forward: Vec3,
    pub back: Vec3,
    pub air: Vec3,
    pub forward_asymmetry: f32,
    pub back_asymmetry: f32,
}

lerp_fields!(SunInScatter {
    forward,
    back,
    air,
    forward_asymmetry,
    back_asymmetry
});

impl SunInScatter {
    /// No in-scatter: the fog keeps its own colours.
    pub const NONE: Self = Self {
        forward: Vec3::ZERO,
        back: Vec3::ZERO,
        air: Vec3::ZERO,
        forward_asymmetry: 0.0,
        back_asymmetry: 0.0,
    };

    /// Inferred, as no public source documents how the game reads a `GDRY`: the colours are
    /// sRGB like the weather's other colours, each scale multiplies its colour, the intensity
    /// all three, and a phase is the asymmetry of its lobe ([`asymmetry_of_phase`]).
    pub fn from_god_rays(data: &GodRaysData) -> Result<Self, NegativeGodRaysValue> {
        for (field, value) in [
            ("intensity", data.intensity),
            ("back colour scale", data.back_color_scale),
            ("forward colour scale", data.forward_color_scale),
            ("air colour scale", data.air_color_scale),
        ] {
            if value < 0.0 {
                return Err(NegativeGodRaysValue { field, value });
            }
        }
        let color = |bytes, scale: f32| srgb8_to_linear(bytes) * (scale * data.intensity);
        Ok(Self {
            forward: color(data.forward_color, data.forward_color_scale),
            back: color(data.back_color, data.back_color_scale),
            air: color(data.air_color, data.air_color_scale),
            forward_asymmetry: asymmetry_of_phase(data.forward_phase),
            back_asymmetry: -asymmetry_of_phase(data.back_phase),
        })
    }

    /// The light scattered towards the eye per unit of sun radiance, looking `cos_to_sun` away
    /// from the sun (the cosine of the angle between the view ray and the direction to the sun).
    pub fn radiance(&self, cos_to_sun: f32) -> Vec3 {
        self.forward * henyey_greenstein(cos_to_sun, self.forward_asymmetry)
            + self.back * henyey_greenstein(cos_to_sun, self.back_asymmetry)
            + self.air / (4.0 * PI)
    }

    /// The in-scatter under a sun of `sun_radiance` that lies towards the unit `to_sun`.
    pub fn uniform_fields(&self, sun_radiance: Vec3, to_sun: Vec3) -> InScatterUniformFields {
        InScatterUniformFields {
            forward: (self.forward * sun_radiance).extend(self.forward_asymmetry),
            back: (self.back * sun_radiance).extend(self.back_asymmetry),
            air: (self.air * sun_radiance).extend(0.0),
            to_sun: to_sun.extend(0.0),
        }
    }
}

/// A `GDRY` phase as the asymmetry of a Henyey-Greenstein lobe: vanilla phases lie in 0 to 0.92,
/// so the value is taken as it is, clamped to 0 to [`MAX_ASYMMETRY`]. Inferred from the range.
pub fn asymmetry_of_phase(phase: f32) -> f32 {
    phase.clamp(0.0, MAX_ASYMMETRY)
}

/// The Henyey-Greenstein phase function, normalised over the sphere: the share of scattered
/// light that leaves `cos_theta` away from the incoming direction, per steradian.
pub fn henyey_greenstein(cos_theta: f32, asymmetry: f32) -> f32 {
    let g = asymmetry;
    let denominator = (1.0 + g * g - 2.0 * g * cos_theta.clamp(-1.0, 1.0)).powf(1.5);
    (1.0 - g * g) / (4.0 * PI * denominator)
}

/// The `FrameUniform` in-scatter fields, laid out as `fog_sun_forward`, `fog_sun_back`,
/// `fog_sun_air` and `fog_to_sun` in `shaders/frame.wgsl`.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct InScatterUniformFields {
    /// rgb: the forward lobe's colour times the sun radiance, w: its asymmetry.
    pub forward: Vec4,
    /// rgb: the backward lobe's colour times the sun radiance, w: its asymmetry.
    pub back: Vec4,
    /// rgb: the isotropic air colour times the sun radiance.
    pub air: Vec4,
    /// xyz: unit direction towards the sun.
    pub to_sun: Vec4,
}

impl InScatterUniformFields {
    pub const NONE: Self = Self {
        forward: Vec4::ZERO,
        back: Vec4::ZERO,
        air: Vec4::ZERO,
        to_sun: Vec4::ZERO,
    };
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "the folded scales are exact in f32")]
mod tests {
    use super::*;

    fn data() -> GodRaysData {
        GodRaysData {
            back_color: [255, 0, 0],
            forward_color: [0, 255, 0],
            air_color: [0, 0, 255],
            back_color_scale: 4.0,
            forward_color_scale: 2.0,
            air_color_scale: 0.5,
            intensity: 3.0,
            back_phase: 0.25,
            forward_phase: 0.75,
        }
    }

    fn integral_over_sphere(asymmetry: f32) -> f32 {
        const STEPS: usize = 20_000;
        let step = 2.0 / STEPS as f32;
        (0..STEPS)
            .map(|index| {
                let cos_theta = -1.0 + (index as f32 + 0.5) * step;
                henyey_greenstein(cos_theta, asymmetry) * 2.0 * PI * step
            })
            .sum()
    }

    #[test]
    fn the_json_of_the_database_contract_parses() {
        let json = r#"{"back_color":[32,43,66],"forward_color":[47,47,38],"air_color":[78,83,105],"back_color_scale":18,"forward_color_scale":1,"air_color_scale":0.1,"intensity":1,"back_phase":0.473,"forward_phase":0.915}"#;
        let data: GodRaysData = serde_json::from_str(json).unwrap();
        assert_eq!(data.back_color, [32, 43, 66]);
        assert_eq!(data.back_color_scale, 18.0);
        assert_eq!(data.forward_phase, 0.915);
    }

    #[test]
    fn each_colour_is_linear_with_its_scale_and_the_intensity_folded_in() {
        let in_scatter = SunInScatter::from_god_rays(&data()).unwrap();
        assert_eq!(in_scatter.forward, Vec3::new(0.0, 6.0, 0.0));
        assert_eq!(in_scatter.back, Vec3::new(12.0, 0.0, 0.0));
        assert_eq!(in_scatter.air, Vec3::new(0.0, 0.0, 1.5));
    }

    #[test]
    fn the_forward_phase_leans_towards_the_sun_and_the_back_phase_away_from_it() {
        let in_scatter = SunInScatter::from_god_rays(&data()).unwrap();
        assert_eq!(in_scatter.forward_asymmetry, 0.75);
        assert_eq!(in_scatter.back_asymmetry, -0.25);
    }

    #[test]
    fn a_phase_is_clamped_short_of_a_singular_lobe_and_never_turns_its_lobe_around() {
        assert_eq!(asymmetry_of_phase(0.0), 0.0);
        assert_eq!(asymmetry_of_phase(0.9), 0.9);
        assert_eq!(asymmetry_of_phase(1.0), MAX_ASYMMETRY);
        assert_eq!(asymmetry_of_phase(-0.3), 0.0);
    }

    #[test]
    fn a_negative_intensity_or_scale_is_rejected_by_name() {
        let error = SunInScatter::from_god_rays(&GodRaysData {
            air_color_scale: -1.0,
            ..data()
        })
        .unwrap_err();
        assert_eq!(
            error,
            NegativeGodRaysValue {
                field: "air colour scale",
                value: -1.0
            }
        );
        let error = SunInScatter::from_god_rays(&GodRaysData {
            intensity: -0.5,
            ..data()
        })
        .unwrap_err();
        assert_eq!(error.field, "intensity");
    }

    #[test]
    fn zero_intensity_scatters_nothing_in_any_direction() {
        let in_scatter = SunInScatter::from_god_rays(&GodRaysData {
            intensity: 0.0,
            ..data()
        })
        .unwrap();
        for cos_to_sun in [-1.0, 0.0, 1.0] {
            assert_eq!(in_scatter.radiance(cos_to_sun), Vec3::ZERO, "{cos_to_sun}");
        }
    }

    #[test]
    fn the_phase_function_integrates_to_one_over_the_sphere() {
        for asymmetry in [-0.9, -0.3, 0.0, 0.5, MAX_ASYMMETRY] {
            let integral = integral_over_sphere(asymmetry);
            assert!((integral - 1.0).abs() < 2e-3, "{asymmetry}: {integral}");
        }
    }

    #[test]
    fn without_asymmetry_the_phase_function_is_isotropic() {
        for cos_theta in [-1.0, 0.0, 1.0] {
            let phase = henyey_greenstein(cos_theta, 0.0);
            assert!((phase - 1.0 / (4.0 * PI)).abs() < 1e-7, "{cos_theta}");
        }
    }

    #[test]
    fn the_forward_lobe_peaks_towards_the_sun_and_the_back_lobe_away_from_it() {
        let forward = SunInScatter {
            forward: Vec3::ONE,
            forward_asymmetry: 0.6,
            ..SunInScatter::NONE
        };
        let back = SunInScatter {
            back: Vec3::ONE,
            back_asymmetry: -0.6,
            ..SunInScatter::NONE
        };
        assert!(forward.radiance(1.0).x > forward.radiance(0.0).x);
        assert!(forward.radiance(0.0).x > forward.radiance(-1.0).x);
        assert!(back.radiance(-1.0).x > back.radiance(0.0).x);
        assert!(back.radiance(0.0).x > back.radiance(1.0).x);
        // (1 - g^2) / (4 pi (1 - g)^3) towards the peak.
        let peak = 0.64 / (4.0 * PI * 0.4_f32.powi(3));
        assert!((forward.radiance(1.0).x - peak).abs() < 1e-5);
    }

    #[test]
    fn the_air_scatters_the_same_in_every_direction() {
        let air = SunInScatter {
            air: Vec3::new(4.0 * PI, 0.0, 0.0),
            ..SunInScatter::NONE
        };
        for cos_to_sun in [-1.0, 0.3, 1.0] {
            assert!((air.radiance(cos_to_sun).x - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn the_lobes_and_the_air_add_up() {
        let in_scatter = SunInScatter::from_god_rays(&data()).unwrap();
        let alone = |only: SunInScatter| only.radiance(0.4);
        let sum = alone(SunInScatter {
            back: Vec3::ZERO,
            air: Vec3::ZERO,
            ..in_scatter
        }) + alone(SunInScatter {
            forward: Vec3::ZERO,
            air: Vec3::ZERO,
            ..in_scatter
        }) + alone(SunInScatter {
            forward: Vec3::ZERO,
            back: Vec3::ZERO,
            ..in_scatter
        });
        assert!(in_scatter.radiance(0.4).abs_diff_eq(sum, 1e-6));
    }

    #[test]
    fn a_cosine_beyond_one_is_clamped() {
        assert_eq!(henyey_greenstein(1.5, 0.5), henyey_greenstein(1.0, 0.5));
        assert_eq!(henyey_greenstein(-1.5, 0.5), henyey_greenstein(-1.0, 0.5));
    }

    #[test]
    fn the_uniform_fields_carry_the_colours_under_the_sun_with_the_asymmetries_and_direction() {
        let in_scatter = SunInScatter::from_god_rays(&data()).unwrap();
        let fields = in_scatter.uniform_fields(Vec3::new(0.5, 1.0, 2.0), Vec3::Y);
        assert_eq!(fields.forward, Vec4::new(0.0, 6.0, 0.0, 0.75));
        assert_eq!(fields.back, Vec4::new(6.0, 0.0, 0.0, -0.25));
        assert_eq!(fields.air, Vec4::new(0.0, 0.0, 3.0, 0.0));
        assert_eq!(fields.to_sun, Vec4::new(0.0, 1.0, 0.0, 0.0));
    }

    #[test]
    fn no_in_scatter_or_no_sun_gives_colourless_fields() {
        let none = SunInScatter::NONE.uniform_fields(Vec3::ONE, Vec3::Y);
        let dark = SunInScatter::from_god_rays(&data())
            .unwrap()
            .uniform_fields(Vec3::ZERO, Vec3::Y);
        for fields in [none, dark] {
            assert_eq!(fields.forward.truncate(), Vec3::ZERO);
            assert_eq!(fields.back.truncate(), Vec3::ZERO);
            assert_eq!(fields.air.truncate(), Vec3::ZERO);
        }
    }

    #[test]
    fn a_blend_towards_no_in_scatter_fades_every_colour() {
        use super::super::lerp::Lerp;
        let in_scatter = SunInScatter::from_god_rays(&data()).unwrap();
        let halfway = in_scatter.lerp(&SunInScatter::NONE, 0.5);
        assert_eq!(halfway.forward, in_scatter.forward * 0.5);
        assert_eq!(halfway.back, in_scatter.back * 0.5);
        assert_eq!(halfway.air, in_scatter.air * 0.5);
    }
}
