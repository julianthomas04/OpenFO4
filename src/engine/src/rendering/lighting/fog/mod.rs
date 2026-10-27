//! Distance and height fog, following measurements of vanilla Fallout 4's fog. [`model`] is the
//! per-point maths `fog.wgsl` mirrors, [`exterior`] resolves a weather's fog for a time of day,
//! and [`measured`] holds the measured constants and the estimates for what is still open.

mod exterior;
mod height;
pub mod measured;
mod model;
mod time;
mod uniform;

use glam::Vec3;

pub use self::exterior::{ExteriorFog, FogRange};
#[cfg(test)]
pub(in crate::rendering::lighting) use self::height::test_support;
pub use self::height::{HeightBand, HeightBands, HeightFogError, RawHeightFog};
pub use self::model::FogBlend;
#[cfg(test)]
pub(crate) use self::model::probe as model_probe;
pub use self::time::FogTime;
pub use self::uniform::FogUniformFields;
use super::lerp::lerp_fields;

/// The four fog colours, linear.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct FogColors {
    pub near: Vec3,
    pub far: Vec3,
    pub near_high: Vec3,
    pub far_high: Vec3,
}

lerp_fields!(FogColors {
    near,
    far,
    near_high,
    far_high
});

impl FogColors {
    /// The same colour near, far and high.
    pub fn uniform(color: Vec3) -> Self {
        Self {
            near: color,
            far: color,
            near_high: color,
            far_high: color,
        }
    }

    fn map(self, color: impl Fn(Vec3) -> Vec3) -> Self {
        Self {
            near: color(self.near),
            far: color(self.far),
            near_high: color(self.near_high),
            far_high: color(self.far_high),
        }
    }
}

/// What the sunlight does to the fog colour short of full fog: `color * kept + added`.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct SunlightTerm {
    pub kept: f32,
    pub added: Vec3,
}

impl SunlightTerm {
    pub const NONE: Self = Self {
        kept: 1.0,
        added: Vec3::ZERO,
    };

    pub fn apply(self, color: Vec3) -> Vec3 {
        color * self.kept + self.added
    }
}

/// Fog ready to render: engine units, and colours with their time-of-day factors applied.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct FogParams {
    pub colors: FogColors,
    pub near: f32,
    pub far: f32,
    pub power: f32,
    pub max: f32,
    pub height: HeightBands,
    pub sunlight: SunlightTerm,
}

impl FogParams {
    /// A range that does not increase has no gradient; such fog renders as disabled.
    fn has_range(&self) -> bool {
        self.far > self.near
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum Fog {
    Disabled,
    Enabled(FogParams),
}

impl Fog {
    /// The fog with `light` added to all of its colours; where it begins and ends and how it
    /// builds up stay. Disabled fog has no colour to lift.
    #[must_use]
    pub fn brightened(self, light: Vec3) -> Self {
        match self {
            Fog::Enabled(fog) => Fog::Enabled(FogParams {
                colors: fog.colors.map(|color| color + light),
                ..fog
            }),
            Fog::Disabled => Fog::Disabled,
        }
    }

    /// The parameters of fog that renders, `None` for disabled fog and a range that does not
    /// increase.
    fn rendered(&self) -> Option<&FogParams> {
        match self {
            Fog::Enabled(fog) if fog.has_range() => Some(fog),
            Fog::Enabled(_) | Fog::Disabled => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> FogParams {
        FogParams {
            colors: FogColors {
                near: Vec3::new(0.1, 0.2, 0.3),
                far: Vec3::new(0.4, 0.5, 0.6),
                near_high: Vec3::new(0.7, 0.8, 0.9),
                far_high: Vec3::new(1.0, 1.1, 1.2),
            },
            near: 10.0,
            far: 100.0,
            power: 2.0,
            max: 0.5,
            height: HeightBands {
                near: HeightBand::new(20.0, 10.0),
                far: HeightBand::new(50.0, 25.0),
                high_density_scale: 0.5,
            },
            sunlight: SunlightTerm {
                kept: 0.9,
                added: Vec3::splat(0.01),
            },
        }
    }

    #[test]
    fn brightening_adds_the_light_to_every_colour_and_nothing_else() {
        let light = Vec3::new(0.5, 0.25, 0.125);
        let Fog::Enabled(brightened) = Fog::Enabled(params()).brightened(light) else {
            panic!("brightened fog stays enabled");
        };
        let original = params();
        assert_eq!(
            brightened.colors,
            FogColors {
                near: original.colors.near + light,
                far: original.colors.far + light,
                near_high: original.colors.near_high + light,
                far_high: original.colors.far_high + light,
            }
        );
        assert_eq!(
            FogParams {
                colors: original.colors,
                ..brightened
            },
            original
        );
    }

    #[test]
    fn brightening_by_no_light_changes_nothing_and_disabled_fog_stays_disabled() {
        let fog = Fog::Enabled(params());
        assert_eq!(fog.brightened(Vec3::ZERO), fog);
        assert_eq!(Fog::Disabled.brightened(Vec3::ONE), Fog::Disabled);
    }

    #[test]
    fn the_sunlight_term_keeps_a_share_of_the_colour_and_adds_its_light() {
        let term = SunlightTerm {
            kept: 0.5,
            added: Vec3::splat(0.25),
        };
        assert_eq!(term.apply(Vec3::ONE), Vec3::splat(0.75));
        assert_eq!(SunlightTerm::NONE.apply(Vec3::ONE), Vec3::ONE);
    }

    #[test]
    fn only_enabled_fog_with_an_increasing_range_renders() {
        assert!(Fog::Enabled(params()).rendered().is_some());
        assert!(Fog::Disabled.rendered().is_none());
        for (near, far) in [(100.0, 100.0), (100.0, 10.0), (f32::NAN, 100.0)] {
            let fog = Fog::Enabled(FogParams {
                near,
                far,
                ..params()
            });
            assert!(fog.rendered().is_none(), "{near}..{far}");
        }
    }
}
