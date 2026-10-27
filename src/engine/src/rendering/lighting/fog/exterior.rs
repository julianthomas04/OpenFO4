use glam::Vec3;

use super::measured::{BlendedFogColors, colour_factors, sunlight_term};
use super::{Fog, FogParams, FogTime, HeightBands};
use crate::rendering::lighting::lerp::{Lerp, lerp_fields};

/// One half of a weather's FNAM fog, in engine units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FogRange {
    pub near: f32,
    pub far: f32,
    pub power: f32,
    pub max: f32,
    pub height: HeightBands,
}

lerp_fields!(FogRange {
    near,
    far,
    power,
    max,
    height
});

/// A weather's fog at one point between its time slots, before the time of day weighs its two
/// halves. The colours carry no factors yet. `None` is a half without fog.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExteriorFog {
    pub colors: BlendedFogColors,
    /// The weather's sunlight colour, linear with gamma 2.2.
    pub sunlight: Vec3,
    pub day: Option<FogRange>,
    pub night: Option<FogRange>,
}

lerp_fields!(ExteriorFog {
    colors,
    sunlight,
    day,
    night
});

impl ExteriorFog {
    /// The fog `time` makes of the weather. A half that has fog cannot be blended with one that
    /// has none, so the distances snap at the midpoint of the night weight (to Day at exactly
    /// one half).
    pub fn resolve(&self, time: FogTime) -> Fog {
        match self.day.lerp(&self.night, time.night_weight) {
            Some(range) => Fog::Enabled(FogParams {
                colors: colour_factors(time.day_colour_weight).apply(self.colors.linear()),
                near: range.near,
                far: range.far,
                power: range.power,
                max: range.max,
                height: range.height,
                sunlight: sunlight_term(self.sunlight, time.day_colour_weight),
            }),
            None => Fog::Disabled,
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "the blends below are exact in f32")]
mod tests {
    use super::super::measured::{DAY_COLOUR_FACTORS, NIGHT_COLOUR_FACTORS};
    use super::super::{FogColors, HeightBand, SunlightTerm};
    use super::*;

    fn range(near: f32) -> FogRange {
        FogRange {
            near,
            far: near * 10.0,
            power: near / 100.0,
            max: 0.5,
            height: HeightBands {
                near: HeightBand::new(near, 10.0),
                far: HeightBand::new(near, 20.0),
                high_density_scale: 0.5,
            },
        }
    }

    fn fog() -> ExteriorFog {
        ExteriorFog {
            colors: BlendedFogColors::from_stored(FogColors::uniform(Vec3::ONE)),
            sunlight: Vec3::splat(0.5),
            day: Some(range(100.0)),
            night: Some(range(200.0)),
        }
    }

    fn enabled(fog: Fog) -> FogParams {
        match fog {
            Fog::Enabled(params) => params,
            Fog::Disabled => panic!("the fog should be enabled"),
        }
    }

    #[test]
    fn full_day_takes_the_day_distances_the_day_factors_and_the_sunlight() {
        let resolved = enabled(fog().resolve(FogTime::DAY));
        assert_eq!(resolved.near, 100.0);
        assert_eq!(resolved.far, 1000.0);
        assert_eq!(
            resolved.colors,
            DAY_COLOUR_FACTORS.apply(fog().colors.linear())
        );
        assert_eq!(resolved.sunlight, sunlight_term(Vec3::splat(0.5), 1.0));
    }

    #[test]
    fn full_night_takes_the_night_distances_the_night_factors_and_no_sunlight() {
        let resolved = enabled(fog().resolve(FogTime {
            night_weight: 1.0,
            day_colour_weight: 0.0,
        }));
        assert_eq!(resolved.near, 200.0);
        assert_eq!(
            resolved.colors,
            NIGHT_COLOUR_FACTORS.apply(fog().colors.linear())
        );
        assert_eq!(resolved.sunlight, SunlightTerm::NONE);
    }

    #[test]
    fn the_distances_and_the_colours_follow_their_own_weights() {
        let resolved = enabled(fog().resolve(FogTime {
            night_weight: 0.5,
            day_colour_weight: 0.0,
        }));
        assert_eq!(resolved.near, 150.0);
        assert_eq!(resolved.far, 1500.0);
        assert_eq!(resolved.power, 1.5);
        assert_eq!(resolved.height.near.mid, 150.0);
        assert_eq!(
            resolved.colors,
            NIGHT_COLOUR_FACTORS.apply(fog().colors.linear())
        );
    }

    #[test]
    fn a_half_without_fog_snaps_at_the_midpoint_of_the_night_weight() {
        let foggy_nights = ExteriorFog { day: None, ..fog() };
        let at = |night_weight| {
            foggy_nights.resolve(FogTime {
                night_weight,
                day_colour_weight: 1.0,
            })
        };
        assert_eq!(at(0.5), Fog::Disabled);
        assert!(matches!(at(0.6), Fog::Enabled(_)));
    }

    #[test]
    fn a_weather_without_fog_in_either_half_is_disabled_all_day() {
        let clear = ExteriorFog {
            day: None,
            night: None,
            ..fog()
        };
        assert_eq!(clear.resolve(FogTime::DAY), Fog::Disabled);
    }
}
