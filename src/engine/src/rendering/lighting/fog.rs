use glam::{Vec3, Vec4};

/// Bethesda-style parametric fog in engine units and linear colours; shared by
/// the exterior fog settings and interior cell lighting.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct FogParams {
    pub near_color: Vec3,
    pub far_color: Vec3,
    pub near: f32,
    pub far: f32,
    pub power: f32,
    pub max: f32,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum Fog {
    Disabled,
    Enabled(FogParams),
}

/// The `FrameUniform` fog fields, laid out as `fog_near_color`, `fog_far_color`
/// and `fog_params` in `shaders/frame.wgsl`.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct FogUniformFields {
    pub near_color: Vec4,
    pub far_color: Vec4,
    pub params: Vec4,
}

impl FogUniformFields {
    /// Zero maximum opacity, so `apply_fog` leaves the colour untouched.
    const DISABLED: Self = Self {
        near_color: Vec4::ZERO,
        far_color: Vec4::ZERO,
        params: Vec4::ZERO,
    };
}

impl Fog {
    /// The fog with `light` added to both of its colours; where it begins and ends and how it
    /// builds up stay. Disabled fog has no colour to lift.
    #[must_use]
    pub fn brightened(self, light: Vec3) -> Self {
        match self {
            Fog::Enabled(fog) => Fog::Enabled(FogParams {
                near_color: fog.near_color + light,
                far_color: fog.far_color + light,
                ..fog
            }),
            Fog::Disabled => Fog::Disabled,
        }
    }

    /// A fog range that does not increase has no meaningful gradient and is
    /// treated as disabled.
    pub fn uniform_fields(&self) -> FogUniformFields {
        match *self {
            Fog::Enabled(fog) if fog.far > fog.near => FogUniformFields {
                near_color: fog.near_color.extend(fog.near),
                far_color: fog.far_color.extend(fog.far),
                params: Vec4::new(fog.power, fog.max, 0.0, 0.0),
            },
            Fog::Enabled(_) | Fog::Disabled => FogUniformFields::DISABLED,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(near: f32, far: f32) -> FogParams {
        FogParams {
            near_color: Vec3::new(0.1, 0.2, 0.3),
            far_color: Vec3::new(0.4, 0.5, 0.6),
            near,
            far,
            power: 2.0,
            max: 0.5,
        }
    }

    #[test]
    fn enabled_fog_packs_colours_with_their_distances_and_power_with_max() {
        let fields = Fog::Enabled(params(10.0, 100.0)).uniform_fields();
        assert_eq!(fields.near_color, Vec4::new(0.1, 0.2, 0.3, 10.0));
        assert_eq!(fields.far_color, Vec4::new(0.4, 0.5, 0.6, 100.0));
        assert_eq!(fields.params, Vec4::new(2.0, 0.5, 0.0, 0.0));
    }

    #[test]
    fn disabled_fog_has_zero_opacity() {
        let fields = Fog::Disabled.uniform_fields();
        assert_eq!(fields.params, Vec4::ZERO);
    }

    #[test]
    fn a_fog_range_that_does_not_increase_is_treated_as_disabled() {
        for (near, far) in [(100.0, 100.0), (100.0, 10.0), (0.0, 0.0)] {
            let fields = Fog::Enabled(params(near, far)).uniform_fields();
            assert_eq!(fields, Fog::Disabled.uniform_fields(), "{near}..{far}");
        }
    }

    #[test]
    fn brightening_adds_the_light_to_both_colours_and_nothing_else() {
        let light = Vec3::new(0.5, 0.25, 0.125);
        let Fog::Enabled(brightened) = Fog::Enabled(params(10.0, 100.0)).brightened(light) else {
            panic!("brightened fog stays enabled");
        };
        let original = params(10.0, 100.0);
        assert_eq!(brightened.near_color, original.near_color + light);
        assert_eq!(brightened.far_color, original.far_color + light);
        assert_eq!(
            FogParams {
                near_color: original.near_color,
                far_color: original.far_color,
                ..brightened
            },
            original
        );
    }

    #[test]
    fn brightening_by_no_light_changes_nothing_and_disabled_fog_stays_disabled() {
        let fog = Fog::Enabled(params(10.0, 100.0));
        assert_eq!(fog.brightened(Vec3::ZERO), fog);
        assert_eq!(Fog::Disabled.brightened(Vec3::ONE), Fog::Disabled);
    }

    #[test]
    fn a_nan_fog_range_is_treated_as_disabled() {
        let fields = Fog::Enabled(params(f32::NAN, 100.0)).uniform_fields();
        assert_eq!(fields.params, Vec4::ZERO);
    }
}
